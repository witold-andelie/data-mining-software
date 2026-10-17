//! Operators and built-in functions (docs/clem/functions.tsv). A null argument gives a null
//! result unless noted; domain errors (division by zero, square root of a negative number)
//! give null, as the user guide describes.
//!
//! Choices where IBM's documentation is silent are marked GAP and listed for oracle probes.

mod dates;
mod lists;
mod numeric;
mod strings;

use crate::parser::BinaryOp;
use crate::types::Ty;
use crate::{ClemError, Datum};
use dms_types::Value;
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy)]
pub struct Function {
    pub name: &'static str,
    /// Accepted argument counts.
    pub arities: &'static [usize],
    /// What binding checks about the written arguments.
    pub args: Args,
    /// The result type, for binding.
    pub returns: Returns,
    /// For type tests: the answer from the argument's static type, when it is known (oracle:
    /// `is_integer` of a null integer field is true).
    pub type_test: Option<fn(Ty) -> bool>,
    pub apply: fn(&[Datum]) -> Result<Datum, ClemError>,
}

/// Argument checks Modeler makes when it compiles an expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Args {
    Any,
    /// The arguments are conditions: a flag field stands for its truth value.
    Condition,
    /// The first argument is a backquoted character; `'c'` and `"c"` are rejected (oracle
    /// rule 44).
    CharFirst,
    /// Argument `n` is a list of fields; literals are rejected (oracle rule 30).
    Fields(usize),
    /// Argument `n` is a list of only literals or only fields (oracle rule 30, `member`).
    Uniform(usize),
}

/// How the result type follows from the arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Returns {
    Fixed(Ty),
    /// The type of argument `n`.
    Arg(usize),
    /// The common type of all arguments.
    Common,
    /// The common type of the items of the list at argument `n`.
    Items(usize),
}

impl Function {
    const fn returns(self, returns: Returns) -> Function {
        Function { returns, ..self }
    }

    const fn args(self, args: Args) -> Function {
        Function { args, ..self }
    }

    const fn arities(self, arities: &'static [usize]) -> Function {
        Function { arities, ..self }
    }

    const fn type_test(self, test: fn(Ty) -> bool) -> Function {
        Function {
            type_test: Some(test),
            ..self
        }
    }
}

const LOGIC: &[Function] = &[f("not", 1, |a| {
    Ok(first(a)?.truth()?.map_or(Datum::NULL, |t| Datum::Bool(!t)))
})
.returns(Returns::Fixed(Ty::Bool))
.args(Args::Condition)];

const GROUPS: &[&[Function]] = &[
    LOGIC,
    numeric::FUNCTIONS,
    lists::FUNCTIONS,
    strings::FUNCTIONS,
    dates::FUNCTIONS,
];

/// Argument counts by number, for `f`.
const ARITIES: [&[usize]; 6] = [&[0], &[1], &[2], &[3], &[4], &[5]];

/// A function taking `arity` arguments (at most 5; others use `arities`), of unknown result
/// type until `returns` says otherwise.
const fn f(
    name: &'static str,
    arity: usize,
    apply: fn(&[Datum]) -> Result<Datum, ClemError>,
) -> Function {
    Function {
        name,
        arities: ARITIES[arity],
        args: Args::Any,
        returns: Returns::Fixed(Ty::Unknown),
        type_test: None,
        apply,
    }
}

pub fn lookup(name: &str) -> Option<Function> {
    GROUPS
        .iter()
        .flat_map(|group| group.iter())
        .find(|function| function.name == name)
        .copied()
}

/// Names of all implemented functions (for coverage reports).
pub fn names() -> impl Iterator<Item = &'static str> {
    GROUPS
        .iter()
        .flat_map(|group| group.iter().map(|function| function.name))
        .chain(
            crate::GlobalStatistic::ALL
                .into_iter()
                .map(crate::GlobalStatistic::function),
        )
}

fn first(args: &[Datum]) -> Result<Datum, ClemError> {
    args.first()
        .cloned()
        .ok_or_else(|| ClemError::Type("missing argument".into()))
}

fn number(datum: &Datum) -> Result<f64, ClemError> {
    match datum {
        Datum::Value(value) => value
            .as_f64()
            .ok_or_else(|| ClemError::Type(format!("expected a number, found {value:?}"))),
        Datum::Bool(_) => Err(ClemError::Type("expected a number, found a boolean".into())),
    }
}

