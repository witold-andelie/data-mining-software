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
    // GAP: whether to_integer rounds or truncates reals; rounding half away from zero.
    f("to_integer", 1, |a| to_number(a, true)),
    f("to_real", 1, |a| {
        to_number(a, false).map(|d| match d {
            Datum::Value(Value::Integer(i)) => Datum::Value(Value::Real(i as f64)),
            other => other,
        })
    }),
    f("to_number", 1, |a| to_number(a, false)),
    f("to_string", 1, |a| {
        let value = first(a)?;
        Ok(if value.is_null() {
            Datum::NULL
        } else {
            Datum::Value(Value::string(value.to_text()))
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
    f("trim_start", 1, |a| {
        string1(a, |s| Value::string(s.trim_start()))
    }),
    f("trimend", 1, |a| {
        string1(a, |s| Value::string(s.trim_end()))
    }),
    // substring(N, LEN, STRING): LEN characters from 1-based position N.
    // GAP: out-of-range positions; we return the characters that exist.
    f("substring", 3, |a| {
        let (Some(start), Some(length), Some(text)) = (a.first(), a.get(1), a.get(2)) else {
            return Err(ClemError::Type("substring needs three arguments".into()));
        };
        if start.is_null() || length.is_null() || text.is_null() {
            return Ok(Datum::NULL);
        }
        let (start, length) = (integer(start)?, integer(length)?);
        let text = string(text)?;
        let skip = usize::try_from(start.saturating_sub(1)).unwrap_or(0);
        let take = usize::try_from(length).unwrap_or(0);
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

/// An integral real as an integer, or the real itself when it does not fit.
fn real_to_integer(value: f64) -> Value {
    if value.is_finite() && value.abs() < 9.0e18 {
        Value::Integer(value as i64)
    } else {
        Value::real(value)
    }
}

/// GAP: unparsable strings give null.
fn to_number(args: &[Datum], integer_result: bool) -> Result<Datum, ClemError> {
    let value = match first(args)? {
        Datum::Value(Value::Null) => return Ok(Datum::NULL),
        Datum::Value(Value::String(text)) => {
            let text = text.trim();
            match text.parse::<i64>() {
                Ok(i) => Value::Integer(i),
                Err(_) => match text.parse::<f64>() {
                    Ok(r) => Value::real(r),
                    Err(_) => return Ok(Datum::NULL),
                },
            }
        }
        Datum::Value(v @ (Value::Integer(_) | Value::Real(_))) => v,
        other => {
            return Err(ClemError::Type(format!(
                "cannot convert {other:?} to a number"
            )));
        }
    };
    Ok(Datum::Value(match (integer_result, value) {
        (true, Value::Real(r)) => real_to_integer(r.round()),
        (_, other) => other,
    }))
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
        Datum::Value(Value::Integer(i)) => Datum::Value(
            i.checked_neg()
                .map_or_else(|| Value::real(-(i as f64)), Value::Integer),
        ),
        other => Datum::Value(Value::real(-number(&other)?)),
    })
}

/// `+ - * / ** div rem`. Integers stay integers for `+ - * div rem` and non-negative integer
/// powers, falling back to reals on overflow. GAP: `/` always gives a real.
pub fn arithmetic(op: BinaryOp, left: Datum, right: Datum) -> Result<Datum, ClemError> {
    if left.is_null() || right.is_null() {
        return Ok(Datum::NULL);
    }
    if let (Datum::Value(Value::Integer(a)), Datum::Value(Value::Integer(b))) = (&left, &right) {
        let (a, b) = (*a, *b);
        let exact = match op {
            BinaryOp::Add => a.checked_add(b),
            BinaryOp::Subtract => a.checked_sub(b),
            BinaryOp::Multiply => a.checked_mul(b),
            // GAP: sign of `div`/`rem` with negative operands; truncation toward zero.
            BinaryOp::Div | BinaryOp::Rem if b == 0 => return Ok(Datum::NULL),
            BinaryOp::Div => a.checked_div(b),
            BinaryOp::Rem => a.checked_rem(b),
            BinaryOp::Power if b >= 0 => u32::try_from(b).ok().and_then(|e| a.checked_pow(e)),
            _ => None,
        };
        if let Some(value) = exact {
            return Ok(Datum::Value(Value::Integer(value)));
        }
    }
    if matches!(op, BinaryOp::Div | BinaryOp::Rem) {
        return Err(ClemError::Type("div and rem need integer operands".into()));
    }
    let (a, b) = (number(&left)?, number(&right)?);
    let result = match op {
        BinaryOp::Add => a + b,
        BinaryOp::Subtract => a - b,
        BinaryOp::Multiply => a * b,
        BinaryOp::Divide if b == 0.0 => return Ok(Datum::NULL),
        BinaryOp::Divide => a / b,
        BinaryOp::Power => a.powf(b),
        other => return Err(ClemError::Type(format!("{other:?} is not arithmetic"))),
    };
    Ok(Datum::Value(if result.is_finite() {
        Value::Real(result)
    } else {
        Value::Null
    }))
}
