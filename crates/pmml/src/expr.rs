//! Evaluation of PMML transformation expressions (`Expr`, `NormContinuous`, `NormDiscrete`,
//! built-in functions, and `DefineFunction` calls).

use crate::score::Env;
use crate::{DefineFunction, Expr, Outliers, PmmlError, Value};

const MAX_DEPTH: usize = 64;

pub(crate) fn eval(
    expr: &Expr,
    env: &Env<'_>,
    functions: &[DefineFunction],
    params: &[(&str, Value)],
    depth: usize,
) -> Result<Value, PmmlError> {
    if depth > MAX_DEPTH {
        return Err(PmmlError::Score(
            "expression evaluation depth limit exceeded".into(),
        ));
    }
    match expr {
        Expr::Constant(v) => Ok(v.clone()),
        Expr::FieldRef {
            field,
            map_missing_to,
        } => {
            let val = lookup(field, env, params)?;
            if val == Value::Missing {
                Ok(map_missing_to.clone().unwrap_or(Value::Missing))
            } else {
                Ok(val)
            }
        }
        Expr::NormContinuous {
            field,
            points,
            outliers,
            map_missing_to,
        } => {
            let val = lookup(field, env, params)?;
            if val == Value::Missing {
                return Ok(map_missing_to.map_or(Value::Missing, Value::Number));
            }
            let x = as_f64(&val, field)?;
            eval_norm_continuous(x, points, *outliers, *map_missing_to)
        }
        Expr::MapValues {
            pairs,
            output,
            rows,
            default,
            map_missing_to,
        } => {
            let mut keys = Vec::with_capacity(pairs.len());
            for (field, column) in pairs {
                let value = lookup(field, env, params)?;
                if value == Value::Missing {
                    return Ok(map_missing_to.clone().unwrap_or(Value::Missing));
                }
                keys.push((column, value));
            }
            fn cell<'r>(row: &'r [(String, String)], column: &str) -> Option<&'r str> {
                row.iter()
                    .find(|(c, _)| c == column)
                    .map(|(_, v)| v.as_str())
            }
            let hit = rows.iter().find(|row| {
                keys.iter().all(|(column, value)| {
                    cell(row, column).is_some_and(|text| value_matches_str(value, text))
                })
            });
            Ok(match hit.and_then(|row| cell(row, output)) {
                Some(text) => crate::parse::parse_value_str(text),
                None => default.clone().unwrap_or(Value::Missing),
            })
        }
        Expr::NormDiscrete {
            field,
            value,
            map_missing_to,
        } => {
            let val = lookup(field, env, params)?;
            if val == Value::Missing {
                return Ok(map_missing_to.map_or(Value::Missing, Value::Number));
            }
            let matched = value_matches_str(&val, value);
            Ok(Value::Number(if matched { 1.0 } else { 0.0 }))
        }
        Expr::Apply {
            function,
            args,
            map_missing_to,
        } => {
            let raw = eval_apply(function, args, env, functions, params, depth)?;
            if raw == Value::Missing {
                Ok(map_missing_to.clone().unwrap_or(Value::Missing))
            } else {
                Ok(raw)
            }
        }
    }
}

fn lookup(field: &str, env: &Env<'_>, params: &[(&str, Value)]) -> Result<Value, PmmlError> {
    if let Some((_, val)) = params.iter().find(|(name, _)| *name == field) {
        return Ok(val.clone());
    }
    env.get(field)
        .cloned()
        .ok_or_else(|| PmmlError::UnknownField(field.to_string()))
}

pub(crate) fn as_f64(val: &Value, context: &str) -> Result<f64, PmmlError> {
    match val {
        Value::Number(n) => {
            if n.is_finite() {
                Ok(*n)
            } else {
                Err(PmmlError::Score(format!("non-finite number in {context}")))
            }
        }
        Value::Text(s) => {
            let n = s.trim().parse::<f64>().map_err(|_| {
                PmmlError::Score(format!("expected number for {context}, got {s:?}"))
            })?;
            if n.is_finite() {
                Ok(n)
            } else {
                Err(PmmlError::Score(format!("non-finite number in {context}")))
            }
        }
        Value::Missing => Err(PmmlError::Score(format!("unexpected missing in {context}"))),
    }
}

