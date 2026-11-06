//! Delimited text output in the convention of Modeler's Flat File export with the settings the
//! oracle uses (UTF-8, comma, double quotes, header row): field names are quoted only when
//! needed, string values are always quoted (spaces kept), numbers, dates and times are not, and
//! `$null$` is an empty field. Inner quotes are doubled (RFC 4180); Modeler leaves them as they
//! are, which `xtask compare` reads as Modeler's dialect.
//! Reals use the shortest text that reads back to the same value. A field's
//! [`dms_types::DisplayFormat`] overrides the plain rendering (`Dec 2019`); otherwise `dates`,
//! the stream's date print format, applies to Date values (`format27`: `Jan 1999`).

use dms_types::{DisplayFormat, Storage, Table, Value};
use std::io::{self, Write};

pub fn write_table(
    table: &Table,
    out: &mut impl Write,
    dates: Option<DisplayFormat>,
) -> io::Result<()> {
    let header: Vec<String> = table
        .model()
        .fields()
        .iter()
        .map(|field| header_name(&field.name))
        .collect();
    writeln!(out, "{}", header.join(","))?;
    let formats: Vec<_> = table
        .model()
        .fields()
        .iter()
        .map(|f| f.display.or(dates.filter(|_| f.storage == Storage::Date)))
        .collect();
    for row in table.rows() {
        let cells: Vec<String> = row
            .iter()
            .zip(&formats)
            .map(
                |(value, format)| match format.and_then(|f| f.render(value)) {
                    Some(text) => text,
                    None => cell(value),
                },
            )
            .collect();
        writeln!(out, "{}", cells.join(","))?;
    }
    Ok(())
}

fn cell(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => quote(text),
        Value::List(_) => quote(&value.to_string()),
        other => other.to_string(),
    }
}

fn header_name(name: &str) -> String {
    if name.contains([',', '"', '\n', '\r']) {
        quote(name)
    } else {
        name.to_string()
    }
}

fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dms_types::{DataModel, Field, Storage};

    #[test]
    fn writes_modeler_style_csv() {
        let model = DataModel::new(vec![
            Field::new("name", Storage::String),
            Field::new("n", Storage::Integer),
            Field::new("x", Storage::Real),
            Field::new("d", Storage::Date),
        ])
        .unwrap();
        let table = Table::new(
            model,
            vec![
                vec![
                    Value::string("MALE  \"x\""),
                    Value::Integer(23),
                    Value::Real(0.1),
                    Value::Date(9135),
                ],
                vec![Value::Null, Value::Null, Value::Real(2.0), Value::Null],
            ],
        )
        .unwrap();
        let mut out = Vec::new();
        write_table(&table, &mut out, None).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "name,n,x,d\n\"MALE  \"\"x\"\"\",23,0.1,1995-01-05\n,,2,\n"
        );
    }
}