fn integer(datum: &Datum) -> Result<i64, ClemError> {
    match datum {
        Datum::Value(Value::Integer(i)) => Ok(*i),
        other => Err(ClemError::Type(format!(
            "expected an integer, found {other:?}"
        ))),
    }
}

fn string(datum: &Datum) -> Result<String, ClemError> {
    match datum {
        Datum::Value(Value::String(s)) => Ok(s.to_string()),
        other => Err(ClemError::Type(format!(
            "expected a string, found {other:?}"
        ))),
    }
}

/// Extracts a single character from a one-character string (`Token::Char` or `subscrs`).
fn char_arg(datum: &Datum) -> Result<char, ClemError> {
    match datum {
        Datum::Value(Value::String(s)) => {
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Ok(c),
                _ => Err(ClemError::Type(format!(
                    "expected a single character, found {s:?}"
                ))),
            }
        }
        other => Err(ClemError::Type(format!(
            "expected a character, found {other:?}"
        ))),
    }
}

/// CLEM's order of two comparable values: like `Value::total_cmp`, except that numbers compare
/// by IEEE value, so `-0.0 = 0.0` (`ibm-demos/condlearn`: `PowerInc * @OFFSET(PowerInc, 1) < 0`
/// is false when one factor is 0 and the product is -0.0).
pub(crate) fn value_order(left: &Value, right: &Value) -> Ordering {
    match (left, right) {
        (Value::Integer(_) | Value::Real(_), Value::Integer(_) | Value::Real(_))
            if !matches!((left, right), (Value::Integer(_), Value::Integer(_))) =>
        {
            let a = left.as_f64().unwrap_or(0.0);
            let b = right.as_f64().unwrap_or(0.0);
            a.partial_cmp(&b).unwrap_or_else(|| a.total_cmp(&b))
        }
        _ => left.total_cmp(right),
    }
}

/// Total ordering across comparable CLEM values (numbers, strings, dates, times, timestamps).
fn compare_values(left: &Value, right: &Value) -> Result<Ordering, ClemError> {
    let comparable = matches!(
        (left, right),
        (
            Value::Integer(_) | Value::Real(_),
            Value::Integer(_) | Value::Real(_)
        ) | (Value::String(_), Value::String(_))
            | (Value::Date(_), Value::Date(_))
            | (Value::Time(_), Value::Time(_))
            | (Value::Timestamp(_), Value::Timestamp(_))
    );
    if comparable {
        Ok(value_order(left, right))
    } else {
        Err(ClemError::Type(format!(
            "cannot compare {left:?} with {right:?}"
        )))
    }
}

fn numeric1(
    args: &[Datum],
    on_integer: fn(i64) -> Option<Value>,
    on_real: fn(f64) -> Value,
) -> Result<Datum, ClemError> {
    match first(args)? {
        Datum::Value(Value::Null) => Ok(Datum::NULL),
        Datum::Value(Value::Integer(i)) => Ok(Datum::Value(
            on_integer(i).unwrap_or_else(|| on_real(i as f64)),
        )),
        other => Ok(Datum::Value(on_real(number(&other)?))),
    }
}

/// A real-valued function; `None` (domain error) gives null.
fn real1(args: &[Datum], apply: fn(f64) -> Option<f64>) -> Result<Datum, ClemError> {
    let value = first(args)?;
    if value.is_null() {
        return Ok(Datum::NULL);
    }
    Ok(Datum::Value(
        apply(number(&value)?).map_or(Value::Null, Value::real),
    ))
}

fn to_integer_with(args: &[Datum], round: fn(f64) -> f64) -> Result<Datum, ClemError> {
    match first(args)? {
        Datum::Value(Value::Null) => Ok(Datum::NULL),
        Datum::Value(Value::Integer(i)) => Ok(Datum::Value(Value::Integer(i))),
        other => Ok(Datum::Value(real_to_integer(round(number(&other)?)))),
    }
}

/// An integral real as an integer; null outside the integer range (oracle rule 24:
/// `to_integer(1e20)` is null).
fn real_to_integer(value: f64) -> Value {
    // 2^63: the first real above the integer range.
    const LIMIT: f64 = 9_223_372_036_854_775_808.0;
    if (-LIMIT..LIMIT).contains(&value) {
        Value::Integer(value as i64)
    } else {
        Value::Null
    }
}

