//! Operators and built-in functions (docs/clem/functions.tsv). A null argument gives a null
//! result unless noted; domain errors (division by zero, square root of a negative number)
//! give null, as the user guide describes.
//!
//! Choices where IBM's documentation is silent are marked GAP and listed for oracle probes.

use crate::parser::BinaryOp;
use crate::{ClemError, Datum};
use dms_types::Value;

#[derive(Debug, Clone, Copy)]
pub struct Function {
    pub name: &'static str,
    pub arity: usize,
    pub apply: fn(&[Datum]) -> Result<Datum, ClemError>,
}

const FUNCTIONS: &[Function] = &[
    f("abs", 1, |a| {
        numeric1(
            a,
            |i| i.checked_abs().map(Value::Integer),
            |r| Value::real(r.abs()),
        )
    }),
    f("sqrt", 1, |a| real1(a, |r| (r >= 0.0).then(|| r.sqrt()))),
    f("exp", 1, |a| real1(a, |r| Some(r.exp()))),
    f("log", 1, |a| real1(a, |r| (r > 0.0).then(|| r.ln()))),
    f("log10", 1, |a| real1(a, |r| (r > 0.0).then(|| r.log10()))),
    f("fracof", 1, |a| real1(a, |r| Some(r.fract()))),
    f("intof", 1, |a| to_integer_with(a, f64::trunc)),
    // Oracle rule 25: half away from zero, as `f64::round`.
    f("round", 1, |a| to_integer_with(a, f64::round)),
    f("sign", 1, |a| {
        numeric1(
            a,
            |i| Some(Value::Integer(i.signum())),
            |r| Value::real(if r == 0.0 { 0.0 } else { r.signum() }),
        )
    }),
    f("negate", 1, |a| negate(first(a)?)),
    f("not", 1, |a| {
        Ok(first(a)?.truth()?.map_or(Datum::NULL, |t| Datum::Bool(!t)))
    }),
    // Oracle rule 24: truncates toward zero, also for decimal strings.
    f("to_integer", 1, |a| {
        Ok(match parse_number(a)? {
            Some(Value::Real(r)) => Datum::Value(real_to_integer(r.trunc())),
            Some(value) => Datum::Value(value),
            None => Datum::NULL,
        })
    }),
    // Oracle rule 26: both give reals.
    f("to_real", 1, to_real),
    f("to_number", 1, to_real),
    f("to_string", 1, |a| {
        let value = first(a)?;
        Ok(if value.is_null() {
            Datum::NULL
        } else {
            Datum::Value(Value::string(value.to_text()?))
        })
    }),
    f("length", 1, |a| {
        string1(a, |s| {
            Value::Integer(i64::try_from(s.chars().count()).unwrap_or(i64::MAX))
        })
    }),
    f("lowertoupper", 1, |a| {
        string1(a, |s| Value::string(s.to_uppercase()))
    }),
    f("uppertolower", 1, |a| {
        string1(a, |s| Value::string(s.to_lowercase()))
    }),
    f("trim", 1, |a| string1(a, |s| Value::string(s.trim()))),
    // Oracle rule 45: `trimstart`; the guide's `trim_start` does not exist.
    f("trimstart", 1, |a| {
        string1(a, |s| Value::string(s.trim_start()))
    }),
    f("trimend", 1, |a| {
        string1(a, |s| Value::string(s.trim_end()))
    }),
    // substring(N, LEN, STRING): LEN characters from 1-based position N. Oracle rule 42: a
    // range outside the string gives null instead of being clamped.
    f("substring", 3, |a| {
        let (Some(start), Some(length), Some(text)) = (a.first(), a.get(1), a.get(2)) else {
            return Err(ClemError::Type("substring needs three arguments".into()));
        };
        if start.is_null() || length.is_null() || text.is_null() {
            return Ok(Datum::NULL);
        }
        let (start, length) = (integer(start)?, integer(length)?);
        let text = string(text)?;
        let count = i64::try_from(text.chars().count()).unwrap_or(i64::MAX);
        if start < 1 || length < 0 || (start - 1).saturating_add(length) > count {
            return Ok(Datum::NULL);
        }
        let skip = usize::try_from(start - 1).unwrap_or(usize::MAX);
        let take = usize::try_from(length).unwrap_or(usize::MAX);
        Ok(Datum::Value(Value::string(
            text.chars().skip(skip).take(take).collect::<String>(),
        )))
    }),
    // member(ITEM, LIST)
    f("member", 2, |a| {
        let (Some(item), Some(Datum::Value(Value::List(list)))) = (a.first(), a.get(1)) else {
            return Err(ClemError::Type("member needs an item and a list".into()));
        };
        if item.is_null() {
            return Ok(Datum::NULL);
        }
        let item = item.clone().into_value()?;
        Ok(Datum::Bool(list.iter().any(|v| v.total_cmp(&item).is_eq())))
    }),
    f("is_integer", 1, |a| {
        kind(a, |v| matches!(v, Value::Integer(_)))
    }),
    f("is_real", 1, |a| kind(a, |v| matches!(v, Value::Real(_)))),
    f("is_number", 1, |a| {
        kind(a, |v| matches!(v, Value::Integer(_) | Value::Real(_)))
    }),
    f("is_string", 1, |a| {
        kind(a, |v| matches!(v, Value::String(_)))
    }),
];

const fn f(
    name: &'static str,
    arity: usize,
    apply: fn(&[Datum]) -> Result<Datum, ClemError>,
) -> Function {
    Function { name, arity, apply }
}

pub fn lookup(name: &str) -> Option<Function> {
    FUNCTIONS
        .iter()
        .find(|function| function.name == name)
        .copied()
}

/// Names of all implemented functions (for coverage reports).
pub fn names() -> impl Iterator<Item = &'static str> {
    FUNCTIONS.iter().map(|function| function.name)
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
                None => next[j] = reachable[j] || (j > 0 && next[j - 1]),
                Some(wanted) => {
                    next[j] = j > 0
                        && reachable[j - 1]
                        && wanted.is_none_or(|c| text.get(j - 1) == Some(&c));
                }
            }
        }
        reachable = next;
    }
    Ok(Datum::Bool(reachable[text.len()]))
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
