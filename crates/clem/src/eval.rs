//! Binding an expression to a data model and evaluating it on records.

use crate::functions::{self, Args, Function, Returns};
use crate::parser::{BinaryOp, Expr};
use crate::types::{Ty, unify};
use crate::{ClemError, Datum};
use dms_types::{DataModel, MissingSpec, Role, Value, ValueSpec};
use std::cmp::Ordering;

/// What names mean while binding.
#[derive(Debug, Clone, Copy)]
pub struct Bindings<'a> {
    pub model: &'a DataModel,
    /// The field `@FIELD` stands for, in multi-field nodes (Filler, multiple Derive).
    pub current_field: Option<usize>,
}

/// An expression bound to a data model, ready to evaluate on its records.
#[derive(Debug, Clone)]
pub struct Compiled {
    root: Node,
}

#[derive(Debug, Clone)]
enum Node {
    Const(Datum),
    /// `undef`.
    Undef,
    Field(usize),
    /// A flag field read as a condition: true for its true value, false for its false value.
    Flag {
        position: usize,
        true_value: Value,
        false_value: Value,
    },
    Negate(Box<Node>),
    Complement(Box<Node>),
    Chain(Box<Node>, Vec<(BinaryOp, Node)>),
    If(Vec<(Node, Node)>, Box<Node>),
    Call(Function, Vec<Node>),
    List(Vec<Node>),
    /// `@NULL(expr)`
    IsNull(Box<Node>),
    /// `@BLANK(field)`: the field's blank definition, captured when binding.
    IsBlank(usize, MissingSpec),
    /// `@INDEX`: 1-based record number.
    Index,
}

pub fn compile(expr: &Expr, bindings: &Bindings<'_>) -> Result<Compiled, ClemError> {
    Ok(Compiled {
        root: bind(expr, bindings)?,
    })
}

/// Compiles an expression used as a condition (Select, Flag Derive): a flag field then stands
/// for its truth value.
pub fn compile_condition(expr: &Expr, bindings: &Bindings<'_>) -> Result<Compiled, ClemError> {
    Ok(Compiled {
        root: condition(expr, bindings)?,
    })
}

/// Binds an expression in a condition position. Modeler accepts only conditions and fields
/// whose measurement is flag there (oracle: `if 1 then` and a continuous field are rejected).
fn condition(expr: &Expr, b: &Bindings<'_>) -> Result<Node, ClemError> {
    let name = match expr {
        Expr::Field(name) | Expr::Quoted(name) => name,
        _ => return bind(expr, b),
    };
    let flag =
        b.model
            .position(name)
            .and_then(|position| match &b.model.fields().get(position)?.values {
                ValueSpec::Flag {
                    true_value,
                    false_value,
                } => Some(Node::Flag {
                    position,
                    true_value: true_value.clone(),
                    false_value: false_value.clone(),
                }),
                _ => None,
            });
    match flag {
        Some(node) => Ok(node),
        None => bind(expr, b),
    }
}

