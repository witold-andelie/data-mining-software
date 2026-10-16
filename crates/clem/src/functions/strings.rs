//! String, character (`CHAR`), Unicode and SoundEx CLEM functions.

use super::{Args, Function, Returns, char_arg, f, first, integer, kind, number, string, string1};
use crate::types::Ty;
use crate::{ClemError, Datum};
use dms_types::Value;

/// Upper bound on `replicate` output length in bytes so hostile inputs cannot exhaust memory.
const MAX_REPLICATE_BYTES: usize = 1 << 20;

pub(super) const FUNCTIONS: &[Function] = &[
    f("to_string", 1, |a| {
        let value = first(a)?;
        Ok(if value.is_null() {
            Datum::NULL
        } else {
            Datum::Value(Value::string(value.to_text()?))
        })
    })
    .returns(Returns::Fixed(Ty::String)),
    f("is_string", 1, |a| {
        kind(a, |v| matches!(v, Value::String(_)))
    })
    .returns(Returns::Fixed(Ty::Bool))
    .type_test(|t| matches!(t, Ty::String)),
    f("length", 1, |a| {
        string1(a, |s| {
            Value::Integer(i64::try_from(s.chars().count()).unwrap_or(i64::MAX))
        })
    })
    .returns(Returns::Fixed(Ty::Integer)),
    f("lowertoupper", 1, |a| {
        string1(a, |s| Value::string(s.to_uppercase()))
    })
    .returns(Returns::Fixed(Ty::String)),
    f("uppertolower", 1, |a| {
        string1(a, |s| Value::string(s.to_lowercase()))
    })
    .returns(Returns::Fixed(Ty::String)),
    f("trim", 1, |a| string1(a, |s| Value::string(s.trim()))).returns(Returns::Fixed(Ty::String)),
    // Oracle rule 45: `trimstart`; the guide's `trim_start` does not exist.
    f("trimstart", 1, |a| {
        string1(a, |s| Value::string(s.trim_start()))
    })
    .returns(Returns::Fixed(Ty::String)),
    f("trimend", 1, |a| {
        string1(a, |s| Value::string(s.trim_end()))
    })
    .returns(Returns::Fixed(Ty::String)),
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
    })
    .returns(Returns::Fixed(Ty::String)),
    // Oracle rule 42: 1-based inclusive on both ends; null if N1 < 1, N1 > N2, or N2 > length.
    f("substring_between", 3, |a| {
        let (Some(n1), Some(n2), Some(text)) = (a.first(), a.get(1), a.get(2)) else {
            return Err(ClemError::Type(
                "substring_between needs three arguments".into(),
            ));
        };
        if n1.is_null() || n2.is_null() || text.is_null() {
            return Ok(Datum::NULL);
        }
        let (n1, n2) = (integer(n1)?, integer(n2)?);
        let text = string(text)?;
        let count = i64::try_from(text.chars().count()).unwrap_or(i64::MAX);
        if n1 < 1 || n1 > n2 || n2 > count {
            return Ok(Datum::NULL);
        }
        let skip = usize::try_from(n1 - 1).unwrap_or(usize::MAX);
        let take = usize::try_from(n2 - n1 + 1).unwrap_or(usize::MAX);
        Ok(Datum::Value(Value::string(
            text.chars().skip(skip).take(take).collect::<String>(),
        )))
    })
    .returns(Returns::Fixed(Ty::String)),
    // Oracle rule 43: 1-based character at N; null if out of 1..=length(s).
    f("subscrs", 2, |a| {
        let (Some(n), Some(text)) = (a.first(), a.get(1)) else {
            return Err(ClemError::Type("subscrs needs two arguments".into()));
        };
        if n.is_null() || text.is_null() {
            return Ok(Datum::NULL);
        }
        let n = integer(n)?;
        if n < 1 {
            return Ok(Datum::NULL);
        }
        let text = string(text)?;
        let idx = usize::try_from(n - 1).ok();
        Ok(match idx.and_then(|i| text.chars().nth(i)) {
            Some(ch) => Datum::Value(Value::string(ch.to_string())),
            None => Datum::NULL,
        })
    })
    .returns(Returns::Fixed(Ty::String)),
    // Oracle rule 47: last character of STRING, or null when empty.
    f("last", 1, |a| {
        let value = first(a)?;
        if value.is_null() {
            return Ok(Datum::NULL);
        }
        let text = string(&value)?;
        Ok(match text.chars().next_back() {
            Some(ch) => Datum::Value(Value::string(ch.to_string())),
            None => Datum::NULL,
        })
    })
    .returns(Returns::Fixed(Ty::String)),
    // Oracle rule 44: 1-based index of CHAR from N forward/backward; 0 if absent; null when N
    // is outside 1..=length(s) (so `locchar(c, 1, "")` is null).
    f("locchar", 3, |a| locate_char(a, false, true))
        .returns(Returns::Fixed(Ty::Integer))
        .args(Args::CharFirst),
    f("locchar_back", 3, |a| locate_char(a, true, true))
        .returns(Returns::Fixed(Ty::Integer))
        .args(Args::CharFirst),
    f("skipchar", 3, |a| locate_char(a, false, false))
        .returns(Returns::Fixed(Ty::Integer))
        .args(Args::CharFirst),
    f("skipchar_back", 3, |a| locate_char(a, true, false))
        .returns(Returns::Fixed(Ty::Integer))
        .args(Args::CharFirst),
    f("stripchar", 2, |a| {
        let (Some(ch), Some(text)) = (a.first(), a.get(1)) else {
            return Err(ClemError::Type("stripchar needs two arguments".into()));
        };
        if ch.is_null() || text.is_null() {
            return Ok(Datum::NULL);
        }
        let ch = char_arg(ch)?;
        let text = string(text)?;
        Ok(Datum::Value(Value::string(
            text.chars().filter(|&c| c != ch).collect::<String>(),
        )))
    })
    .returns(Returns::Fixed(Ty::String))
    .args(Args::CharFirst),
    // Oracle rule 44: unlike `locchar(c, 1, s)`, `strmember(c, "")` returns 0 rather than null.
    f("strmember", 2, |a| {
        let (Some(ch), Some(text)) = (a.first(), a.get(1)) else {
            return Err(ClemError::Type("strmember needs two arguments".into()));
        };
        if ch.is_null() || text.is_null() {
            return Ok(Datum::NULL);
        }
        let ch = char_arg(ch)?;
        let text = string(text)?;
        let pos = text
            .chars()
            .position(|c| c == ch)
            .map_or(0, |i| i64::try_from(i).unwrap_or(0).saturating_add(1));
        Ok(Datum::Value(Value::Integer(pos)))
    })
    .returns(Returns::Fixed(Ty::Integer))
    .args(Args::CharFirst),
    // Oracle rule 45: clamps when LENGTH > length(s); null when LENGTH < 0.
    f("startstring", 2, |a| start_end_string(a, true)).returns(Returns::Fixed(Ty::String)),
    f("endstring", 2, |a| start_end_string(a, false)).returns(Returns::Fixed(Ty::String)),
    // Oracle rule 45: returns "" when N == length(s) and null when N < 0 or N > length(s).
    f("allbutfirst", 2, |a| all_but(a, true)).returns(Returns::Fixed(Ty::String)),
    f("allbutlast", 2, |a| all_but(a, false)).returns(Returns::Fixed(Ty::String)),
    // Oracle rule 46: exact code-point (ASCII) order.
    f("alphabefore", 2, |a| {
        let (Some(left), Some(right)) = (a.first(), a.get(1)) else {
            return Err(ClemError::Type("alphabefore needs two arguments".into()));
        };
        if left.is_null() || right.is_null() {
            return Ok(Datum::NULL);
        }
        Ok(Datum::Bool(string(left)? < string(right)?))
    })
    .returns(Returns::Fixed(Ty::Bool)),
    // Oracle rule 47: `replace("", new, s)` leaves `s` unchanged.
    f("replace", 3, |a| {
        let (Some(sub), Some(new_sub), Some(text)) = (a.first(), a.get(1), a.get(2)) else {
            return Err(ClemError::Type("replace needs three arguments".into()));
        };
        if sub.is_null() || new_sub.is_null() || text.is_null() {
            return Ok(Datum::NULL);
        }
        let (sub, new_sub, text) = (string(sub)?, string(new_sub)?, string(text)?);
        let result = if sub.is_empty() {
            text
        } else {
            text.replace(&sub, &new_sub)
        };
        Ok(Datum::Value(Value::string(result)))
    })
    .returns(Returns::Fixed(Ty::String)),
    // Oracle rule 47: `replicate(0, s)` -> `""`; `replicate(-1, s)` -> null.
    f("replicate", 2, |a| {
        let (Some(count), Some(text)) = (a.first(), a.get(1)) else {
            return Err(ClemError::Type("replicate needs two arguments".into()));
        };
        if count.is_null() || text.is_null() {
            return Ok(Datum::NULL);
        }
        let count = integer(count)?;
        if count < 0 {
            return Ok(Datum::NULL);
        }
        let text = string(text)?;
        let count = usize::try_from(count).unwrap_or(usize::MAX);
        if text.len().saturating_mul(count) > MAX_REPLICATE_BYTES {
            return Err(ClemError::Type("replicated string too long".into()));
        }
        Ok(Datum::Value(Value::string(text.repeat(count))))
    })
    .returns(Returns::Fixed(Ty::String)),
    // Oracle rule 47: counts overlapping occurrences (`count_substring("aaaa", "aa")` is 3).
    f("count_substring", 2, |a| {
        let (Some(text), Some(sub)) = (a.first(), a.get(1)) else {
            return Err(ClemError::Type(
                "count_substring needs two arguments".into(),
            ));
        };
        if text.is_null() || sub.is_null() {
            return Ok(Datum::NULL);
        }
        let text: Vec<char> = string(text)?.chars().collect();
        let sub: Vec<char> = string(sub)?.chars().collect();
        // GAP: empty substring returns 0.
        if sub.is_empty() || sub.len() > text.len() {
            return Ok(Datum::Value(Value::Integer(0)));
        }
        let count = text
            .windows(sub.len())
            .filter(|window| *window == sub.as_slice())
            .count();
        Ok(Datum::Value(Value::Integer(
            i64::try_from(count).unwrap_or(i64::MAX),
        )))
    })
    .returns(Returns::Fixed(Ty::Integer)),
    // Oracle rule 47: 1-based start subscript or 0 (integer, not boolean); N defaults to 1.
    f("issubstring", 3, |a| is_has_substring(a, false))
        .returns(Returns::Fixed(Ty::Integer))
        .arities(&[2, 3]),
    f("hassubstring", 3, |a| is_has_substring(a, true))
        .returns(Returns::Fixed(Ty::Integer))
        .arities(&[2, 3]),
    f("issubstring_count", 3, issubstring_count).returns(Returns::Fixed(Ty::Integer)),
    f("issubstring_lim", 5, issubstring_lim).returns(Returns::Fixed(Ty::Integer)),
    f("isstartstring", 2, |a| start_or_end_match(a, true, false))
        .returns(Returns::Fixed(Ty::Integer)),
    f("hasstartstring", 2, |a| start_or_end_match(a, true, true))
        .returns(Returns::Fixed(Ty::Integer)),
    f("isendstring", 2, |a| start_or_end_match(a, false, false))
        .returns(Returns::Fixed(Ty::Integer)),
    f("hasendstring", 2, |a| start_or_end_match(a, false, true))
        .returns(Returns::Fixed(Ty::Integer)),
    f("ismidstring", 2, |a| mid_match(a, false)).returns(Returns::Fixed(Ty::Integer)),
    f("hasmidstring", 2, |a| mid_match(a, true)).returns(Returns::Fixed(Ty::Integer)),
    f("isalphacode", 1, |a| char_pred(a, char::is_alphabetic))
        .returns(Returns::Fixed(Ty::Bool))
        .args(Args::CharFirst),
    f("islowercode", 1, |a| char_pred(a, char::is_lowercase))
        .returns(Returns::Fixed(Ty::Bool))
        .args(Args::CharFirst),
    f("isuppercode", 1, |a| char_pred(a, char::is_uppercase))
        .returns(Returns::Fixed(Ty::Bool))
        .args(Args::CharFirst),
    f("isnumbercode", 1, |a| char_pred(a, |c| c.is_ascii_digit()))
        .returns(Returns::Fixed(Ty::Bool))
        .args(Args::CharFirst),
    f("unicode_value", 1, |a| {
        let value = first(a)?;
        if value.is_null() {
            return Ok(Datum::NULL);
        }
        let ch = char_arg(&value)?;
        Ok(Datum::Value(Value::Integer(i64::from(u32::from(ch)))))
    })
    .returns(Returns::Fixed(Ty::Integer))
    .args(Args::CharFirst),
    f("unicode_char", 1, |a| {
        let value = first(a)?;
        if value.is_null() {
            return Ok(Datum::NULL);
        }
        let code = number(&value)?.trunc();
        let ch = if (1.0..=f64::from(u32::MAX)).contains(&code) {
            char::from_u32(code as u32)
        } else {
            None
        };
        Ok(match ch {
            Some(c) => Datum::Value(Value::string(c.to_string())),
            None => Datum::NULL,
        })
    })
    .returns(Returns::Fixed(Ty::String)),
    // Oracle rule 48: `soundex` returns a 4-character String ("S530"); `soundex_difference`
    // returns an Integer in 0..=4.
    f("soundex", 1, |a| {
        let value = first(a)?;
        if value.is_null() {
            return Ok(Datum::NULL);
        }
        Ok(Datum::Value(Value::string(soundex_code(&string(&value)?))))
    })
    .returns(Returns::Fixed(Ty::String)),
    f("soundex_difference", 2, |a| {
        let (Some(s1), Some(s2)) = (a.first(), a.get(1)) else {
            return Err(ClemError::Type(
                "soundex_difference needs two arguments".into(),
            ));
        };
        if s1.is_null() || s2.is_null() {
            return Ok(Datum::NULL);
        }
        let (c1, c2) = (soundex_code(&string(s1)?), soundex_code(&string(s2)?));
        let matching = c1.chars().zip(c2.chars()).filter(|(a, b)| a == b).count();
        Ok(Datum::Value(Value::Integer(
            i64::try_from(matching).unwrap_or(0),
        )))
    })
    .returns(Returns::Fixed(Ty::Integer)),
];

