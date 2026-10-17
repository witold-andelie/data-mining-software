//! Delimited text as tables, the way Modeler's Var. File node reads it. Field texts become
//! values as [`crate::text`] describes.
//!
//! Implemented (settings of `DelimitedFileImportNode`): delimiters (comma, tab, space, other
//! characters, non-printing characters), header row, header lines to skip, quote handling
//! (`include`, `discard`, `pairDiscard`) for single and double quotes, and the shared text
//! settings ([`TextOptions`]).
//!
//! Rules (checked against the oracle for `DRUG1n`, `HEART.csv`, `acct97.txt`, `customer.dat`,
//! `fraud_info.csv`, `customer_bank_data.csv` and `goods.dat`):
//! - Field names are kept exactly as written (no trimming: `"sex "` stays `sex `); a missing
//!   header gives `field1`, `field2`, ... (`oracle/probes/settings_variablefile/s090.str`).
//! - A field missing from a short record is `Null` (`oracle/probes/clem_semantics`).
//! - Only undecodable bytes are invalid characters.
//!
//! GAPs: records are lines (quoted fields may not span lines); `multiBlankDelimiters` and
//! end-of-line comments are not supported.

use crate::text::{TextError, TextOptions, build_table, decode, finish};
use dms_types::Table;
use std::collections::HashSet;

/// Columns accepted from one file.
pub const MAX_COLUMNS: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QuoteMode {
    /// The quote character is ordinary text.
    Include,
    /// Quote characters are removed wherever they occur.
    Discard,
    /// In a field that starts with a quote, every quote toggles quoted text: the quotes are
    /// removed and delimiters between a pair are text. The field ends at a delimiter outside the
    /// pairs (`exports/export_main.csv`: `"say ""hi"""` reads `say hi`, and
    /// `"q ""and"" semi;colon,comma"` reads `q and semi;colon,comma`).
    #[default]
    PairAndDiscard,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelimitedOptions {
    pub comma: bool,
    pub tab: bool,
    pub space: bool,
    pub non_printing: bool,
    /// Extra delimiter characters.
    pub other: Vec<char>,
    pub field_names_included: bool,
    /// Fixed number of fields when `useManualFieldCount` is enabled.
    pub manual_field_count: Option<usize>,
    pub single_quote: QuoteMode,
    pub double_quote: QuoteMode,
    /// Encoding, trimming, records to skip (counted after quotes are resolved) and value reading.
    pub text: TextOptions,
}

impl Default for DelimitedOptions {
    fn default() -> Self {
        DelimitedOptions {
            comma: true,
            tab: false,
            space: false,
            non_printing: false,
            other: Vec::new(),
            field_names_included: true,
            manual_field_count: None,
            single_quote: QuoteMode::PairAndDiscard,
            double_quote: QuoteMode::PairAndDiscard,
            text: TextOptions::default(),
        }
    }
}

/// Reads a delimited file.
pub fn read_table(bytes: &[u8], options: &DelimitedOptions) -> Result<Table, TextError> {
    let text = decode(bytes, &options.text, |_| false)?;
    let mut records = parse_records(&text, options);
    let skipped = options.text.skip_lines.min(records.len());
    records.drain(..skipped);
    let header = if options.field_names_included && !records.is_empty() {
        Some(records.remove(0))
    } else {
        None
    };
    let width = options
        .manual_field_count
        .map(|n| n.max(1))
        .unwrap_or_else(|| {
            header
                .as_ref()
                .map_or_else(|| records.iter().map(Vec::len).max().unwrap_or(0), Vec::len)
        });
    if width > MAX_COLUMNS {
        return Err(TextError::TooManyColumns(width));
    }
    let names = field_names(header.as_deref(), width);
    build_table(&names, &records, &options.text)
}

fn field_names(header: Option<&[String]>, width: usize) -> Vec<String> {
    (0..width)
        .map(|index| match header.and_then(|names| names.get(index)) {
            Some(name) if !name.is_empty() => name.clone(),
            _ => format!("field{}", index + 1),
        })
        .collect()
}

struct Scanner<'a> {
    chars: Vec<char>,
    options: &'a DelimitedOptions,
    /// Quote characters for which no closing quote exists after some position.
    unclosed: HashSet<char>,
}

