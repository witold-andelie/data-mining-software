//! Binding an expression to a data model and evaluating it on records.

use crate::functions::{self, Args, Function, Returns};
use crate::parser::{BinaryOp, Expr};
use crate::sequence::{self, Statistic};
use crate::types::{Ty, unify};
use crate::{ClemError, Datum, GlobalStatistic, GlobalValues};
use dms_types::{DataModel, MissingSpec, Role, Value, ValueSpec};
use std::cell::RefCell;
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
    /// Sequence functions that need all records first, in dependency order: one may use the
    /// columns of those before it.
    accumulators: Vec<Accumulator>,
}

/// A sequence function evaluated over all records before the expression itself.
#[derive(Debug, Clone)]
struct Accumulator {
    kind: Sequence,
    value: Node,
    window: Option<Node>,
}

#[derive(Debug, Clone)]
enum Sequence {
    Statistic(Statistic),
    /// `@SINCE` and `@SINCE0`; `uses_this` when the condition looks at the current record
    /// through `@THIS`, which needs the condition per pair of records.
    Since {
        include_current: bool,
        uses_this: bool,
    },
    LastNonBlank(MissingSpec),
}

/// Bound to the data model while binding, with the sequence functions found so far.
struct Binder<'a> {
    model: &'a DataModel,
    globals: Option<&'a GlobalValues>,
    current_field: Option<usize>,
    accumulators: RefCell<Vec<Accumulator>>,
}