fn locate_char(args: &[Datum], backward: bool, want_equal: bool) -> Result<Datum, ClemError> {
    let (Some(ch), Some(n), Some(text)) = (args.first(), args.get(1), args.get(2)) else {
        return Err(ClemError::Type(
            "character search needs three arguments".into(),
        ));
    };
    if ch.is_null() || n.is_null() || text.is_null() {
        return Ok(Datum::NULL);
    }
    let ch = char_arg(ch)?;
    let n = integer(n)?;
    let chars: Vec<char> = string(text)?.chars().collect();
    let len = i64::try_from(chars.len()).unwrap_or(i64::MAX);
    if n < 1 || n > len {
        return Ok(Datum::NULL);
    }
    let start = usize::try_from(n - 1).unwrap_or(0);
    let matches_char = |c: char| (c == ch) == want_equal;
    let found = if backward {
        (0..=start)
            .rev()
            .find(|&i| chars.get(i).is_some_and(|&c| matches_char(c)))
    } else {
        (start..chars.len()).find(|&i| chars.get(i).is_some_and(|&c| matches_char(c)))
    };
    let pos = found.map_or(0, |i| i64::try_from(i).unwrap_or(0).saturating_add(1));
    Ok(Datum::Value(Value::Integer(pos)))
}

fn start_end_string(args: &[Datum], from_start: bool) -> Result<Datum, ClemError> {
    let (Some(length), Some(text)) = (args.first(), args.get(1)) else {
        return Err(ClemError::Type("needs two arguments".into()));
    };
    if length.is_null() || text.is_null() {
        return Ok(Datum::NULL);
    }
    let length = integer(length)?;
    if length < 0 {
        return Ok(Datum::NULL);
    }
    let chars: Vec<char> = string(text)?.chars().collect();
    let take = usize::try_from(length)
        .unwrap_or(usize::MAX)
        .min(chars.len());
    let slice = if from_start {
        chars.get(..take).unwrap_or_default()
    } else {
        chars.get(chars.len() - take..).unwrap_or_default()
    };
    Ok(Datum::Value(Value::string(
        slice.iter().collect::<String>(),
    )))
}

