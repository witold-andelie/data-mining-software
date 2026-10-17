//! JSON files as tables, the way Modeler's JSON source node (`json_import!pasw.json`) reads them.
//! Modeler runs pandas `read_json` with the node's orient and turns the frame's columns into
//! fields, so the rules are pandas' (`oracle/sources/json_*`; the storages are the stream's
//! cached data model):
//!
//! - `records`: an array of objects. Fields are the keys in order of first appearance; a key a
//!   record lacks is null (`json_sparse`: `z, a, all_null, late`). A key given twice keeps its
//!   first position and its last value (`json_duplicate_keys`: `x` is 20).
//! - `values`: an array of arrays. Fields are named `0`, `1`, ... (`json_values`); a short array
//!   is null at the end.
//! - Trailing commas are accepted (`json_malformed_rejected`: `{"id":2,}` reads `id` 2).
//! - A column of integers without nulls is `Integer` (`id`). Every other column is converted to
//!   floating point when every value can be: numbers, booleans (1 and 0) and strings Python's
//!   `float()` reads, nulls staying null (`json_default`: `i` with a null, `flag` with booleans
//!   and a null; `json_mixed_types`: `y` from `true, 2, false, null`). Such a column is
//!   `Integer` when it then has no nulls and only whole numbers within 64 bits, else `Real`
//!   (`json_number_limits`: `x` with a null is real, 9007199254740993 becomes
//!   9007199254740992; `json_sparse`: `all_null` is real).
//! - Any other column is `String`: strings as they are, other values as Python's `json.dumps`
//!   writes them (`json_mixed_types`: `1`, `2.5`; `json_nested_rejected`: `{"a": 1}`), nulls null.
//!   Date-like strings stay strings (`json_date_strings`).
//! - Numeric fields are range (`Continuous`), string fields categorical.
//!
//! Rejected: an empty array (Modeler reports "No records received", `json_empty`), other
//! top-level shapes, a column of booleans without nulls (pandas keeps it boolean; no probe shows
//! its storage), integers beyond 64 bits, and columns pandas converts to dates by their name
//! (`date`, `datetime`, `modified`, `timestamp...`, `..._at`, `..._time`).
//!
//! GAPs: pandas decodes numbers with a fast routine that can differ from the correctly rounded
//! value in the last bit (`precise_float=False`); we round correctly. A boolean in a string column
//! is written `true`/`false` like `json.dumps` (no probe). Strings with `_` between digits, which
//! Python's `float()` reads, stay strings.

use dms_types::{DataModel, Field, Measurement, ModelError, Storage, Table, Value};