pub(crate) fn value_matches_str(val: &Value, target: &str) -> bool {
    match val {
        Value::Missing => false,
        Value::Text(s) => {
            if s == target {
                return true;
            }
            if let (Ok(a), Ok(b)) = (s.trim().parse::<f64>(), target.trim().parse::<f64>()) {
                return a == b;
            }
            false
        }
        Value::Number(n) => target.trim().parse::<f64>().is_ok_and(|t| *n == t),
    }
}

pub(crate) fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Missing, Value::Missing) => true,
        (Value::Missing, _) | (_, Value::Missing) => false,
        (Value::Number(x), Value::Number(y)) => x == y,
        (Value::Text(x), Value::Text(y)) => {
            if x == y {
                return true;
            }
            if let (Ok(nx), Ok(ny)) = (x.trim().parse::<f64>(), y.trim().parse::<f64>()) {
                return nx == ny;
            }
            false
        }
        (Value::Number(n), Value::Text(s)) | (Value::Text(s), Value::Number(n)) => {
            s.trim().parse::<f64>().is_ok_and(|parsed| parsed == *n)
        }
    }
}

fn eval_norm_continuous(
    x: f64,
    points: &[(f64, f64)],
    outliers: Outliers,
    map_missing_to: Option<f64>,
) -> Result<Value, PmmlError> {
    let (Some(&(first_x, first_y)), Some(&(last_x, last_y))) = (points.first(), points.last())
    else {
        return Err(PmmlError::Score(
            "NormContinuous has fewer than two points".into(),
        ));
    };

    if x < first_x {
        return match outliers {
            Outliers::AsMissingValues => Ok(map_missing_to.map_or(Value::Missing, Value::Number)),
            Outliers::AsExtremeValues => Ok(Value::Number(first_y)),
            Outliers::AsIs => {
                let &(x1, y1) = points.get(1).ok_or_else(|| {
                    PmmlError::Score("NormContinuous has fewer than two points".into())
                })?;
                Ok(Value::Number(interpolate(x, first_x, first_y, x1, y1)))
            }
        };
    }

    if x > last_x {
        return match outliers {
            Outliers::AsMissingValues => Ok(map_missing_to.map_or(Value::Missing, Value::Number)),
            Outliers::AsExtremeValues => Ok(Value::Number(last_y)),
            Outliers::AsIs => {
                let prev_idx = points.len().saturating_sub(2);
                let &(x0, y0) = points.get(prev_idx).ok_or_else(|| {
                    PmmlError::Score("NormContinuous has fewer than two points".into())
                })?;
                Ok(Value::Number(interpolate(x, x0, y0, last_x, last_y)))
            }
        };
    }

    for window in points.windows(2) {
        if let [(x0, y0), (x1, y1)] = *window
            && x >= x0
            && x <= x1
        {
            return Ok(Value::Number(interpolate(x, x0, y0, x1, y1)));
        }
    }

    Ok(Value::Number(last_y))
}

fn interpolate(x: f64, x0: f64, y0: f64, x1: f64, y1: f64) -> f64 {
    let dx = x1 - x0;
    if dx.abs() == 0.0 {
        y0
    } else {
        y0 + (x - x0) / dx * (y1 - y0)
    }
}

fn as_bool(val: &Value) -> Result<Option<bool>, PmmlError> {
    match val {
        Value::Missing => Ok(None),
        Value::Text(s) => match s.trim() {
            "true" | "TRUE" | "True" | "1" => Ok(Some(true)),
            "false" | "FALSE" | "False" | "0" => Ok(Some(false)),
            other => Err(PmmlError::Score(format!("expected boolean, got {other:?}"))),
        },
        Value::Number(n) => Ok(Some(*n != 0.0)),
    }
}