fn all_but(args: &[Datum], from_start: bool) -> Result<Datum, ClemError> {
    let (Some(n), Some(text)) = (args.first(), args.get(1)) else {
        return Err(ClemError::Type("needs two arguments".into()));
    };
    if n.is_null() || text.is_null() {
        return Ok(Datum::NULL);
    }
    let n = integer(n)?;
    let chars: Vec<char> = string(text)?.chars().collect();
    let len = i64::try_from(chars.len()).unwrap_or(i64::MAX);
    if n < 0 || n > len {
        return Ok(Datum::NULL);
    }
    let drop = usize::try_from(n).unwrap_or(0);
    let slice = if from_start {
        chars.get(drop..).unwrap_or_default()
    } else {
        chars.get(..chars.len() - drop).unwrap_or_default()
    };
    Ok(Datum::Value(Value::string(
        slice.iter().collect::<String>(),
    )))
}

fn is_has_substring(args: &[Datum], has_order: bool) -> Result<Datum, ClemError> {
    // The start position N is optional and defaults to 1.
    let one = Datum::Value(Value::Integer(1));
    let (first_arg, n, third_arg) = match args {
        [first_arg, third_arg] => (first_arg, &one, third_arg),
        [first_arg, n, third_arg] => (first_arg, n, third_arg),
        _ => return Err(ClemError::Type("needs two or three arguments".into())),
    };
    if first_arg.is_null() || n.is_null() || third_arg.is_null() {
        return Ok(Datum::NULL);
    }
    let (sub, text) = if has_order {
        (string(third_arg)?, string(first_arg)?)
    } else {
        (string(first_arg)?, string(third_arg)?)
    };
    let n = integer(n)?;
    let text_chars: Vec<char> = text.chars().collect();
    let sub_chars: Vec<char> = sub.chars().collect();
    let len = i64::try_from(text_chars.len()).unwrap_or(i64::MAX);
    if n < 1 {
        return Ok(Datum::NULL);
    }
    // Oracle: `issubstring(sub, "")` (where N defaults to 1) returns 0 on empty strings, while
    // an explicit N > length(s) (e.g. `issubstring("o", 6, s)`) returns null.
    if n > len {
        return if n == 1 && len == 0 {
            Ok(Datum::Value(Value::Integer(0)))
        } else {
            Ok(Datum::NULL)
        };
    }
    if sub_chars.is_empty() || sub_chars.len() > text_chars.len() {
        return Ok(Datum::Value(Value::Integer(0)));
    }
    let start = usize::try_from(n - 1).unwrap_or(0);
    let found = (start..=text_chars.len() - sub_chars.len()).find(|&i| {
        text_chars
            .get(i..i + sub_chars.len())
            .is_some_and(|w| w == sub_chars.as_slice())
    });
    let pos = found.map_or(0, |i| i64::try_from(i).unwrap_or(0).saturating_add(1));
    Ok(Datum::Value(Value::Integer(pos)))
}