impl Scanner<'_> {
    fn is_delimiter(&self, c: char) -> bool {
        match c {
            ',' => self.options.comma,
            '\t' => self.options.tab,
            ' ' => self.options.space,
            '\n' | '\r' => false,
            c if c.is_control() => self.options.non_printing || self.options.other.contains(&c),
            c => self.options.other.contains(&c),
        }
    }

    fn quote_mode(&self, c: char) -> Option<QuoteMode> {
        match c {
            '"' => Some(self.options.double_quote),
            '\'' => Some(self.options.single_quote),
            _ => None,
        }
    }

    fn ends_field(&self, index: usize) -> bool {
        match self.chars.get(index) {
            None => true,
            Some(&c) => c == '\n' || c == '\r' || self.is_delimiter(c),
        }
    }

    /// The value of the field whose first quote is at `open`, and the index after its last
    /// character, if every quote in it is paired before the text ends.
    fn quoted_field(&mut self, open: usize, quote: char) -> Option<(String, usize)> {
        if self.unclosed.contains(&quote) {
            return None;
        }
        let mut value = String::new();
        let mut inside = true;
        let mut index = open + 1;
        while let Some(&c) = self.chars.get(index) {
            if c == quote {
                inside = !inside;
            } else if !inside && self.ends_field(index) {
                return Some((value, index));
            } else {
                value.push(c);
            }
            index += 1;
        }
        if !inside {
            return Some((value, index));
        }
        self.unclosed.insert(quote);
        None
    }
}