fn bind(expr: &Expr, b: &Bindings<'_>) -> Result<Node, ClemError> {
    Ok(match expr {
        Expr::Literal(value) => Node::Const(Datum::Value(value.clone())),
        Expr::Boolean(truth) => Node::Const(Datum::Bool(*truth)),
        Expr::Char(c) => Node::Const(Datum::Value(Value::string(c.to_string()))),
        Expr::Undef => Node::Undef,
        Expr::Field(name) => match b.model.position(name) {
            Some(position) => Node::Field(position),
            // `pi` is written without parentheses; a field of that name wins.
            None if name == "pi" => Node::Const(Datum::Value(Value::Real(std::f64::consts::PI))),
            None => return Err(ClemError::UnknownField(name.clone())),
        },
        Expr::Quoted(name) => match b.model.position(name) {
            Some(position) => Node::Field(position),
            None => Node::Const(Datum::Value(Value::string(name.as_str()))),
        },
        Expr::Special(name) => special(name, b)?,
        Expr::Negate(inner) => Node::Negate(Box::new(bind(inner, b)?)),
        Expr::Complement(inner) => Node::Complement(Box::new(bind(inner, b)?)),
        Expr::Chain(first, rest) => {
            // A level holds either `and`/`or` or no logical operator at all.
            let logical = rest
                .iter()
                .any(|(op, _)| matches!(op, BinaryOp::And | BinaryOp::Or));
            let operand = |expr: &Expr| {
                if logical {
                    condition(expr, b)
                } else {
                    bind(expr, b)
                }
            };
            Node::Chain(
                Box::new(operand(first)?),
                rest.iter()
                    .map(|(op, expr)| Ok((*op, operand(expr)?)))
                    .collect::<Result<_, ClemError>>()?,
            )
        }
        Expr::If {
            branches,
            otherwise,
        } => {
            let mut branches = branches
                .iter()
                .map(|(test, value)| Ok((condition(test, b)?, bind(value, b)?)))
                .collect::<Result<Vec<_>, ClemError>>()?;
            let mut otherwise = bind(otherwise, b)?;
            // Oracle rule 38: the branches share one type; integers widen to reals.
            let mut result = ty(&otherwise, b);
            for (_, value) in &branches {
                result = unify(result, ty(value, b)).ok_or_else(|| {
                    ClemError::Type("the branches of `if` have incompatible types".into())
                })?;
            }
            if result == Ty::Real {
                for (_, value) in &mut branches {
                    widen(value, b);
                }
                widen(&mut otherwise, b);
            }
            Node::If(branches, Box::new(otherwise))
        }
        Expr::List(items) => Node::List(
            items
                .iter()
                .map(|item| bind(item, b))
                .collect::<Result<_, _>>()?,
        ),
        Expr::Call(name, args) => call(name, args, b)?,
    })
}

fn special(name: &str, b: &Bindings<'_>) -> Result<Node, ClemError> {
    let partition = || {
        b.model
            .fields()
            .iter()
            .position(|field| field.role == Role::Partition)
            .ok_or_else(|| ClemError::Unsupported(format!("{name}: no partition field")))
    };
    // Partition values in Partition-node order: training, testing, validation.
    let partition_value = |ordinal: usize| -> Result<Node, ClemError> {
        let field = b
            .model
            .fields()
            .get(partition()?)
            .ok_or_else(|| ClemError::Unsupported(name.to_string()))?;
        match &field.values {
            ValueSpec::Categories(values) => Ok(Node::Const(Datum::Value(
                values.get(ordinal).cloned().unwrap_or(Value::Null),
            ))),
            _ => Err(ClemError::Unsupported(format!(
                "{name}: partition values are not known"
            ))),
        }
    };
    match name {
        "@FIELD" => b
            .current_field
            .map(Node::Field)
            .ok_or_else(|| ClemError::Unsupported("@FIELD outside a multi-field node".into())),
        "@PARTITION_FIELD" => partition().map(Node::Field),
        "@TRAINING_PARTITION" => partition_value(0),
        "@TESTING_PARTITION" => partition_value(1),
        "@VALIDATION_PARTITION" => partition_value(2),
        "@INDEX" => Ok(Node::Index),
        other => Err(ClemError::Unsupported(other.to_string())),
    }
}