fn issubstring_count(args: &[Datum]) -> Result<Datum, ClemError> {
    let (Some(sub), Some(nth), Some(text)) = (args.first(), args.get(1), args.get(2)) else {
        return Err(ClemError::Type(
            "issubstring_count needs three arguments".into(),
        ));
    };
    if sub.is_null() || nth.is_null() || text.is_null() {
        return Ok(Datum::NULL);
    }
    let nth = integer(nth)?;
    if nth < 1 {
        return Ok(Datum::NULL);
    }
    let text_chars: Vec<char> = string(text)?.chars().collect();
    let sub_chars: Vec<char> = string(sub)?.chars().collect();
    if sub_chars.is_empty() || sub_chars.len() > text_chars.len() {
        return Ok(Datum::Value(Value::Integer(0)));
    }
    let mut seen: i64 = 0;
    for i in 0..=text_chars.len() - sub_chars.len() {
        if text_chars
            .get(i..i + sub_chars.len())
            .is_some_and(|w| w == sub_chars.as_slice())
        {
            seen = seen.saturating_add(1);
            if seen == nth {
                return Ok(Datum::Value(Value::Integer(
                    i64::try_from(i).unwrap_or(0).saturating_add(1),
                )));
            }
        }
    }
    Ok(Datum::Value(Value::Integer(0)))
}

