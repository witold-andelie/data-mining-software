//! Numeric, trigonometric, conversion and bitwise CLEM functions.

use super::{
    Function, Returns, f, first, integer, kind, negate, number, numeric1, parse_number,
    real_to_integer, real1, to_integer_with, to_real,
};
use crate::types::Ty;
use crate::{ClemError, Datum};
use dms_types::Value;

pub(super) const FUNCTIONS: &[Function] = &[
    f("abs", 1, |a| {
        numeric1(
            a,
            |i| i.checked_abs().map(Value::Integer),
            |r| Value::real(r.abs()),
        )
    })
    .returns(Returns::Arg(0)),
    f("sqrt", 1, |a| real1(a, |r| (r >= 0.0).then(|| r.sqrt()))).returns(Returns::Fixed(Ty::Real)),
    f("exp", 1, |a| real1(a, |r| Some(r.exp()))).returns(Returns::Fixed(Ty::Real)),
    f("log", 1, |a| real1(a, |r| (r > 0.0).then(|| r.ln()))).returns(Returns::Fixed(Ty::Real)),
    f("log10", 1, |a| real1(a, |r| (r > 0.0).then(|| r.log10()))).returns(Returns::Fixed(Ty::Real)),
    f("fracof", 1, |a| real1(a, |r| Some(r.fract()))).returns(Returns::Fixed(Ty::Real)),
    f("intof", 1, |a| to_integer_with(a, f64::trunc)).returns(Returns::Fixed(Ty::Integer)),
    // Oracle rule 25: half away from zero, as `f64::round`.
    f("round", 1, |a| to_integer_with(a, f64::round)).returns(Returns::Fixed(Ty::Integer)),
    f("sign", 1, |a| {
        numeric1(
            a,
            |i| Some(Value::Integer(i.signum())),
            |r| Value::real(if r == 0.0 { 0.0 } else { r.signum() }),
        )
    })
    .returns(Returns::Arg(0)),
    f("negate", 1, |a| negate(first(a)?)).returns(Returns::Arg(0)),
    // Trigonometric functions (GAP: stream angle setting defaults to radians).
    f("sin", 1, |a| real1(a, |r| Some(r.sin()))).returns(Returns::Fixed(Ty::Real)),
    f("cos", 1, |a| real1(a, |r| Some(r.cos()))).returns(Returns::Fixed(Ty::Real)),
    f("tan", 1, |a| real1(a, |r| Some(r.tan()))).returns(Returns::Fixed(Ty::Real)),
    f("arcsin", 1, |a| {
        real1(a, |r| (-1.0..=1.0).contains(&r).then(|| r.asin()))
    })
    .returns(Returns::Fixed(Ty::Real)),
    f("arccos", 1, |a| {
        real1(a, |r| (-1.0..=1.0).contains(&r).then(|| r.acos()))
    })
    .returns(Returns::Fixed(Ty::Real)),
    f("arctan", 1, |a| real1(a, |r| Some(r.atan()))).returns(Returns::Fixed(Ty::Real)),
    f("arctan2", 2, |a| {
        let (Some(y), Some(x)) = (a.first(), a.get(1)) else {
            return Err(ClemError::Type("arctan2 needs two arguments".into()));
        };
        if y.is_null() || x.is_null() {
            return Ok(Datum::NULL);
        }
        Ok(Datum::Value(Value::real(number(y)?.atan2(number(x)?))))
    })
    .returns(Returns::Fixed(Ty::Real)),
    f("sinh", 1, |a| real1(a, |r| Some(r.sinh()))).returns(Returns::Fixed(Ty::Real)),
    f("cosh", 1, |a| real1(a, |r| Some(r.cosh()))).returns(Returns::Fixed(Ty::Real)),
    f("tanh", 1, |a| real1(a, |r| Some(r.tanh()))).returns(Returns::Fixed(Ty::Real)),
    f("arcsinh", 1, |a| real1(a, |r| Some(r.asinh()))).returns(Returns::Fixed(Ty::Real)),
    f("arccosh", 1, |a| {
        real1(a, |r| (r >= 1.0).then(|| r.acosh()))
    })
    .returns(Returns::Fixed(Ty::Real)),
    f("arctanh", 1, |a| {
        real1(a, |r| (-1.0 < r && r < 1.0).then(|| r.atanh()))
    })
    .returns(Returns::Fixed(Ty::Real)),
    // Oracle rule 29: two-argument `min`/`max` propagate null (unlike `min_n`/`max_n` on lists)
    // and widen mixed integer/real pairs to real.
    f("min", 2, |a| min_max(a, false)).returns(Returns::Common),
    f("max", 2, |a| min_max(a, true)).returns(Returns::Common),
    // Oracle rule 31: 1-bits for >= 0, 0-bits for < 0 in two's complement.
    f("integer_bitcount", 1, |a| {
        let value = first(a)?;
        if value.is_null() {
            return Ok(Datum::NULL);
        }
        let i = integer(&value)?;
        let count = if i >= 0 {
            i.count_ones()
        } else {
            (!i).count_ones()
        };
        Ok(Datum::Value(Value::Integer(i64::from(count))))
    })
    .returns(Returns::Fixed(Ty::Integer)),
    // Oracle rule 31: 0-based position of the least significant set bit; `0` returns `31`.
    f("integer_leastbit", 1, |a| {
        let value = first(a)?;
        if value.is_null() {
            return Ok(Datum::NULL);
        }
        let i = integer(&value)?;
        let bit = if i == 0 {
            31
        } else {
            i64::from(i.trailing_zeros())
        };
        Ok(Datum::Value(Value::Integer(bit)))
    })
    .returns(Returns::Fixed(Ty::Integer)),
    // Oracle rule 31: smallest N >= 1 with i < (1 << N) (for i >= 0) or i >= (-1 << N) (i < 0);
    // both `0` and `-1` return `1`.
    f("integer_length", 1, |a| {
        let value = first(a)?;
        if value.is_null() {
            return Ok(Datum::NULL);
        }
        let i = integer(&value)?;
        let magnitude = if i >= 0 { i as u64 } else { (!i) as u64 };
        let bits = (u64::BITS - magnitude.leading_zeros()).max(1);
        Ok(Datum::Value(Value::Integer(i64::from(bits))))
    })
    .returns(Returns::Fixed(Ty::Integer)),
    f("testbit", 2, |a| {
        let (Some(val), Some(bit)) = (a.first(), a.get(1)) else {
            return Err(ClemError::Type("testbit needs two arguments".into()));
        };
        if val.is_null() || bit.is_null() {
            return Ok(Datum::NULL);
        }
        let (val, bit) = (integer(val)?, integer(bit)?);
        // GAP: negative bit index returns false; bit >= 64 follows two's-complement sign bit.
        let set = match u32::try_from(bit) {
            Ok(shift) if shift < 64 => ((val >> shift) & 1) != 0,
            Ok(_) => val < 0,
            Err(_) => false,
        };
        Ok(Datum::Bool(set))
    })
    .returns(Returns::Fixed(Ty::Bool)),
    // Oracle rule 24: truncates toward zero, also for decimal strings.
    f("to_integer", 1, |a| {
        Ok(match parse_number(a)? {
            Some(Value::Real(r)) => Datum::Value(real_to_integer(r.trunc())),
            Some(value) => Datum::Value(value),
            None => Datum::NULL,
        })
    })
    .returns(Returns::Fixed(Ty::Integer)),
    // Oracle rule 26: both give reals.
    f("to_real", 1, to_real).returns(Returns::Fixed(Ty::Real)),
    f("to_number", 1, to_real).returns(Returns::Fixed(Ty::Real)),
    f("is_integer", 1, |a| {
        kind(a, |v| matches!(v, Value::Integer(_)))
    })
    .returns(Returns::Fixed(Ty::Bool))
    .type_test(|t| matches!(t, Ty::Integer)),
    f("is_real", 1, |a| kind(a, |v| matches!(v, Value::Real(_))))
        .returns(Returns::Fixed(Ty::Bool))
        .type_test(|t| matches!(t, Ty::Real)),
    f("is_number", 1, |a| {
        kind(a, |v| matches!(v, Value::Integer(_) | Value::Real(_)))
    })
    .returns(Returns::Fixed(Ty::Bool))
    .type_test(|t| matches!(t, Ty::Integer | Ty::Real)),
];