/// Nesting accepted in one file.
const MAX_DEPTH: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orient {
    Records,
    Values,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonError {
    /// Not JSON text: the byte offset and what was wrong.
    Syntax(usize, String),
    /// JSON that is not a table in the orient.
    Shape(String),
    Unsupported(String),
    Model(ModelError),
}

impl std::fmt::Display for JsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JsonError::Syntax(offset, message) => write!(f, "{message} at byte {offset}"),
            JsonError::Shape(message) => write!(f, "{message}"),
            JsonError::Unsupported(message) => write!(f, "{message} is not supported yet"),
            JsonError::Model(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for JsonError {}

#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

/// Reads a JSON file (UTF-8) in the given orient.
pub fn read_table(text: &str, orient: Orient) -> Result<Table, JsonError> {
    let document = Parser::new(text).document()?;
    let Json::Array(rows) = document else {
        return Err(JsonError::Unsupported(
            "a JSON file that is not an array".into(),
        ));
    };
    if rows.is_empty() {
        return Err(JsonError::Shape("No records received".into()));
    }
    let (names, columns) = match orient {
        Orient::Records => records(rows)?,
        Orient::Values => values(rows)?,
    };
    let mut fields = Vec::with_capacity(names.len());
    let mut typed: Vec<Vec<Value>> = Vec::with_capacity(names.len());
    for (name, cells) in names.into_iter().zip(columns) {
        if is_date_name(&name) {
            return Err(JsonError::Unsupported(format!(
                "field {name:?}: pandas converts fields with this name to dates, which"
            )));
        }
        let (storage, values) = column(&name, cells)?;
        let mut field = Field::new(name, storage);
        field.measurement = match storage {
            Storage::String => Measurement::Categorical,
            _ => Measurement::Continuous,
        };
        fields.push(field);
        typed.push(values);
    }
    let count = typed.first().map_or(0, Vec::len);
    let rows = (0..count)
        .map(|row| {
            typed
                .iter()
                .map(|column| column.get(row).cloned().unwrap_or(Value::Null))
                .collect()
        })
        .collect();
    let model = DataModel::new(fields).map_err(JsonError::Model)?;
    Table::new(model, rows).map_err(JsonError::Model)
}

type Columns = (Vec<String>, Vec<Vec<Json>>);

fn records(rows: Vec<Json>) -> Result<Columns, JsonError> {
    let count = rows.len();
    let mut names: Vec<String> = Vec::new();
    let mut columns: Vec<Vec<Json>> = Vec::new();
    for (index, row) in rows.into_iter().enumerate() {
        let Json::Object(entries) = row else {
            return Err(JsonError::Shape(format!(
                "record {} is not a JSON object",
                index + 1
            )));
        };
        for (key, value) in entries {
            let at = match names.iter().position(|name| *name == key) {
                Some(at) => at,
                None => {
                    names.push(key);
                    columns.push(vec![Json::Null; count]);
                    columns.len() - 1
                }
            };
            if let Some(cell) = columns.get_mut(at).and_then(|c| c.get_mut(index)) {
                *cell = value;
            }
        }
    }
    Ok((names, columns))
}

fn values(rows: Vec<Json>) -> Result<Columns, JsonError> {
    let count = rows.len();
    let mut columns: Vec<Vec<Json>> = Vec::new();
    for (index, row) in rows.into_iter().enumerate() {
        let Json::Array(cells) = row else {
            return Err(JsonError::Shape(format!(
                "record {} is not a JSON array",
                index + 1
            )));
        };
        while columns.len() < cells.len() {
            columns.push(vec![Json::Null; count]);
        }
        for (column, value) in columns.iter_mut().zip(cells) {
            if let Some(cell) = column.get_mut(index) {
                *cell = value;
            }
        }
    }
    let names = (0..columns.len()).map(|i| i.to_string()).collect();
    Ok((names, columns))
}

/// pandas' `keep_default_dates`.
fn is_date_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with("_at")
        || lower.ends_with("_time")
        || lower == "modified"
        || lower == "date"
        || lower == "datetime"
        || lower.starts_with("timestamp")
}

fn column(name: &str, cells: Vec<Json>) -> Result<(Storage, Vec<Value>), JsonError> {
    if !cells.is_empty() && cells.iter().all(|c| matches!(c, Json::Int(_))) {
        let values = cells
            .iter()
            .map(|c| match c {
                Json::Int(i) => Value::Integer(*i),
                _ => Value::Null,
            })
            .collect();
        return Ok((Storage::Integer, values));
    }
    if !cells.is_empty() && cells.iter().all(|c| matches!(c, Json::Bool(_))) {
        return Err(JsonError::Unsupported(format!(
            "field {name:?}: a column of booleans without nulls"
        )));
    }
    let numbers: Option<Vec<Option<f64>>> = cells
        .iter()
        .map(|c| match c {
            Json::Null => Some(None),
            Json::Bool(b) => Some(Some(if *b { 1.0 } else { 0.0 })),
            Json::Int(i) => Some(Some(*i as f64)),
            Json::Float(f) => Some(Some(*f)),
            Json::Str(s) => python_float(s).map(Some),
            Json::Array(_) | Json::Object(_) => None,
        })
        .map(|n| n.map(|v| v.filter(|f| !f.is_nan())))
        .collect();
    if let Some(numbers) = numbers {
        // 2^63: float64 values at or above it do not fit int64.
        const LIMIT: f64 = 9_223_372_036_854_775_808.0;
        let whole = numbers.iter().all(|n| {
            n.is_some_and(|f| f.is_finite() && f.fract() == 0.0 && (-LIMIT..LIMIT).contains(&f))
        });
        if whole {
            let values = numbers
                .iter()
                .map(|n| n.map_or(Value::Null, |f| Value::Integer(f as i64)))
                .collect();
            return Ok((Storage::Integer, values));
        }
        let values = numbers
            .iter()
            .map(|n| n.map_or(Value::Null, Value::real))
            .collect();
        return Ok((Storage::Real, values));
    }
    let values = cells
        .iter()
        .map(|c| match c {
            Json::Null => Value::Null,
            Json::Str(s) => Value::string(s.as_str()),
            other => Value::string(dumps(other)),
        })
        .collect();
    Ok((Storage::String, values))
}

/// Python's `float()` of a string, without `_` digit separators.
fn python_float(text: &str) -> Option<f64> {
    let text = text.trim();
    if text.contains('_') {
        return None;
    }
    text.parse::<f64>().ok()
}

/// Python's `json.dumps` with its default separators and `ensure_ascii`.
fn dumps(value: &Json) -> String {
    let mut out = String::new();
    write_json(&mut out, value);
    out
}

fn write_json(out: &mut String, value: &Json) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(true) => out.push_str("true"),
        Json::Bool(false) => out.push_str("false"),
        Json::Int(i) => out.push_str(&i.to_string()),
        Json::Float(f) => out.push_str(&python_repr(*f)),
        Json::Str(s) => write_string(out, s),
        Json::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_json(out, item);
            }
            out.push(']');
        }
        Json::Object(entries) => {
            out.push('{');
            for (index, (key, item)) in entries.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_string(out, key);
                out.push_str(": ");
                write_json(out, item);
            }
            out.push('}');
        }
    }
}

fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            c => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out.push('"');
}

/// Python's `repr` of a float: the shortest digits that read back, fixed notation for exponents
/// from -4 to 15, else `1e+20` style.
fn python_repr(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let (sign, mantissa) = match mantissa.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", mantissa),
    };
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    if (-4..16).contains(&exponent) {
        let point = exponent + 1;
        let body = if point <= 0 {
            format!("0.{}{digits}", "0".repeat(point.unsigned_abs() as usize))
        } else {
            let point = point.unsigned_abs() as usize;
            if digits.len() > point {
                format!("{}.{}", &digits[..point], &digits[point..])
            } else {
                format!("{digits}{}.0", "0".repeat(point - digits.len()))
            }
        };
        format!("{sign}{body}")
    } else {
        let exponent_sign = if exponent < 0 { '-' } else { '+' };
        format!(
            "{sign}{mantissa}e{exponent_sign}{:02}",
            exponent.unsigned_abs()
        )
    }
}

struct Parser<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Parser { text, at: 0 }
    }

    fn error<T>(&self, message: &str) -> Result<T, JsonError> {
        Err(JsonError::Syntax(self.at, message.to_string()))
    }

    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.at).copied()
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn document(&mut self) -> Result<Json, JsonError> {
        self.skip_space();
        let value = self.value(0)?;
        self.skip_space();
        if self.at < self.text.len() {
            return self.error("unexpected text after the JSON value");
        }
        Ok(value)
    }

    fn value(&mut self, depth: usize) -> Result<Json, JsonError> {
        if depth > MAX_DEPTH {
            return self.error("nesting too deep");
        }
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Json::Str),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => {
                for (word, value) in [
                    ("true", Json::Bool(true)),
                    ("false", Json::Bool(false)),
                    ("null", Json::Null),
                ] {
                    if self.text[self.at..].starts_with(word) {
                        self.at += word.len();
                        return Ok(value);
                    }
                }
                self.error("expected a JSON value")
            }
            None => self.error("unexpected end of the file"),
        }
    }

    /// Items up to `close`, separated by commas, a trailing comma allowed.
    fn items(
        &mut self,
        close: u8,
        mut item: impl FnMut(&mut Self) -> Result<(), JsonError>,
    ) -> Result<(), JsonError> {
        self.at += 1;
        self.skip_space();
        if self.peek() == Some(close) {
            self.at += 1;
            return Ok(());
        }
        loop {
            item(self)?;
            self.skip_space();
            match self.peek() {
                Some(b',') => {
                    self.at += 1;
                    self.skip_space();
                    if self.peek() == Some(close) {
                        self.at += 1;
                        return Ok(());
                    }
                }
                Some(c) if c == close => {
                    self.at += 1;
                    return Ok(());
                }
                _ => return self.error("expected a comma or the end of the list"),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, JsonError> {
        let mut items = Vec::new();
        self.items(b']', |parser| {
            items.push(parser.value(depth + 1)?);
            Ok(())
        })?;
        Ok(Json::Array(items))
    }

    fn object(&mut self, depth: usize) -> Result<Json, JsonError> {
        let mut entries: Vec<(String, Json)> = Vec::new();
        self.items(b'}', |parser| {
            if parser.peek() != Some(b'"') {
                return parser.error("expected a key");
            }
            let key = parser.string()?;
            parser.skip_space();
            if parser.peek() != Some(b':') {
                return parser.error("expected a colon");
            }
            parser.at += 1;
            parser.skip_space();
            let value = parser.value(depth + 1)?;
            match entries.iter_mut().find(|(k, _)| *k == key) {
                Some(entry) => entry.1 = value,
                None => entries.push((key, value)),
            }
            Ok(())
        })?;
        Ok(Json::Object(entries))
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.at += 1;
        let mut out = String::new();
        loop {
            let rest = &self.text[self.at..];
            let Some(c) = rest.chars().next() else {
                return self.error("unterminated string");
            };
            self.at += c.len_utf8();
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let escape = self.peek();
                    self.at += 1;
                    match escape {
                        Some(b'"') => out.push('"'),
                        Some(b'\\') => out.push('\\'),
                        Some(b'/') => out.push('/'),
                        Some(b'b') => out.push('\u{8}'),
                        Some(b'f') => out.push('\u{c}'),
                        Some(b'n') => out.push('\n'),
                        Some(b'r') => out.push('\r'),
                        Some(b't') => out.push('\t'),
                        Some(b'u') => out.push(self.unicode_escape()?),
                        _ => return self.error("bad escape"),
                    }
                }
                c if u32::from(c) < 0x20 => return self.error("control character in a string"),
                c => out.push(c),
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let digits = self.text.get(self.at..self.at + 4);
        match digits.and_then(|d| u32::from_str_radix(d, 16).ok()) {
            Some(unit) if digits.is_some_and(|d| d.bytes().all(|b| b.is_ascii_hexdigit())) => {
                self.at += 4;
                Ok(unit)
            }
            _ => self.error("bad \\u escape"),
        }
    }

    fn unicode_escape(&mut self) -> Result<char, JsonError> {
        let first = self.hex4()?;
        let code = if (0xd800..0xdc00).contains(&first) && self.text[self.at..].starts_with("\\u") {
            let mark = self.at;
            self.at += 2;
            let second = self.hex4()?;
            if (0xdc00..0xe000).contains(&second) {
                0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00)
            } else {
                self.at = mark;
                first
            }
        } else {
            first
        };
        // A lone surrogate is not a character; Python keeps it, we cannot.
        char::from_u32(code).map_or_else(|| self.error("lone surrogate in \\u escape"), Ok)
    }

    fn number(&mut self) -> Result<Json, JsonError> {
        let start = self.at;
        let bytes = self.text.as_bytes();
        let mut end = start;
        if bytes.get(end) == Some(&b'-') {
            end += 1;
        }
        let digits = |from: usize| {
            bytes[from..]
                .iter()
                .take_while(|b| b.is_ascii_digit())
                .count()
        };
        let whole = digits(end);
        if whole == 0 {
            return self.error("bad number");
        }
        end += whole;
        let mut float = false;
        if bytes.get(end) == Some(&b'.') {
            let fraction = digits(end + 1);
            if fraction == 0 {
                return self.error("bad number");
            }
            end += 1 + fraction;
            float = true;
        }
        if matches!(bytes.get(end), Some(b'e' | b'E')) {
            end += 1;
            if matches!(bytes.get(end), Some(b'+' | b'-')) {
                end += 1;
            }
            let exponent = digits(end);
            if exponent == 0 {
                return self.error("bad number");
            }
            end += exponent;
            float = true;
        }
        let literal = &self.text[start..end];
        self.at = end;
        if float {
            literal
                .parse::<f64>()
                .map(Json::Float)
                .or_else(|_| self.error("bad number"))
        } else {
            literal.parse::<i64>().map(Json::Int).map_err(|_| {
                JsonError::Unsupported(format!("the integer {literal} beyond 64 bits"))
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(table: &Table, name: &str) -> Vec<Value> {
        table.column(name).unwrap().cloned().collect()
    }

    fn storages(table: &Table) -> Vec<Storage> {
        table.model().fields().iter().map(|f| f.storage).collect()
    }

    #[test]
    fn records_types_and_key_order() {
        let table = read_table(
            r#"[{"id":3,"i":-10,"flag":true,"s":"a"},{"i":null,"s":null,"id":1,"flag":null,"late":"7"},]"#,
            Orient::Records,
        )
        .unwrap();
        let names: Vec<&str> = table
            .model()
            .fields()
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(names, ["id", "i", "flag", "s", "late"]);
        assert_eq!(
            storages(&table),
            [
                Storage::Integer,
                Storage::Real,
                Storage::Real,
                Storage::String,
                Storage::Real
            ]
        );
        assert_eq!(column(&table, "flag"), [Value::Real(1.0), Value::Null]);
        assert_eq!(column(&table, "s"), [Value::string("a"), Value::Null]);
        assert_eq!(column(&table, "late"), [Value::Null, Value::Real(7.0)]);
    }

    #[test]
    fn whole_floats_strings_and_mixed_values() {
        let table = read_table(
            r#"[[1.0,"2",1,{"a":[1,2.5,"é"]},true],[2.0," 3 ","x",null,1]]"#,
            Orient::Values,
        )
        .unwrap();
        assert_eq!(
            storages(&table),
            [
                Storage::Integer,
                Storage::Integer,
                Storage::String,
                Storage::String,
                Storage::Integer
            ]
        );
        assert_eq!(column(&table, "1"), [Value::Integer(2), Value::Integer(3)]);
        assert_eq!(
            column(&table, "2"),
            [Value::string("1"), Value::string("x")]
        );
        assert_eq!(
            column(&table, "3"),
            [Value::string(r#"{"a": [1, 2.5, "\u00e9"]}"#), Value::Null]
        );
    }

    #[test]
    fn duplicate_keys_keep_first_position_and_last_value() {
        let table = read_table(r#"[{"x":1,"id":2,"x":3}]"#, Orient::Records).unwrap();
        assert_eq!(table.model().fields()[0].name, "x");
        assert_eq!(column(&table, "x"), [Value::Integer(3)]);
    }

    #[test]
    fn rejections() {
        assert!(matches!(
            read_table("[]", Orient::Records),
            Err(JsonError::Shape(_))
        ));
        assert!(read_table(r#"{"a":1}"#, Orient::Records).is_err());
        assert!(read_table(r#"[[1]]"#, Orient::Records).is_err());
        assert!(read_table(r#"[{"a":true}]"#, Orient::Records).is_err());
        assert!(read_table(r#"[{"date":"x"}]"#, Orient::Records).is_err());
        assert!(read_table(r#"[{"a":99999999999999999999}]"#, Orient::Records).is_err());
        assert!(read_table(r#"[{"a":1} x]"#, Orient::Records).is_err());
        assert!(read_table(r#"[{"a":"b]"#, Orient::Records).is_err());
        assert!(read_table(&"[".repeat(MAX_DEPTH + 10), Orient::Values).is_err());
    }

    #[test]
    fn python_float_repr() {
        assert_eq!(python_repr(2.5), "2.5");
        assert_eq!(python_repr(1.0), "1.0");
        assert_eq!(python_repr(-0.0), "-0.0");
        assert_eq!(python_repr(1e20), "1e+20");
        assert_eq!(python_repr(1e16), "1e+16");
        assert_eq!(python_repr(1234567890123456.0), "1234567890123456.0");
        assert_eq!(python_repr(0.0001), "0.0001");
        assert_eq!(python_repr(1.5e-5), "1.5e-05");
        assert_eq!(python_repr(0.123456789012345), "0.123456789012345");
    }
}