fn issubstring_lim(args: &[Datum]) -> Result<Datum, ClemError> {
    let (Some(sub), Some(n), Some(start_lim), Some(end_lim), Some(text)) = (
        args.first(),
        args.get(1),
        args.get(2),
        args.get(3),
        args.get(4),
    ) else {
        return Err(ClemError::Type(
            "issubstring_lim needs five arguments".into(),
        ));
    };
    if sub.is_null() || n.is_null() || start_lim.is_null() || end_lim.is_null() || text.is_null() {
        return Ok(Datum::NULL);
    }
    let text_chars: Vec<char> = string(text)?.chars().collect();
    let sub_chars: Vec<char> = string(sub)?.chars().collect();
    let len = i64::try_from(text_chars.len()).unwrap_or(i64::MAX);
    let n = integer(n)?;
    if n < 1 || n > len {
        return Ok(Datum::NULL);
    }
    let parse_lim = |d: &Datum| -> Result<Option<i64>, ClemError> {
        match d {
            Datum::Bool(false) => Ok(None),
            _ => Ok(Some(integer(d)?)),
        }
    };
    let max_start = match parse_lim(start_lim)? {
        Some(lim) if lim < 1 || lim > len => return Ok(Datum::NULL),
        Some(lim) => lim,
        None => len,
    };
    let max_end = match parse_lim(end_lim)? {
        Some(lim) if lim < 1 || lim > len => return Ok(Datum::NULL),
        Some(lim) => lim,
        None => len,
    };
    if sub_chars.is_empty() || sub_chars.len() > text_chars.len() {
        return Ok(Datum::Value(Value::Integer(0)));
    }
    let start = usize::try_from(n - 1).unwrap_or(0);
    let sub_len = i64::try_from(sub_chars.len()).unwrap_or(i64::MAX);
    for i in start..=text_chars.len() - sub_chars.len() {
        let pos1 = i64::try_from(i).unwrap_or(0).saturating_add(1);
        let end1 = pos1.saturating_add(sub_len - 1);
        if pos1 > max_start || end1 > max_end {
            break;
        }
        if text_chars
            .get(i..i + sub_chars.len())
            .is_some_and(|w| w == sub_chars.as_slice())
        {
            return Ok(Datum::Value(Value::Integer(pos1)));
        }
    }
    Ok(Datum::Value(Value::Integer(0)))
}