/// A number from a number or a string (trimmed); unparsable strings give `None` like null.
fn parse_number(args: &[Datum]) -> Result<Option<Value>, ClemError> {
    Ok(match first(args)? {
        Datum::Value(Value::Null) => None,
        Datum::Value(Value::String(text)) => {
            let text = text.trim();
            match text.parse::<i64>() {
                Ok(i) => Some(Value::Integer(i)),
                Err(_) => text.parse::<f64>().ok().map(Value::real),
            }
        }
        Datum::Value(value @ (Value::Integer(_) | Value::Real(_))) => Some(value),
        other => {
            return Err(ClemError::Type(format!(
                "cannot convert {other:?} to a number"
            )));
        }
    })
}

fn to_real(args: &[Datum]) -> Result<Datum, ClemError> {
    Ok(match parse_number(args)? {
        Some(Value::Integer(i)) => Datum::Value(Value::Real(i as f64)),
        Some(value) => Datum::Value(value),
        None => Datum::NULL,
    })
}

fn string1(args: &[Datum], apply: fn(&str) -> Value) -> Result<Datum, ClemError> {
    let value = first(args)?;
    if value.is_null() {
        return Ok(Datum::NULL);
    }
    Ok(Datum::Value(apply(&string(&value)?)))
}

fn kind(args: &[Datum], test: fn(&Value) -> bool) -> Result<Datum, ClemError> {
    Ok(match first(args)? {
        Datum::Value(Value::Null) => Datum::NULL,
        Datum::Value(value) => Datum::Bool(test(&value)),
        Datum::Bool(_) => Datum::Bool(false),
    })
}

pub fn negate(value: Datum) -> Result<Datum, ClemError> {
    Ok(match value {
        Datum::Value(Value::Null) => Datum::NULL,
        Datum::Value(Value::Integer(i)) => Datum::Value(Value::Integer(i.wrapping_neg())),
        other => Datum::Value(Value::real(-number(&other)?)),
    })
}

/// `~~`: bitwise complement of an integer.
pub fn complement(value: Datum) -> Result<Datum, ClemError> {
    match value {
        Datum::Value(Value::Null) => Ok(Datum::NULL),
        Datum::Value(Value::Integer(i)) => Ok(Datum::Value(Value::Integer(!i))),
        other => Err(ClemError::Type(format!(
            "~~ needs an integer, found {other:?}"
        ))),
    }
}

/// `&&=_0` and `&&/=_0`: whether two integers share no set bit, or some.
pub fn bits_test(op: BinaryOp, left: Datum, right: Datum) -> Result<Datum, ClemError> {
    if left.is_null() || right.is_null() {
        return Ok(Datum::NULL);
    }
    let shared = integer(&left)? & integer(&right)?;
    Ok(Datum::Bool((shared == 0) == (op == BinaryOp::BitsClear)))
}

/// Arithmetic and bitwise operators, as the oracle showed them (docs/clem/oracle-results.md
/// rules 17-23): integers are 64-bit and wrap on overflow; `/` always gives a real; `div`
/// truncates toward zero, `rem` takes the sign of the dividend and `mod` that of the divisor;
/// any division by zero gives null; real overflow gives an infinity.
pub fn arithmetic(op: BinaryOp, left: Datum, right: Datum) -> Result<Datum, ClemError> {
    if left.is_null() || right.is_null() {
        return Ok(Datum::NULL);
    }
    if let (Datum::Value(Value::Integer(a)), Datum::Value(Value::Integer(b))) = (&left, &right) {
        return integer_arithmetic(op, *a, *b);
    }
    let (a, b) = (number(&left)?, number(&right)?);
    let result = match op {
        BinaryOp::Add => a + b,
        BinaryOp::Subtract => a - b,
        BinaryOp::Multiply => a * b,
        BinaryOp::Divide | BinaryOp::Div | BinaryOp::Rem | BinaryOp::Mod if b == 0.0 => {
            return Ok(Datum::NULL);
        }
        BinaryOp::Divide => a / b,
        BinaryOp::Power => a.powf(b),
        // Oracle rule 20: `div` on reals gives a 32-bit integer (`1e10 div 1` is
        // -2147483648, the x86 result for an out-of-range conversion).
        BinaryOp::Div => {
            let quotient = (a / b).trunc();
            let value = if (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&quotient) {
                quotient as i64
            } else {
                i64::from(i32::MIN)
            };
            return Ok(Datum::Value(Value::Integer(value)));
        }
        BinaryOp::Rem => a % b,
        // GAP: `mod` on reals; floored like on integers.
        BinaryOp::Mod => floored(a % b, b),
        other => {
            return Err(ClemError::Type(format!("{other:?} needs integer operands")));
        }
    };
    Ok(Datum::Value(Value::real(result)))
}

