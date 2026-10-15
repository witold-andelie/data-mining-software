//! Binding an expression to a data model and evaluating it on records.

use crate::functions::{self, Function};
use crate::parser::{BinaryOp, Expr};
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
    Field(usize),
    Negate(Box<Node>),
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

fn bind(expr: &Expr, b: &Bindings<'_>) -> Result<Node, ClemError> {
    Ok(match expr {
        Expr::Literal(value) => Node::Const(Datum::Value(value.clone())),
        Expr::Field(name) => Node::Field(
            b.model
                .position(name)
                .ok_or_else(|| ClemError::UnknownField(name.clone()))?,
        ),
        Expr::Quoted(name) => match b.model.position(name) {
            Some(position) => Node::Field(position),
            None => Node::Const(Datum::Value(Value::string(name.as_str()))),
        },
        Expr::Special(name) => special(name, b)?,
        Expr::Negate(inner) => Node::Negate(Box::new(bind(inner, b)?)),
        Expr::Chain(first, rest) => Node::Chain(
            Box::new(bind(first, b)?),
            rest.iter()
                .map(|(op, operand)| Ok((*op, bind(operand, b)?)))
                .collect::<Result<_, ClemError>>()?,
        ),
        Expr::If {
            branches,
            otherwise,
        } => Node::If(
            branches
                .iter()
                .map(|(condition, value)| Ok((bind(condition, b)?, bind(value, b)?)))
                .collect::<Result<_, ClemError>>()?,
            Box::new(bind(otherwise, b)?),
        ),
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
    let arity = |expected: usize| {
        if args.len() == expected {
            Ok(())
        } else {
            Err(ClemError::Arity {
                function: name.to_string(),
                expected,
                found: args.len(),
            })
        }
    };
    match name {
        "@NULL" => {
            arity(1)?;
            let inner = args
                .first()
                .ok_or_else(|| ClemError::Unsupported(name.into()))?;
            Ok(Node::IsNull(Box::new(bind(inner, b)?)))
        }
        "@BLANK" => {
            arity(1)?;
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
            arity(function.arity)?;
            let args = args
                .iter()
                .map(|arg| bind(arg, b))
                .collect::<Result<_, _>>()?;
            Ok(Node::Call(function, args))
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
        Node::Field(position) => Ok(Datum::Value(
            row.get(*position).cloned().unwrap_or(Value::Null),
        )),
        Node::Index => Ok(Datum::Value(Value::Integer(
            i64::try_from(index).unwrap_or(i64::MAX),
        ))),
        Node::IsNull(inner) => Ok(Datum::Bool(eval(inner, row, index)?.is_null())),
        Node::IsBlank(position, missing) => Ok(Datum::Bool(
            row.get(*position)
                .is_some_and(|value| missing.is_blank(value)),
        )),
        Node::Negate(inner) => functions::negate(eval(inner, row, index)?),
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
            for (op, operand) in rest {
                acc = match op {
                    BinaryOp::And => and(acc, || eval(operand, row, index))?,
                    BinaryOp::Or => or(acc, || eval(operand, row, index))?,
                    _ => binary(*op, acc, eval(operand, row, index)?)?,
                };
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
            let equal = match (left.is_null(), right.is_null()) {
                // Two nulls compare equal; null against a value is null.
                (true, true) => true,
                (true, false) | (false, true) => return Ok(Datum::NULL),
                (false, false) => compare(&left, &right)? == Ordering::Equal,
            };
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
                left.to_text(),
                right.to_text()
            ))))
        }
        _ => functions::arithmetic(op, left, right),
    }
}

/// Ordering between comparable values: numbers with numbers, strings with strings, dates,
/// times and timestamps with their own kind, booleans with booleans.
fn compare(left: &Datum, right: &Datum) -> Result<Ordering, ClemError> {
    match (left, right) {
        (Datum::Bool(a), Datum::Bool(b)) => Ok(a.cmp(b)),
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