fn start_or_end_match(args: &[Datum], at_start: bool, has_order: bool) -> Result<Datum, ClemError> {
    let (Some(a0), Some(a1)) = (args.first(), args.get(1)) else {
        return Err(ClemError::Type("needs two arguments".into()));
    };
    if a0.is_null() || a1.is_null() {
        return Ok(Datum::NULL);
    }
    let (sub, text) = if has_order {
        (string(a1)?, string(a0)?)
    } else {
        (string(a0)?, string(a1)?)
    };
    if sub.is_empty() {
        return Ok(Datum::Value(Value::Integer(0)));
    }
    let pos = if at_start {
        i64::from(text.starts_with(&sub))
    } else if text.ends_with(&sub) {
        let t_len = i64::try_from(text.chars().count()).unwrap_or(0);
        let s_len = i64::try_from(sub.chars().count()).unwrap_or(0);
        t_len.saturating_sub(s_len).saturating_add(1)
    } else {
        0
    };
    Ok(Datum::Value(Value::Integer(pos)))
}

fn mid_match(args: &[Datum], has_order: bool) -> Result<Datum, ClemError> {
    let (Some(a0), Some(a1)) = (args.first(), args.get(1)) else {
        return Err(ClemError::Type("needs two arguments".into()));
    };
    if a0.is_null() || a1.is_null() {
        return Ok(Datum::NULL);
    }
    let (sub, text) = if has_order {
        (string(a1)?, string(a0)?)
    } else {
        (string(a0)?, string(a1)?)
    };
    let text_chars: Vec<char> = text.chars().collect();
    let sub_chars: Vec<char> = sub.chars().collect();
    if sub_chars.is_empty() || text_chars.len() < sub_chars.len().saturating_add(2) {
        return Ok(Datum::Value(Value::Integer(0)));
    }
    let found = (1..text_chars.len() - sub_chars.len()).find(|&i| {
        text_chars
            .get(i..i + sub_chars.len())
            .is_some_and(|w| w == sub_chars.as_slice())
    });
    let pos = found.map_or(0, |i| i64::try_from(i).unwrap_or(0).saturating_add(1));
    Ok(Datum::Value(Value::Integer(pos)))
}

