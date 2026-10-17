//! Fixed-width text as tables, the way Modeler's Fixed File node reads it. Field texts become
//! values as [`crate::text`] describes.
//!
//! Rules (`oracle/sources/fixed_*`):
//! - Records are lines (`\n` or `\r\n`; the last one may lack its line end) when line-oriented,
//!   else consecutive blocks of `record_length` characters (`fixed_packed`). `skip_lines` lines
//!   come off the front first (`fixed_skip_headers`).
//! - A field is the characters (not bytes: `fixed_utf8`) from its 1-based start for its length.
//!   A blank line is a record; a field beyond the end of a short record is empty, so the empty
//!   string in a string column and `Null` otherwise (`fixed_short_records`).
//! - NUL characters are invalid characters, like undecodable bytes. They are discarded or
//!   replaced before the record is cut into fields, so discarding one shifts the later fields
//!   (`fixed_invalid_discard`: `a\0b` gives `ab` and the next field starts one character early).
//!
//! GAP: lines to skip before packed records (no probe).

use crate::text::{TextError, TextOptions, build_table, decode, finish};
use dms_types::Table;

/// One field of the layout (`fixedFileColumnSpecs/entries/entry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedColumn {
    pub name: String,
    /// 1-based position of the first character.
    pub start: usize,
    pub length: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedOptions {
    pub columns: Vec<FixedColumn>,
    /// Characters per record when not line-oriented (`recordLength`).
    pub record_length: usize,
    /// Records end at line ends (`lineOriented`, default true).
    pub line_oriented: bool,
    pub text: TextOptions,
}

/// Reads a fixed-width file.
pub fn read_table(bytes: &[u8], options: &FixedOptions) -> Result<Table, TextError> {
    let text = decode(bytes, &options.text, |c| c == '\0')?;
    let mut lines = text.split_inclusive('\n');
    for _ in 0..options.text.skip_lines {
        if lines.next().is_none() {
            break;
        }
    }
    let records: Vec<Vec<char>> = if options.line_oriented {
        lines
            .map(|line| {
                let line = line.strip_suffix('\n').unwrap_or(line);
                line.strip_suffix('\r').unwrap_or(line).chars().collect()
            })
            .collect()
    } else {
        let rest: Vec<char> = lines.flat_map(str::chars).collect();
        rest.chunks(options.record_length.max(1))
            .map(<[char]>::to_vec)
            .collect()
    };
    let fields: Vec<Vec<String>> = records
        .iter()
        .map(|record| {
            options
                .columns
                .iter()
                .map(|column| {
                    let start = column.start.saturating_sub(1);
                    let cell: String = record.iter().skip(start).take(column.length).collect();
                    finish(cell, options.text.trim)
                })
                .collect()
        })
        .collect();
    let names: Vec<String> = options.columns.iter().map(|c| c.name.clone()).collect();
    build_table(&names, &fields, &options.text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{InvalidCharacters, Trim};
    use dms_types::{Storage, Value};

    fn options(line_oriented: bool) -> FixedOptions {
        let column = |name: &str, start, length| FixedColumn {
            name: name.into(),
            start,
            length,
        };
        FixedOptions {
            columns: vec![column("a", 1, 2), column("b", 3, 3)],
            record_length: 5,
            line_oriented,
            text: TextOptions {
                trim: Trim::Both,
                ..TextOptions::default()
            },
        }
    }

    fn column(table: &Table, name: &str) -> Vec<Value> {
        table.column(name).unwrap().cloned().collect()
    }

    #[test]
    fn lines_short_records_and_blank_lines() {
        let table = read_table(b"1 abc\r\n\r\n2 x\n3", &options(true)).unwrap();
        assert_eq!(
            column(&table, "a"),
            [
                Value::Integer(1),
                Value::Null,
                Value::Integer(2),
                Value::Integer(3)
            ]
        );
        assert_eq!(table.model().field("b").unwrap().storage, Storage::String);
        assert_eq!(
            column(&table, "b"),
            [
                Value::string("abc"),
                Value::string(""),
                Value::string("x"),
                Value::string("")
            ]
        );
    }

    #[test]
    fn packed_records_characters_and_skipped_lines() {
        let mut packed = options(false);
        packed.text.skip_lines = 1;
        let table = read_table("head\n1 é2 2 xyz".as_bytes(), &packed).unwrap();
        assert_eq!(column(&table, "a"), [Value::Integer(1), Value::Integer(2)]);
        assert_eq!(
            column(&table, "b"),
            [Value::string("é2"), Value::string("xyz")]
        );
    }

    #[test]
    fn discarded_nul_shifts_later_fields() {
        let discard = read_table(b"1\0 abc\n", &options(true)).unwrap();
        assert_eq!(column(&discard, "b"), [Value::string("abc")]);
        let mut replace = options(true);
        replace.text.invalid = InvalidCharacters::Replace("?".into());
        let table = read_table(b"1\0 abc\n", &replace).unwrap();
        assert_eq!(column(&table, "a"), [Value::string("1?")]);
        assert_eq!(column(&table, "b"), [Value::string("ab")]);
    }
}