fn min_max(args: &[Datum], pick_max: bool) -> Result<Datum, ClemError> {
    let (Some(left), Some(right)) = (args.first(), args.get(1)) else {
        return Err(ClemError::Type("min/max needs two arguments".into()));
    };
    if left.is_null() || right.is_null() {
        return Ok(Datum::NULL);
    }
    let choose = |ord: std::cmp::Ordering| {
        if pick_max {
            ord != std::cmp::Ordering::Less
        } else {
            ord != std::cmp::Ordering::Greater
        }
    };
    match (left, right) {
        (Datum::Value(Value::Integer(a)), Datum::Value(Value::Integer(b))) => {
            Ok(Datum::Value(Value::Integer(if choose(a.cmp(b)) {
                *a
            } else {
                *b
            })))
        }
        (
            Datum::Value(Value::Integer(_) | Value::Real(_)),
            Datum::Value(Value::Integer(_) | Value::Real(_)),
        ) => {
            let (a, b) = (number(left)?, number(right)?);
            Ok(Datum::Value(Value::real(if choose(a.total_cmp(&b)) {
                a
            } else {
                b
            })))
        }
        (Datum::Value(Value::Date(a)), Datum::Value(Value::Date(b))) => {
            Ok(Datum::Value(Value::Date(if choose(a.cmp(b)) {
                *a
            } else {
                *b
            })))
        }
        (Datum::Value(Value::Time(a)), Datum::Value(Value::Time(b))) => {
            Ok(Datum::Value(Value::Time(if choose(a.cmp(b)) {
                *a
            } else {
                *b
            })))
        }
        (Datum::Value(Value::Timestamp(a)), Datum::Value(Value::Timestamp(b))) => {
            Ok(Datum::Value(Value::Timestamp(if choose(a.cmp(b)) {
                *a
            } else {
                *b
            })))
        }
        (Datum::Value(Value::String(a)), Datum::Value(Value::String(b))) => {
            Ok(Datum::Value(Value::String(if choose(a.cmp(b)) {
                a.clone()
            } else {
                b.clone()
            })))
        }
        _ => Err(ClemError::Type(format!(
            "cannot take min/max of {left:?} and {right:?}"
        ))),
    }
}