fn integer_arithmetic(op: BinaryOp, a: i64, b: i64) -> Result<Datum, ClemError> {
    let value = match op {
        BinaryOp::Add => a.wrapping_add(b),
        BinaryOp::Subtract => a.wrapping_sub(b),
        BinaryOp::Multiply => a.wrapping_mul(b),
        BinaryOp::Divide if b == 0 => return Ok(Datum::NULL),
        BinaryOp::Divide => return Ok(Datum::Value(Value::Real(a as f64 / b as f64))),
        BinaryOp::Div | BinaryOp::Rem | BinaryOp::Mod if b == 0 => return Ok(Datum::NULL),
        BinaryOp::Div => a.wrapping_div(b),
        BinaryOp::Rem => a.wrapping_rem(b),
        BinaryOp::Mod => floored(a.wrapping_rem(b), b),
        // Oracle rules 22-23: an integer power out of range is the smallest integer (`2 ** 63`,
        // `10 ** 20`); a negative exponent truncates the real power (`2 ** -1` is 0).
        BinaryOp::Power => match u32::try_from(b) {
            Ok(exponent) => a.checked_pow(exponent).unwrap_or(i64::MIN),
            Err(_) if b < 0 => match real_to_integer((a as f64).powf(b as f64).trunc()) {
                Value::Integer(value) => value,
                _ => return Ok(Datum::NULL),
            },
            Err(_) => match a {
                0 | 1 => a,
                -1 => 1 - 2 * (b & 1),
                _ => i64::MIN,
            },
        },
        BinaryOp::BitAnd => a & b,
        BinaryOp::BitAndNot => a & !b,
        BinaryOp::BitOr => a | b,
        BinaryOp::BitXor => a ^ b,
        // Oracle rule 15: a negative shift count gives 0. GAP: `>>` by a negative count.
        BinaryOp::ShiftLeft => u32::try_from(b)
            .ok()
            .and_then(|count| a.checked_shl(count))
            .unwrap_or(0),
        BinaryOp::ShiftRight => match u32::try_from(b) {
            Ok(count) => a.checked_shr(count).unwrap_or(if a < 0 { -1 } else { 0 }),
            Err(_) => 0,
        },
        other => return Err(ClemError::Type(format!("{other:?} is not arithmetic"))),
    };
    Ok(Datum::Value(Value::Integer(value)))
}

/// `STRING matches PATTERN` (oracle rule 46): case-sensitive; `*` matches any text, `?` one
/// character, a backslash makes the next character literal; everything else is literal.
pub fn matches(text: Datum, pattern: Datum) -> Result<Datum, ClemError> {
    if text.is_null() || pattern.is_null() {
        return Ok(Datum::NULL);
    }
    let text: Vec<char> = string(&text)?.chars().collect();
    let mut tokens = Vec::new();
    let mut chars = string(&pattern)?.chars().collect::<Vec<_>>().into_iter();
    while let Some(c) = chars.next() {
        tokens.push(match c {
            '*' => None,
            '?' => Some(None),
            '\\' => Some(Some(chars.next().unwrap_or('\\'))),
            other => Some(Some(other)),
        });
    }
    // reachable[j]: the pattern so far can match the first j characters (O(n*m) time).
    let mut reachable = vec![false; text.len() + 1];
    reachable[0] = true;
    for token in tokens {
        let mut next = vec![false; text.len() + 1];
        for j in 0..=text.len() {
            match token {
                None => {
                    next[j] = reachable.get(j).copied().unwrap_or(false)
                        || (j > 0 && next.get(j - 1).copied().unwrap_or(false));
                }
                Some(wanted) => {
                    next[j] = j > 0
                        && reachable.get(j - 1).copied().unwrap_or(false)
                        && wanted.is_none_or(|c| text.get(j - 1) == Some(&c));
                }
            }
        }
        reachable = next;
    }
    Ok(Datum::Bool(
        reachable.get(text.len()).copied().unwrap_or(false),
    ))
}

/// A truncated remainder moved to the sign of the divisor.
fn floored<T>(remainder: T, divisor: T) -> T
where
    T: Copy + PartialOrd + Default + std::ops::Add<Output = T>,
{
    let zero = T::default();
    if remainder != zero && (remainder < zero) != (divisor < zero) {
        remainder + divisor
    } else {
        remainder
    }
}