fn call(name: &str, args: &[Expr], b: &Bindings<'_>) -> Result<Node, ClemError> {
    let arity = |accepted: &[usize]| {
        if accepted.contains(&args.len()) {
            Ok(())
        } else {
            let expected: Vec<String> = accepted.iter().map(usize::to_string).collect();
            Err(ClemError::Arity {
                function: name.to_string(),
                expected: expected.join(" or "),
                found: args.len(),
            })
        }
    };
    match name {
        "@NULL" => {
            arity(&[1])?;
            let inner = args
                .first()
                .ok_or_else(|| ClemError::Unsupported(name.into()))?;
            Ok(Node::IsNull(Box::new(bind(inner, b)?)))
        }
        "@BLANK" => {
            arity(&[1])?;
            let field = match args.first().map(|arg| bind(arg, b)) {
                Some(Ok(Node::Field(position))) => position,
                _ => {
                    return Err(ClemError::Type(
                        "@BLANK needs a field as its argument".into(),
                    ));
                }
            };
            let missing = b
                .model
                .fields()
                .get(field)
                .map(|f| f.missing.clone())
                .unwrap_or_default();
            Ok(Node::IsBlank(field, missing))
        }
        _ => {
            let function = functions::lookup(name)
                .ok_or_else(|| ClemError::UnknownFunction(name.to_string()))?;
            arity(function.arities)?;
            check_args(&function, args)?;
            let args = args
                .iter()
                .map(|arg| {
                    if function.args == Args::Condition {
                        condition(arg, b)
                    } else {
                        bind(arg, b)
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            // A type test is answered by the argument's static type when that is known.
            if let (Some(test), [arg]) = (function.type_test, args.as_slice()) {
                let arg_ty = ty(arg, b);
                if arg_ty.is_known() {
                    return Ok(Node::Const(Datum::Bool(test(arg_ty))));
                }
            }
            Ok(Node::Call(function, args))
        }
    }
}

/// The checks Modeler makes on the written arguments when it compiles a call.
fn check_args(function: &Function, args: &[Expr]) -> Result<(), ClemError> {
    let literal = |expr: &Expr| {
        matches!(
            expr,
            Expr::Literal(_) | Expr::Boolean(_) | Expr::Char(_) | Expr::Undef
        )
    };
    let list = |n: usize| match args.get(n) {
        Some(Expr::List(items)) => Ok(items),
        _ => Err(ClemError::Type(format!(
            "{}: argument {} must be a list",
            function.name,
            n + 1
        ))),
    };
    match function.args {
        Args::Any | Args::Condition => Ok(()),
        Args::CharFirst => match args.first() {
            Some(Expr::Literal(Value::String(_))) | Some(Expr::Quoted(_)) => {
                Err(ClemError::Type(format!(
                    "{} takes a character such as `c`, not a string",
                    function.name
                )))
            }
            _ => Ok(()),
        },
        Args::Fields(n) => {
            if list(n)?.iter().any(literal) {
                Err(ClemError::Type(format!(
                    "{} takes a list of fields",
                    function.name
                )))
            } else {
                Ok(())
            }
        }
        Args::Uniform(n) => {
            let items = list(n)?;
            if items.iter().any(literal) && !items.iter().all(literal) {
                Err(ClemError::Type(format!(
                    "{}: the list mixes values and fields",
                    function.name
                )))
            } else {
                Ok(())
            }
        }
    }
}

impl Compiled {
    /// Evaluates on one record; `index` is the 1-based record number (`@INDEX`).
    pub fn eval(&self, row: &[Value], index: u64) -> Result<Datum, ClemError> {
        eval(&self.root, row, index)
    }
}

/// Recursion follows the bound tree, whose depth the parser bounds.
fn eval(node: &Node, row: &[Value], index: u64) -> Result<Datum, ClemError> {
    match node {
        Node::Const(datum) => Ok(datum.clone()),
        Node::Undef => Ok(Datum::NULL),
        Node::Field(position) => Ok(Datum::Value(
            row.get(*position).cloned().unwrap_or(Value::Null),
        )),
        // GAP: a value that is neither flag value; null.
        Node::Flag {
            position,
            true_value,
            false_value,
        } => Ok(match row.get(*position) {
            Some(value) if value.total_cmp(true_value).is_eq() => Datum::Bool(true),
            Some(value) if value.total_cmp(false_value).is_eq() => Datum::Bool(false),
            _ => Datum::NULL,
        }),
        Node::Index => Ok(Datum::Value(Value::Integer(
            i64::try_from(index).unwrap_or(i64::MAX),
        ))),
        Node::IsNull(inner) => Ok(Datum::Bool(eval(inner, row, index)?.is_null())),
        Node::IsBlank(position, missing) => Ok(Datum::Bool(
            row.get(*position)
                .is_some_and(|value| missing.is_blank(value)),
        )),
        Node::Negate(inner) => functions::negate(eval(inner, row, index)?),
        Node::Complement(inner) => functions::complement(eval(inner, row, index)?),
        Node::If(branches, otherwise) => {
            for (condition, value) in branches {
                // A null condition falls through to the else branch.
                if eval(condition, row, index)?.truth()? == Some(true) {
                    return eval(value, row, index);
                }
            }
            eval(otherwise, row, index)
        }
        Node::List(items) => Ok(Datum::Value(Value::List(
            items
                .iter()
                .map(|item| eval(item, row, index).and_then(Datum::into_value))
                .collect::<Result<Vec<_>, _>>()?
                .into(),
        ))),
        Node::Call(function, args) => {
            let values = args
                .iter()
                .map(|arg| eval(arg, row, index))
                .collect::<Result<Vec<_>, _>>()?;
            (function.apply)(&values)
        }
        Node::Chain(first, rest) => {
            let mut acc = eval(first, row, index)?;
            let mut left_undef = matches!(**first, Node::Undef);
            for (op, operand) in rest {
                let undef = left_undef || matches!(operand, Node::Undef);
                acc = match op {
                    BinaryOp::And => and(acc, || eval(operand, row, index))?,
                    BinaryOp::Or => or(acc, || eval(operand, row, index))?,
                    // Oracle: comparing with the keyword `undef` tests for null and is never
                    // null itself, while `m = m` on a null `m` is null.
                    BinaryOp::Equal | BinaryOp::NotEqual if undef => {
                        let right = eval(operand, row, index)?;
                        let equal = acc.is_null() == right.is_null();
                        Datum::Bool(equal == (*op == BinaryOp::Equal))
                    }
                    _ => binary(*op, acc, eval(operand, row, index)?)?,
                };
                left_undef = false;
            }
            Ok(acc)
        }
    }
}

fn logical(truth: Option<bool>) -> Datum {
    truth.map_or(Datum::NULL, Datum::Bool)
}

/// Three-valued, short-circuit `and`: false and X = false; null and false = false.
fn and(left: Datum, right: impl FnOnce() -> Result<Datum, ClemError>) -> Result<Datum, ClemError> {
    Ok(match left.truth()? {
        Some(false) => Datum::Bool(false),
        Some(true) => logical(right()?.truth()?),
        None => match right()?.truth()? {
            Some(false) => Datum::Bool(false),
            _ => Datum::NULL,
        },
    })
}

/// Three-valued, short-circuit `or`: true or X = true; null or true = true.
fn or(left: Datum, right: impl FnOnce() -> Result<Datum, ClemError>) -> Result<Datum, ClemError> {
    Ok(match left.truth()? {
        Some(true) => Datum::Bool(true),
        Some(false) => logical(right()?.truth()?),
        None => match right()?.truth()? {
            Some(true) => Datum::Bool(true),
            _ => Datum::NULL,
        },
    })
}

fn binary(op: BinaryOp, left: Datum, right: Datum) -> Result<Datum, ClemError> {
    match op {
        BinaryOp::Equal | BinaryOp::NotEqual => {
            // A null operand gives null (the keyword `undef` is handled by the caller).
            if left.is_null() || right.is_null() {
                return Ok(Datum::NULL);
            }
            let equal = compare(&left, &right)? == Ordering::Equal;
            Ok(Datum::Bool(equal == (op == BinaryOp::Equal)))
        }
        BinaryOp::Greater | BinaryOp::GreaterEqual | BinaryOp::Less | BinaryOp::LessEqual => {
            if left.is_null() || right.is_null() {
                return Ok(Datum::NULL);
            }
            let ordering = compare(&left, &right)?;
            Ok(Datum::Bool(match op {
                BinaryOp::Greater => ordering == Ordering::Greater,
                BinaryOp::GreaterEqual => ordering != Ordering::Less,
                BinaryOp::Less => ordering == Ordering::Less,
                _ => ordering != Ordering::Greater,
            }))
        }
        BinaryOp::Concat => {
            if left.is_null() || right.is_null() {
                return Ok(Datum::NULL);
            }
            Ok(Datum::Value(Value::string(format!(
                "{}{}",
                left.to_text()?,
                right.to_text()?
            ))))
        }
        BinaryOp::BitsClear | BinaryOp::BitsSet => functions::bits_test(op, left, right),
        BinaryOp::Matches => functions::matches(left, right),
        _ => functions::arithmetic(op, left, right),
    }
}

/// Ordering between comparable values: numbers with numbers, strings with strings, dates,
/// times and timestamps with their own kind. Truth values do not compare (oracle: `Boolean =
/// Boolean` is a type error, so comparisons do not chain).
fn compare(left: &Datum, right: &Datum) -> Result<Ordering, ClemError> {
    match (left, right) {
        (Datum::Value(a), Datum::Value(b)) => {
            let comparable = matches!(
                (a, b),
                (
                    Value::Integer(_) | Value::Real(_),
                    Value::Integer(_) | Value::Real(_)
                ) | (Value::String(_), Value::String(_))
                    | (Value::Date(_), Value::Date(_))
                    | (Value::Time(_), Value::Time(_))
                    | (Value::Timestamp(_), Value::Timestamp(_))
            );
            if comparable {
                Ok(a.total_cmp(b))
            } else {
                Err(ClemError::Type(format!("cannot compare {a:?} with {b:?}")))
            }
        }
        _ => Err(ClemError::Type(format!(
            "cannot compare {left:?} with {right:?}"
        ))),
    }
}

/// Recursion follows the bound tree, whose depth the parser bounds.
fn ty(node: &Node, b: &Bindings<'_>) -> Ty {
    match node {
        Node::Const(Datum::Value(value)) => Ty::of_value(value),
        Node::Const(Datum::Bool(_)) | Node::Flag { .. } | Node::IsNull(_) | Node::IsBlank(..) => {
            Ty::Bool
        }
        Node::Undef => Ty::Null,
        Node::Field(position) => b
            .model
            .fields()
            .get(*position)
            .map_or(Ty::Unknown, |field| Ty::of_storage(field.storage)),
        Node::Negate(inner) => ty(inner, b),
        Node::Complement(_) | Node::Index => Ty::Integer,
        Node::List(_) => Ty::Unknown,
        Node::If(branches, otherwise) => branches
            .iter()
            .try_fold(ty(otherwise, b), |acc, (_, value)| unify(acc, ty(value, b)))
            .unwrap_or(Ty::Unknown),
        Node::Call(function, args) => {
            let common = |types: Vec<Ty>| {
                types
                    .into_iter()
                    .try_fold(Ty::Null, unify)
                    .unwrap_or(Ty::Unknown)
            };
            match function.returns {
                Returns::Fixed(result) => result,
                Returns::Arg(n) => args.get(n).map_or(Ty::Unknown, |arg| ty(arg, b)),
                Returns::Common => common(args.iter().map(|arg| ty(arg, b)).collect()),
                Returns::Items(n) => match args.get(n) {
                    Some(Node::List(items)) => {
                        common(items.iter().map(|item| ty(item, b)).collect())
                    }
                    _ => Ty::Unknown,
                },
            }
        }
        Node::Chain(first, rest) => rest.iter().fold(ty(first, b), |left, (op, operand)| {
            let right = ty(operand, b);
            let numeric = |t: Ty| matches!(t, Ty::Integer | Ty::Real);
            match op {
                BinaryOp::And
                | BinaryOp::Or
                | BinaryOp::Equal
                | BinaryOp::NotEqual
                | BinaryOp::Greater
                | BinaryOp::GreaterEqual
                | BinaryOp::Less
                | BinaryOp::LessEqual
                | BinaryOp::BitsClear
                | BinaryOp::BitsSet
                | BinaryOp::Matches => Ty::Bool,
                BinaryOp::Concat => Ty::String,
                BinaryOp::Divide => Ty::Real,
                BinaryOp::Div
                | BinaryOp::BitAnd
                | BinaryOp::BitAndNot
                | BinaryOp::BitOr
                | BinaryOp::BitXor
                | BinaryOp::ShiftLeft
                | BinaryOp::ShiftRight => Ty::Integer,
                BinaryOp::Add
                | BinaryOp::Subtract
                | BinaryOp::Multiply
                | BinaryOp::Power
                | BinaryOp::Rem
                | BinaryOp::Mod => match (left, right) {
                    (Ty::Integer, Ty::Integer) => Ty::Integer,
                    (l, r) if numeric(l) && numeric(r) => Ty::Real,
                    _ => Ty::Unknown,
                },
            }
        }),
    }
}

/// Converts an integer-typed branch to real.
fn widen(node: &mut Node, b: &Bindings<'_>) {
    if ty(node, b) == Ty::Integer
        && let Some(to_real) = functions::lookup("to_real")
    {
        let inner = std::mem::replace(node, Node::Undef);
        *node = Node::Call(to_real, vec![inner]);
    }
}