impl<'a> Binder<'a> {
    fn new(bindings: &Bindings<'a>) -> Binder<'a> {
        Binder {
            model: bindings.model,
            globals: None,
            current_field: bindings.current_field,
            accumulators: RefCell::new(Vec::new()),
        }
    }

    fn accumulate(&self, accumulator: Accumulator) -> Node {
        let mut accumulators = self.accumulators.borrow_mut();
        accumulators.push(accumulator);
        Node::Column(accumulators.len() - 1)
    }

    fn finish(self, root: Node) -> Compiled {
        Compiled {
            root,
            accumulators: self.accumulators.into_inner(),
        }
    }
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
    /// `@OFFSET(value, by)`: `value` on the record `by` records earlier (later when negative);
    /// null outside the records.
    Offset(Box<Node>, Box<Node>),
    /// `@THIS(value)`: `value` on the record a `@SINCE` condition is evaluated for.
    This(Box<Node>),
    /// The result of accumulator `n` on the current record.
    Column(usize),
}

pub fn compile(expr: &Expr, bindings: &Bindings<'_>) -> Result<Compiled, ClemError> {
    let binder = Binder::new(bindings);
    let root = bind(expr, &binder)?;
    Ok(binder.finish(root))
}

/// Binds globals from the current execution's snapshot. Global arguments name fields,
/// rather than reading their per-record values. Resolution happens even on zero records.
/// `as_condition` applies the same flag-field rules as `compile_condition`.
pub fn compile_with_globals(
    expr: &Expr,
    bindings: &Bindings<'_>,
    globals: &GlobalValues,
    as_condition: bool,
) -> Result<Compiled, ClemError> {
    let mut binder = Binder::new(bindings);
    binder.globals = Some(globals);
    let root = if as_condition {
        condition(expr, &binder)?
    } else {
        bind(expr, &binder)?
    };
    Ok(binder.finish(root))
}

/// Compiles an expression used as a condition (Select, Flag Derive): a flag field then stands
/// for its truth value.
pub fn compile_condition(expr: &Expr, bindings: &Bindings<'_>) -> Result<Compiled, ClemError> {
    let binder = Binder::new(bindings);
    let root = condition(expr, &binder)?;
    Ok(binder.finish(root))
}

/// Binds an expression in a condition position. Modeler accepts only conditions and fields
/// whose measurement is flag there (oracle: `if 1 then` and a continuous field are rejected).
fn condition(expr: &Expr, b: &Binder<'_>) -> Result<Node, ClemError> {
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

fn bind(expr: &Expr, b: &Binder<'_>) -> Result<Node, ClemError> {
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

fn special(name: &str, b: &Binder<'_>) -> Result<Node, ClemError> {
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

fn call(name: &str, args: &[Expr], b: &Binder<'_>) -> Result<Node, ClemError> {
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
        name if GlobalStatistic::from_function(name).is_some() => {
            arity(&[1])?;
            let field = match args.first() {
                Some(Expr::Field(field) | Expr::Quoted(field)) => field.as_str(),
                Some(Expr::Special(special)) if special == "@FIELD" => b
                    .current_field
                    .and_then(|position| b.model.fields().get(position))
                    .map(|field| field.name.as_str())
                    .ok_or_else(|| ClemError::Type(format!("{name} needs a field name")))?,
                _ => return Err(ClemError::Type(format!("{name} needs a field name"))),
            };
            let statistic = GlobalStatistic::from_function(name)
                .ok_or_else(|| ClemError::UnknownFunction(name.into()))?;
            let value = b
                .globals
                .ok_or_else(|| ClemError::UndefinedGlobal {
                    function: name.into(),
                    field: field.into(),
                })?
                .get(field, statistic)?;
            Ok(Node::Const(Datum::Value(value.clone())))
        }
        "@NULL" => {
            arity(&[1])?;
            let inner = args
                .first()
                .ok_or_else(|| ClemError::Unsupported(name.into()))?;
            Ok(Node::IsNull(Box::new(bind(inner, b)?)))
        }
        "@OFFSET" => {
            arity(&[2])?;
            let (value, by) = two(args)?;
            Ok(Node::Offset(
                Box::new(bind(value, b)?),
                Box::new(bind(by, b)?),
            ))
        }
        // `@DIFF1(f)` is `f - @OFFSET(f, 1)`, `@DIFF1(f, g)` is `@DIFF1(f) / @DIFF1(g)` and
        // `@DIFF2(f)` is `@DIFF1(f) - @OFFSET(@DIFF1(f), 1)`; the oracle shows integer wrapping
        // and null on division by zero, as for the operators.
        "@DIFF1" => {
            arity(&[1, 2])?;
            let difference = |expr: &Expr| difference(bind(expr, b)?);
            match args {
                [value] => difference(value),
                [value, by] => Ok(Node::Chain(
                    Box::new(difference(value)?),
                    vec![(BinaryOp::Divide, difference(by)?)],
                )),
                _ => Err(ClemError::Unsupported(name.into())),
            }
        }
        "@DIFF2" => {
            arity(&[1])?;
            let value = args
                .first()
                .ok_or_else(|| ClemError::Unsupported(name.into()))?;
            difference(difference(bind(value, b)?)?)
        }
        "@THIS" => {
            arity(&[1])?;
            let value = args
                .first()
                .ok_or_else(|| ClemError::Unsupported(name.into()))?;
            Ok(Node::This(Box::new(bind(value, b)?)))
        }
        "@MEAN" | "@AVE" | "@SUM" | "@MIN" | "@MAX" | "@SDEV" => {
            arity(&[1, 2])?;
            let statistic = match name {
                "@SUM" => Statistic::Sum,
                "@MIN" => Statistic::Min,
                "@MAX" => Statistic::Max,
                "@SDEV" => Statistic::Sdev,
                _ => Statistic::Mean,
            };
            let value = args
                .first()
                .ok_or_else(|| ClemError::Unsupported(name.into()))?;
            let accumulator = Accumulator {
                kind: Sequence::Statistic(statistic),
                value: bind(value, b)?,
                window: args.get(1).map(|window| bind(window, b)).transpose()?,
            };
            Ok(b.accumulate(accumulator))
        }
        "@SINCE" | "@SINCE0" => {
            arity(&[1])?;
            let test = args
                .first()
                .ok_or_else(|| ClemError::Unsupported(name.into()))?;
            let value = condition(test, b)?;
            let accumulator = Accumulator {
                kind: Sequence::Since {
                    include_current: name == "@SINCE0",
                    uses_this: uses_this(&value),
                },
                value,
                window: None,
            };
            Ok(b.accumulate(accumulator))
        }
        "@LAST_NON_BLANK" => {
            arity(&[1])?;
            let field = match args.first().map(|arg| bind(arg, b)) {
                Some(Ok(Node::Field(position))) => position,
                _ => {
                    return Err(ClemError::Type(
                        "@LAST_NON_BLANK needs a field as its argument".into(),
                    ));
                }
            };
            let missing = b
                .model
                .fields()
                .get(field)
                .map(|f| f.missing.clone())
                .unwrap_or_default();
            Ok(b.accumulate(Accumulator {
                kind: Sequence::LastNonBlank(missing),
                value: Node::Field(field),
                window: None,
            }))
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
        // `FIELD(N)`: the Nth character of a string field, as `subscrs(N, FIELD)`.
        _ if args.len() == 1 && b.model.position(name).is_some() => {
            let (Some(subscrs), Some(position), Some(n)) = (
                functions::lookup("subscrs"),
                b.model.position(name),
                args.first(),
            ) else {
                return Err(ClemError::UnknownFunction(name.to_string()));
            };
            Ok(Node::Call(
                subscrs,
                vec![bind(n, b)?, Node::Field(position)],
            ))
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

fn two(args: &[Expr]) -> Result<(&Expr, &Expr), ClemError> {
    match args {
        [first, second] => Ok((first, second)),
        _ => Err(ClemError::Type("two arguments expected".into())),
    }
}

/// `value - @OFFSET(value, 1)`.
fn difference(value: Node) -> Result<Node, ClemError> {
    let previous = Node::Offset(
        Box::new(value.clone()),
        Box::new(Node::Const(Datum::Value(Value::Integer(1)))),
    );
    Ok(Node::Chain(
        Box::new(value),
        vec![(BinaryOp::Subtract, previous)],
    ))
}

/// Whether `@THIS` occurs in a bound expression.
fn uses_this(node: &Node) -> bool {
    match node {
        Node::This(_) => true,
        Node::Const(_)
        | Node::Undef
        | Node::Field(_)
        | Node::Flag { .. }
        | Node::IsBlank(..)
        | Node::Index
        | Node::Column(_) => false,
        Node::Negate(inner) | Node::Complement(inner) | Node::IsNull(inner) => uses_this(inner),
        Node::Offset(value, by) => uses_this(value) || uses_this(by),
        Node::Chain(first, rest) => uses_this(first) || rest.iter().any(|(_, n)| uses_this(n)),
        Node::If(branches, otherwise) => {
            uses_this(otherwise) || branches.iter().any(|(c, v)| uses_this(c) || uses_this(v))
        }
        Node::Call(_, items) | Node::List(items) => items.iter().any(uses_this),
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

/// Above this many condition evaluations, `@SINCE` with `@THIS` gives up instead of running for
/// hours (each record may look back over all earlier records).
const MAX_SINCE_STEPS: u64 = 50_000_000;

impl Compiled {
    /// Evaluates on every record, in order; sequence functions (`@OFFSET`, `@MEAN`, `@SINCE`,
    /// ...) see the other records. `@INDEX` of the first record is 1.
    pub fn eval_records<R: AsRef<[Value]>>(&self, rows: &[R]) -> Result<Vec<Datum>, ClemError> {
        self.evaluate(rows, 1)
    }

    /// Evaluates on one record as if it stood alone: sequence functions see no other record.
    /// `index` is its `@INDEX`.
    pub fn eval(&self, row: &[Value], index: u64) -> Result<Datum, ClemError> {
        // The record is known to the caller; report the error itself.
        let mut results = self
            .evaluate(std::slice::from_ref(&row), index)
            .map_err(|error| match error {
                ClemError::Record { error, .. } => *error,
                other => other,
            })?;
        results
            .pop()
            .ok_or_else(|| ClemError::Unsupported("no record to evaluate".into()))
    }

    fn evaluate<R: AsRef<[Value]>>(
        &self,
        rows: &[R],
        first_index: u64,
    ) -> Result<Vec<Datum>, ClemError> {
        let mut columns: Vec<Vec<Datum>> = Vec::with_capacity(self.accumulators.len());
        for accumulator in &self.accumulators {
            let records = Records {
                rows,
                first_index,
                columns: &columns,
            };
            let column = accumulator.compute(&records)?;
            columns.push(column);
        }
        let records = Records {
            rows,
            first_index,
            columns: &columns,
        };
        records.all(&self.root)
    }
}

/// The records an expression runs over, and the accumulator columns computed so far.
struct Records<'a, R> {
    rows: &'a [R],
    first_index: u64,
    columns: &'a [Vec<Datum>],
}

impl<R: AsRef<[Value]>> Records<'_, R> {
    fn value(&self, row: usize, position: usize) -> Option<&Value> {
        self.rows.get(row)?.as_ref().get(position)
    }

    fn column(&self, n: usize, at: At) -> Datum {
        self.columns
            .get(n)
            .and_then(|column| column.get(at.row))
            .cloned()
            .unwrap_or(Datum::NULL)
    }

    /// `node` on every record.
    fn all(&self, node: &Node) -> Result<Vec<Datum>, ClemError> {
        (0..self.rows.len())
            .map(|row| {
                eval(node, self, At { row, this: row }).map_err(|error| ClemError::Record {
                    index: sequence::index(self.first_index, row),
                    error: Box::new(error),
                })
            })
            .collect()
    }
}

/// Where an expression is evaluated: on record `row`, for the record `this` (`@THIS`), which
/// differs only inside a `@SINCE` condition.
#[derive(Debug, Clone, Copy)]
struct At {
    row: usize,
    this: usize,
}

impl Accumulator {
    fn compute<R: AsRef<[Value]>>(
        &self,
        records: &Records<'_, R>,
    ) -> Result<Vec<Datum>, ClemError> {
        match &self.kind {
            Sequence::Statistic(statistic) => {
                let values = records.all(&self.value)?;
                let windows = self
                    .window
                    .as_ref()
                    .map(|window| records.all(window))
                    .transpose()?;
                sequence::statistic(*statistic, &values, windows.as_deref())
            }
            Sequence::LastNonBlank(missing) => Ok(sequence::last_non_blank(
                &records.all(&self.value)?,
                missing,
            )),
            Sequence::Since {
                include_current,
                uses_this: false,
            } => {
                let conditions = records
                    .all(&self.value)?
                    .iter()
                    .map(Datum::truth)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(sequence::since(
                    &conditions,
                    *include_current,
                    records.first_index,
                ))
            }
            Sequence::Since {
                include_current,
                uses_this: true,
            } => {
                let mut steps = 0u64;
                let mut out = Vec::with_capacity(records.rows.len());
                for this in 0..records.rows.len() {
                    let newest = if *include_current { this + 1 } else { this };
                    let mut back = None;
                    for row in (0..newest).rev() {
                        steps += 1;
                        if steps > MAX_SINCE_STEPS {
                            return Err(ClemError::Unsupported(
                                "@SINCE with @THIS over this many records".into(),
                            ));
                        }
                        if eval(&self.value, records, At { row, this })?.truth()? == Some(true) {
                            back = Some(this - row);
                            break;
                        }
                    }
                    out.push(Datum::Value(Value::Integer(match back {
                        Some(back) => i64::try_from(back).unwrap_or(i64::MAX),
                        None => sequence::index(records.first_index, this).saturating_add(1),
                    })));
                }
                Ok(out)
            }
        }
    }
}

/// Recursion follows the bound tree, whose depth the parser bounds.
fn eval<R: AsRef<[Value]>>(
    node: &Node,
    records: &Records<'_, R>,
    at: At,
) -> Result<Datum, ClemError> {
    match node {
        Node::Const(datum) => Ok(datum.clone()),
        Node::Undef => Ok(Datum::NULL),
        Node::Field(position) => Ok(Datum::Value(
            records
                .value(at.row, *position)
                .cloned()
                .unwrap_or(Value::Null),
        )),
        // GAP: a value that is neither flag value; null.
        Node::Flag {
            position,
            true_value,
            false_value,
        } => Ok(match records.value(at.row, *position) {
            Some(value) if value.total_cmp(true_value).is_eq() => Datum::Bool(true),
            Some(value) if value.total_cmp(false_value).is_eq() => Datum::Bool(false),
            _ => Datum::NULL,
        }),
        Node::Index => Ok(Datum::Value(Value::Integer(sequence::index(
            records.first_index,
            at.row,
        )))),
        Node::Column(n) => Ok(records.column(*n, at)),
        Node::This(inner) => eval(
            inner,
            records,
            At {
                row: at.this,
                this: at.this,
            },
        ),
        Node::Offset(value, by) => {
            let target = match eval(by, records, at)? {
                Datum::Value(Value::Null) => return Ok(Datum::NULL),
                Datum::Value(Value::Integer(by)) => i64::try_from(at.row)
                    .ok()
                    .and_then(|row| row.checked_sub(by))
                    .and_then(|row| usize::try_from(row).ok())
                    .filter(|row| *row < records.rows.len()),
                other => {
                    return Err(ClemError::Type(format!(
                        "@OFFSET needs an integer offset, found {other:?}"
                    )));
                }
            };
            match target {
                Some(row) => eval(value, records, At { row, this: at.this }),
                None => Ok(Datum::NULL),
            }
        }
        Node::IsNull(inner) => Ok(Datum::Bool(eval(inner, records, at)?.is_null())),
        Node::IsBlank(position, missing) => Ok(Datum::Bool(
            records
                .value(at.row, *position)
                .is_some_and(|value| missing.is_blank(value)),
        )),
        Node::Negate(inner) => functions::negate(eval(inner, records, at)?),
        Node::Complement(inner) => functions::complement(eval(inner, records, at)?),
        Node::If(branches, otherwise) => {
            for (condition, value) in branches {
                // A null condition falls through to the else branch.
                if eval(condition, records, at)?.truth()? == Some(true) {
                    return eval(value, records, at);
                }
            }
            eval(otherwise, records, at)
        }
        Node::List(items) => Ok(Datum::Value(Value::List(
            items
                .iter()
                .map(|item| eval(item, records, at).and_then(Datum::into_value))
                .collect::<Result<Vec<_>, _>>()?
                .into(),
        ))),
        Node::Call(function, args) => {
            let values = args
                .iter()
                .map(|arg| eval(arg, records, at))
                .collect::<Result<Vec<_>, _>>()?;
            (function.apply)(&values)
        }
        Node::Chain(first, rest) => {
            let mut acc = eval(first, records, at)?;
            let mut left_undef = matches!(**first, Node::Undef);
            for (op, operand) in rest {
                let undef = left_undef || matches!(operand, Node::Undef);
                acc = match op {
                    BinaryOp::And => and(acc, || eval(operand, records, at))?,
                    BinaryOp::Or => or(acc, || eval(operand, records, at))?,
                    // Oracle: comparing with the keyword `undef` tests for null and is never
                    // null itself, while `m = m` on a null `m` is null.
                    BinaryOp::Equal | BinaryOp::NotEqual if undef => {
                        let right = eval(operand, records, at)?;
                        let equal = acc.is_null() == right.is_null();
                        Datum::Bool(equal == (*op == BinaryOp::Equal))
                    }
                    _ => binary(*op, acc, eval(operand, records, at)?)?,
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
fn ty(node: &Node, b: &Binder<'_>) -> Ty {
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
        Node::Offset(value, _) | Node::This(value) => ty(value, b),
        Node::Column(n) => match b.accumulators.borrow().get(*n) {
            Some(Accumulator {
                kind: Sequence::Statistic(Statistic::Mean | Statistic::Sdev),
                ..
            }) => Ty::Real,
            Some(Accumulator {
                kind: Sequence::Since { .. },
                ..
            }) => Ty::Integer,
            Some(accumulator) => ty(&accumulator.value, b),
            None => Ty::Unknown,
        },
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
fn widen(node: &mut Node, b: &Binder<'_>) {
    if ty(node, b) == Ty::Integer
        && let Some(to_real) = functions::lookup("to_real")
    {
        let inner = std::mem::replace(node, Node::Undef);
        *node = Node::Call(to_real, vec![inner]);
    }
}