fn char_pred(args: &[Datum], pred: fn(char) -> bool) -> Result<Datum, ClemError> {
    let value = first(args)?;
    if value.is_null() {
        return Ok(Datum::NULL);
    }
    Ok(Datum::Bool(pred(char_arg(&value)?)))
}

fn soundex_code(text: &str) -> String {
    let digit = |c: char| -> Option<char> {
        match c.to_ascii_uppercase() {
            'B' | 'F' | 'P' | 'V' => Some('1'),
            'C' | 'G' | 'J' | 'K' | 'Q' | 'S' | 'X' | 'Z' => Some('2'),
            'D' | 'T' => Some('3'),
            'L' => Some('4'),
            'M' | 'N' => Some('5'),
            'R' => Some('6'),
            'H' | 'W' => None,
            _ => Some('0'),
        }
    };
    let mut letters = text.chars().filter(|c| c.is_ascii_alphabetic());
    let Some(first_char) = letters.next() else {
        // GAP: SoundEx on a string without ASCII letters.
        return "0000".to_string();
    };
    let mut out = String::with_capacity(4);
    out.push(first_char.to_ascii_uppercase());
    let mut prev = digit(first_char).unwrap_or('0');
    for ch in letters {
        if out.len() == 4 {
            break;
        }
        if let Some(d) = digit(ch) {
            if d != '0' && d != prev {
                out.push(d);
            }
            prev = d;
        }
    }
    while out.len() < 4 {
        out.push('0');
    }
    out
}