/// Splits the text into records of raw field texts (quotes resolved, nothing trimmed yet except
/// by the trim setting).
fn parse_records(text: &str, options: &DelimitedOptions) -> Vec<Vec<String>> {
    let mut scanner = Scanner {
        chars: text.chars().collect(),
        options,
        unclosed: HashSet::new(),
    };
    let mut records = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut at_field_start = true;
    let mut index = 0;
    while let Some(&c) = scanner.chars.get(index) {
        if at_field_start
            && let Some(mode) = scanner.quote_mode(c)
            && mode == QuoteMode::PairAndDiscard
            && let Some((value, end)) = scanner.quoted_field(index, c)
        {
            field.push_str(&value);
            index = end;
            at_field_start = false;
            continue;
        }
        match c {
            '\r' | '\n' => {
                if c == '\r' && scanner.chars.get(index + 1) == Some(&'\n') {
                    index += 1;
                }
                if !record.is_empty() || !field.is_empty() {
                    record.push(finish(std::mem::take(&mut field), options.text.trim));
                    records.push(std::mem::take(&mut record));
                }
                at_field_start = true;
            }
            c if scanner.is_delimiter(c) => {
                record.push(finish(std::mem::take(&mut field), options.text.trim));
                at_field_start = true;
            }
            c if scanner.quote_mode(c) == Some(QuoteMode::Discard) => {
                at_field_start = false;
            }
            c => {
                field.push(c);
                at_field_start = false;
            }
        }
        index += 1;
    }
    if !record.is_empty() || !field.is_empty() {
        record.push(finish(field, options.text.trim));
        records.push(record);
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{DateFormat, DecimalSymbol, StorageOverride, TextEncoding, Trim};
    use dms_types::{Storage, Value, calendar};

    fn read(text: &str, options: &DelimitedOptions) -> Table {
        read_table(text.as_bytes(), options).unwrap()
    }

    fn column(table: &Table, name: &str) -> Vec<Value> {
        table.column(name).unwrap().cloned().collect()
    }

    #[test]
    fn infers_storage_and_keeps_names() {
        let table = read(
            "Age,sex ,Na,Drug\r\n23,F,0.79,drugY\r\n47,,0.7,drugC\r\n",
            &DelimitedOptions::default(),
        );
        let storages: Vec<Storage> = table.model().fields().iter().map(|f| f.storage).collect();
        assert_eq!(
            storages,
            [
                Storage::Integer,
                Storage::String,
                Storage::Real,
                Storage::String
            ]
        );
        assert!(table.model().field("sex ").is_some());
        assert_eq!(
            column(&table, "sex "),
            [Value::string("F"), Value::string("")]
        );
        assert_eq!(
            column(&table, "Age"),
            [Value::Integer(23), Value::Integer(47)]
        );
    }

    #[test]
    fn headerless_files_get_field_names_and_skip_lines() {
        let options = DelimitedOptions {
            field_names_included: false,
            text: TextOptions {
                skip_lines: 1,
                ..TextOptions::default()
            },
            ..DelimitedOptions::default()
        };
        let table = read("junk\n1,2\n3,4\n", &options);
        assert_eq!(table.model().fields()[1].name, "field2");
        assert_eq!(table.row_count(), 2);
    }

    #[test]
    fn quotes_by_mode() {
        let paired = read(
            "a,b\n\"x,y\",\"say \"\"hi\"\"\"\n",
            &DelimitedOptions::default(),
        );
        assert_eq!(column(&paired, "a"), [Value::string("x,y")]);
        // Every quote toggles (`exports/export_main.csv`); a doubled quote is not an escape.
        assert_eq!(column(&paired, "b"), [Value::string("say hi")]);

        let include = DelimitedOptions {
            double_quote: QuoteMode::Include,
            ..DelimitedOptions::default()
        };
        let table = read("a,b\n\"x\",1\n", &include);
        assert_eq!(column(&table, "a"), [Value::string("\"x\"")]);

        let discard = DelimitedOptions {
            double_quote: QuoteMode::Discard,
            ..DelimitedOptions::default()
        };
        let table = read("a,b\n\"x,y\",1\n", &discard);
        assert_eq!(table.model().len(), 2);
        assert_eq!(column(&table, "a"), [Value::string("x")]);
    }

    #[test]
    fn apostrophes_inside_text_and_unclosed_quotes_are_text() {
        let table = read("a\nO'Brien\n'tis\n", &DelimitedOptions::default());
        assert_eq!(
            column(&table, "a"),
            [Value::string("O'Brien"), Value::string("'tis")]
        );
    }

    #[test]
    fn other_delimiters_trim_and_decimal_comma() {
        let options = DelimitedOptions {
            comma: false,
            other: vec![';'],
            text: TextOptions {
                trim: Trim::Both,
                decimal: DecimalSymbol::Comma,
                ..TextOptions::default()
            },
            ..DelimitedOptions::default()
        };
        let table = read("a;b\n 1,5 ; x \n", &options);
        assert_eq!(column(&table, "a"), [Value::Real(1.5)]);
        assert_eq!(column(&table, "b"), [Value::string("x")]);
        let tab_space = DelimitedOptions {
            comma: false,
            tab: true,
            space: true,
            ..DelimitedOptions::default()
        };
        assert_eq!(read("a b\tc\n1 2\t3\n", &tab_space).model().len(), 3);
    }

    #[test]
    fn dates_times_and_overrides() {
        let table = read(
            "d,t,ts,n\n2020-02-29,13:05:09,2020-02-29 13:05:09,1\n,,,\n",
            &DelimitedOptions::default(),
        );
        let storages: Vec<Storage> = table.model().fields().iter().map(|f| f.storage).collect();
        assert_eq!(
            storages,
            [
                Storage::Date,
                Storage::Time,
                Storage::Timestamp,
                Storage::Integer
            ]
        );
        assert_eq!(column(&table, "d")[1], Value::Null);

        let off = DelimitedOptions {
            text: TextOptions {
                recognize_dates: false,
                ..TextOptions::default()
            },
            ..DelimitedOptions::default()
        };
        assert_eq!(
            read("d\n2020-02-29\n", &off).model().fields()[0].storage,
            Storage::String
        );

        let forced = DelimitedOptions {
            text: TextOptions {
                overrides: vec![StorageOverride {
                    field: "n".into(),
                    storage: Storage::Real,
                    date_format: None,
                    time_format: None,
                }],
                ..TextOptions::default()
            },
            ..DelimitedOptions::default()
        };
        let table = read("n\n1\nx\n", &forced);
        assert_eq!(column(&table, "n"), [Value::Real(1.0), Value::Null]);
    }

    #[test]
    fn big_integers_become_real_and_special_floats_are_text() {
        let table = read(
            "a,b\n99999999999999999999,nan\n1,inf\n",
            &DelimitedOptions::default(),
        );
        assert_eq!(table.model().fields()[0].storage, Storage::Real);
        assert_eq!(table.model().fields()[1].storage, Storage::String);
    }

    #[test]
    fn encodings() {
        let latin = b"a\n\xe9\n";
        let table = read_table(latin, &DelimitedOptions::default()).unwrap();
        assert_eq!(column(&table, "a"), [Value::string("\u{e9}")]);
        let options = DelimitedOptions {
            text: TextOptions {
                encoding: TextEncoding::Label("nonsense".into()),
                ..TextOptions::default()
            },
            ..DelimitedOptions::default()
        };
        assert!(read_table(b"a\n", &options).is_err());
        let bom = read_table(b"\xef\xbb\xbfa\n1\n", &DelimitedOptions::default()).unwrap();
        assert_eq!(bom.model().fields()[0].name, "a");
    }

    #[test]
    fn empty_and_malformed_inputs_do_not_panic() {
        let options = DelimitedOptions::default();
        assert_eq!(read_table(b"", &options).unwrap().model().len(), 0);
        assert_eq!(read_table(b"a,b\n", &options).unwrap().row_count(), 0);
        assert!(read_table(b"a,a\n1,2\n", &options).is_err());
        let ragged = read_table(b"a,b\n1\n1,2,3\n", &options).unwrap();
        assert_eq!(column(&ragged, "b"), [Value::Null, Value::Integer(2)]);
        let _ = read_table(b"\"\"\"\n'\n\x00\xff", &options);
    }

    #[test]
    fn scan_lines_and_date_formats() {
        // Invalid date after scan_lines becomes Null while column stays Date (like acct97.txt).
        let options = DelimitedOptions {
            text: TextOptions {
                scan_lines: 2,
                ..TextOptions::default()
            },
            ..DelimitedOptions::default()
        };
        let table = read("d\n 5/ 1/1995\n11/2/1997\n30/2/1997\n", &options);
        assert_eq!(table.model().fields()[0].storage, Storage::Date);
        assert_eq!(
            column(&table, "d"),
            [
                Value::Date(calendar::days_from_civil(1995, 1, 5).unwrap()),
                Value::Date(calendar::days_from_civil(1997, 2, 11).unwrap()),
                Value::Null,
            ]
        );

        // Stream date format MM/DD/YY (format7) with 2-digit year baseline 1930.
        let mdy = DelimitedOptions {
            text: TextOptions {
                date_format: DateFormat::from_modeler_id("format7").unwrap(),
                two_digit_year_baseline: 1930,
                ..TextOptions::default()
            },
            ..DelimitedOptions::default()
        };
        let table = read("d\n05/01/95\n05/01/25\n", &mdy);
        assert_eq!(
            column(&table, "d"),
            [
                Value::Date(calendar::days_from_civil(1995, 5, 1).unwrap()),
                Value::Date(calendar::days_from_civil(2025, 5, 1).unwrap()),
            ]
        );

        // DD-MON-YYYY (format20).
        let mon = DelimitedOptions {
            text: TextOptions {
                date_format: DateFormat::from_modeler_id("format20").unwrap(),
                ..TextOptions::default()
            },
            ..DelimitedOptions::default()
        };
        let table = read("d\n15-Feb-1997\n", &mon);
        assert_eq!(
            column(&table, "d"),
            [Value::Date(calendar::days_from_civil(1997, 2, 15).unwrap())]
        );
    }
}