fn bool_val(b: bool) -> Value {
    Value::Text(if b { "true".into() } else { "false".into() })
}

fn eval_apply(
    function: &str,
    args: &[Expr],
    env: &Env<'_>,
    functions: &[DefineFunction],
    params: &[(&str, Value)],
    depth: usize,
) -> Result<Value, PmmlError> {
    // User-defined function (`DefineFunction`) takes priority if defined.
    if let Some(def) = functions.iter().find(|f| f.name == function) {
        if args.len() != def.parameters.len() {
            return Err(PmmlError::Score(format!(
                "function {function:?} expects {} arguments, got {}",
                def.parameters.len(),
                args.len()
            )));
        }
        let mut evaluated_args = Vec::with_capacity(args.len());
        for arg in args {
            evaluated_args.push(eval(arg, env, functions, params, depth + 1)?);
        }
        let local_params: Vec<(&str, Value)> = def
            .parameters
            .iter()
            .map(String::as_str)
            .zip(evaluated_args)
            .collect();
        return eval(&def.body, env, functions, &local_params, depth + 1);
    }

    match function {
        "if" => {
            if args.len() != 2 && args.len() != 3 {
                return Err(PmmlError::Score(
                    "built-in 'if' expects 2 or 3 arguments".into(),
                ));
            }
            let cond_expr = args
                .first()
                .ok_or_else(|| PmmlError::Score("missing condition in 'if'".into()))?;
            let cond_val = eval(cond_expr, env, functions, params, depth + 1)?;
            match as_bool(&cond_val)? {
                Some(true) => {
                    let then_expr = args
                        .get(1)
                        .ok_or_else(|| PmmlError::Score("missing then branch in 'if'".into()))?;
                    eval(then_expr, env, functions, params, depth + 1)
                }
                Some(false) => {
                    if let Some(else_expr) = args.get(2) {
                        eval(else_expr, env, functions, params, depth + 1)
                    } else {
                        Ok(Value::Missing)
                    }
                }
                None => Ok(Value::Missing),
            }
        }
        "and" => {
            if args.len() < 2 {
                return Err(PmmlError::Score(
                    "built-in 'and' requires at least 2 arguments".into(),
                ));
            }
            let mut saw_missing = false;
            for arg in args {
                let v = eval(arg, env, functions, params, depth + 1)?;
                match as_bool(&v)? {
                    Some(false) => return Ok(bool_val(false)),
                    Some(true) => {}
                    None => saw_missing = true,
                }
            }
            if saw_missing {
                Ok(Value::Missing)
            } else {
                Ok(bool_val(true))
            }
        }
        "or" => {
            if args.len() < 2 {
                return Err(PmmlError::Score(
                    "built-in 'or' requires at least 2 arguments".into(),
                ));
            }
            let mut saw_missing = false;
            for arg in args {
                let v = eval(arg, env, functions, params, depth + 1)?;
                match as_bool(&v)? {
                    Some(true) => return Ok(bool_val(true)),
                    Some(false) => {}
                    None => saw_missing = true,
                }
            }
            if saw_missing {
                Ok(Value::Missing)
            } else {
                Ok(bool_val(false))
            }
        }
        "not" => {
            let [arg] = args else {
                return Err(PmmlError::Score("built-in 'not' expects 1 argument".into()));
            };
            let v = eval(arg, env, functions, params, depth + 1)?;
            match as_bool(&v)? {
                Some(b) => Ok(bool_val(!b)),
                None => Ok(Value::Missing),
            }
        }
        "isMissing" => {
            let [arg] = args else {
                return Err(PmmlError::Score(
                    "built-in 'isMissing' expects 1 argument".into(),
                ));
            };
            let v = eval(arg, env, functions, params, depth + 1)?;
            Ok(bool_val(v == Value::Missing))
        }
        "isNotMissing" => {
            let [arg] = args else {
                return Err(PmmlError::Score(
                    "built-in 'isNotMissing' expects 1 argument".into(),
                ));
            };
            let v = eval(arg, env, functions, params, depth + 1)?;
            Ok(bool_val(v != Value::Missing))
        }
        "equal" | "notEqual" => {
            let [lhs_expr, rhs_expr] = args else {
                return Err(PmmlError::Score(format!(
                    "built-in {function:?} expects 2 arguments"
                )));
            };
            let lhs = eval(lhs_expr, env, functions, params, depth + 1)?;
            let rhs = eval(rhs_expr, env, functions, params, depth + 1)?;
            if lhs == Value::Missing || rhs == Value::Missing {
                return Ok(Value::Missing);
            }
            let eq = values_equal(&lhs, &rhs);
            Ok(bool_val(if function == "equal" { eq } else { !eq }))
        }
        "lessThan" | "lessOrEqual" | "greaterThan" | "greaterOrEqual" => {
            let [lhs_expr, rhs_expr] = args else {
                return Err(PmmlError::Score(format!(
                    "built-in {function:?} expects 2 arguments"
                )));
            };
            let lhs = eval(lhs_expr, env, functions, params, depth + 1)?;
            let rhs = eval(rhs_expr, env, functions, params, depth + 1)?;
            if lhs == Value::Missing || rhs == Value::Missing {
                return Ok(Value::Missing);
            }
            let a = as_f64(&lhs, function)?;
            let b = as_f64(&rhs, function)?;
            let res = match function {
                "lessThan" => a < b,
                "lessOrEqual" => a <= b,
                "greaterThan" => a > b,
                _ => a >= b,
            };
            Ok(bool_val(res))
        }
        "min" | "max" | "sum" | "avg" | "product" => {
            if args.is_empty() {
                return Err(PmmlError::Score(format!(
                    "built-in {function:?} requires at least 1 argument"
                )));
            }
            let mut nums = Vec::with_capacity(args.len());
            for arg in args {
                let v = eval(arg, env, functions, params, depth + 1)?;
                if v != Value::Missing {
                    nums.push(as_f64(&v, function)?);
                }
            }
            let Some(&first) = nums.first() else {
                return Ok(Value::Missing);
            };
            let out = match function {
                "min" => nums.iter().copied().fold(first, f64::min),
                "max" => nums.iter().copied().fold(first, f64::max),
                "sum" => nums.iter().sum::<f64>(),
                "avg" => nums.iter().sum::<f64>() / (nums.len() as f64),
                _ => nums.iter().product::<f64>(),
            };
            if out.is_finite() {
                Ok(Value::Number(out))
            } else {
                Ok(Value::Missing)
            }
        }
        "+" => {
            if args.is_empty() {
                return Err(PmmlError::Score(
                    "built-in '+' requires at least 1 argument".into(),
                ));
            }
            let mut sum = 0.0;
            for arg in args {
                let v = eval(arg, env, functions, params, depth + 1)?;
                if v == Value::Missing {
                    return Ok(Value::Missing);
                }
                sum += as_f64(&v, "+")?;
            }
            if sum.is_finite() {
                Ok(Value::Number(sum))
            } else {
                Ok(Value::Missing)
            }
        }
        "-" => match args {
            [only] => {
                let v = eval(only, env, functions, params, depth + 1)?;
                if v == Value::Missing {
                    Ok(Value::Missing)
                } else {
                    Ok(Value::Number(-as_f64(&v, "-")?))
                }
            }
            [lhs_expr, rhs_expr] => {
                let lhs = eval(lhs_expr, env, functions, params, depth + 1)?;
                let rhs = eval(rhs_expr, env, functions, params, depth + 1)?;
                if lhs == Value::Missing || rhs == Value::Missing {
                    return Ok(Value::Missing);
                }
                let out = as_f64(&lhs, "-")? - as_f64(&rhs, "-")?;
                if out.is_finite() {
                    Ok(Value::Number(out))
                } else {
                    Ok(Value::Missing)
                }
            }
            _ => Err(PmmlError::Score(
                "built-in '-' expects 1 or 2 arguments".into(),
            )),
        },
        "*" => {
            if args.len() < 2 {
                return Err(PmmlError::Score(
                    "built-in '*' requires at least 2 arguments".into(),
                ));
            }
            let mut prod = 1.0;
            for arg in args {
                let v = eval(arg, env, functions, params, depth + 1)?;
                if v == Value::Missing {
                    return Ok(Value::Missing);
                }
                prod *= as_f64(&v, "*")?;
            }
            if prod.is_finite() {
                Ok(Value::Number(prod))
            } else {
                Ok(Value::Missing)
            }
        }
        "/" => {
            let [lhs_expr, rhs_expr] = args else {
                return Err(PmmlError::Score("built-in '/' expects 2 arguments".into()));
            };
            let lhs = eval(lhs_expr, env, functions, params, depth + 1)?;
            let rhs = eval(rhs_expr, env, functions, params, depth + 1)?;
            if lhs == Value::Missing || rhs == Value::Missing {
                return Ok(Value::Missing);
            }
            let num = as_f64(&lhs, "/")?;
            let den = as_f64(&rhs, "/")?;
            if den == 0.0 {
                return Ok(Value::Missing);
            }
            let out = num / den;
            if out.is_finite() {
                Ok(Value::Number(out))
            } else {
                Ok(Value::Missing)
            }
        }
        "sqrt" | "ln" | "log10" | "exp" | "abs" | "floor" | "ceil" | "round" => {
            let [arg] = args else {
                return Err(PmmlError::Score(format!(
                    "built-in {function:?} expects 1 argument"
                )));
            };
            let v = eval(arg, env, functions, params, depth + 1)?;
            if v == Value::Missing {
                return Ok(Value::Missing);
            }
            let x = as_f64(&v, function)?;
            let out = match function {
                "sqrt" => {
                    if x < 0.0 {
                        return Ok(Value::Missing);
                    }
                    x.sqrt()
                }
                "ln" => {
                    if x <= 0.0 {
                        return Ok(Value::Missing);
                    }
                    x.ln()
                }
                "log10" => {
                    if x <= 0.0 {
                        return Ok(Value::Missing);
                    }
                    x.log10()
                }
                "exp" => x.exp(),
                "abs" => x.abs(),
                "floor" => x.floor(),
                "ceil" => x.ceil(),
                _ => x.round(),
            };
            if out.is_finite() {
                Ok(Value::Number(out))
            } else {
                Ok(Value::Missing)
            }
        }
        "pow" | "threshold" => {
            let [lhs_expr, rhs_expr] = args else {
                return Err(PmmlError::Score(format!(
                    "built-in {function:?} expects 2 arguments"
                )));
            };
            let lhs = eval(lhs_expr, env, functions, params, depth + 1)?;
            let rhs = eval(rhs_expr, env, functions, params, depth + 1)?;
            if lhs == Value::Missing || rhs == Value::Missing {
                return Ok(Value::Missing);
            }
            let a = as_f64(&lhs, function)?;
            let b = as_f64(&rhs, function)?;
            let out = if function == "pow" {
                a.powf(b)
            } else if a > b {
                1.0
            } else {
                0.0
            };
            if out.is_finite() {
                Ok(Value::Number(out))
            } else {
                Ok(Value::Missing)
            }
        }
        "uppercase" | "lowercase" | "trimBlanks" => {
            let [arg] = args else {
                return Err(PmmlError::Score(format!(
                    "built-in {function:?} expects 1 argument"
                )));
            };
            let v = eval(arg, env, functions, params, depth + 1)?;
            match v {
                Value::Missing => Ok(Value::Missing),
                Value::Text(s) => Ok(Value::Text(match function {
                    "uppercase" => s.to_uppercase(),
                    "lowercase" => s.to_lowercase(),
                    _ => s.trim().to_string(),
                })),
                Value::Number(n) => Ok(Value::Text(n.to_string())),
            }
        }
        other => Err(PmmlError::Unsupported(format!(
            "built-in function {other:?}"
        ))),
    }
}
