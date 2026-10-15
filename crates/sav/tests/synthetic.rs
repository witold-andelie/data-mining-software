//! Synthetic files built by the test encoder in `common`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use common::{Cell, Miss, Spec, Var, build, s};
use dms_sav::{
    Alignment, Compression, Endian, ErrorKind, Limits, Measure, MissingValues, Value, read,
};

fn long_text(n: usize) -> String {
    (0..n).map(|i| char::from(b'a' + (i % 26) as u8)).collect()
}

fn mixed_spec() -> Spec {
    let mut name = Var::text("NAME", 5);
    name.label = Some("Person name");
    let mut score = Var::num("SCORE");
    score.long = Some("Exam_Score_Total");
    score.missing = Miss::Num(vec![-9.0, 999.0]);
    let mut grade = Var::num("GRADE");
    grade.missing = Miss::Range(90.0, f64::MAX);
    let mut city = Var::text("CITY", 20);
    city.long = Some("City_of_residence");
    let mut note = Var::text("NOTE", 300);
    note.label = Some("Very long");
    let rows = vec![
        vec![
            s("Ann"),
            Cell::Num(12.5),
            Cell::Num(95.0),
            s("Z\u{fc}rich"),
            s(&long_text(300)),
        ],
        vec![
            s(""),
            Cell::Sys,
            Cell::Num(7.0),
            s("A city name of 20 ch"),
            s("short"),
        ],
        vec![
            s("Bob"),
            Cell::Num(-9.0),
            Cell::Num(0.0),
            s(""),
            s(&long_text(256)),
        ],
        vec![s("Cy"), Cell::Num(1e300), Cell::Num(-1.5), s("x"), s("")],
    ];
    let mut spec = Spec::new(vec![name, score, grade, city, note], rows);
    spec.labels = vec![(
        vec![2],
        vec![(Cell::Num(0.0), "zero"), (Cell::Num(7.0), "seven")],
    )];
    spec.docs = vec!["first line", "second line"];
    spec.char_code = 1252;
    spec
}

fn check_mixed(spec: &Spec) {
    let bytes = build(spec);
    let file = read(&bytes, &Limits::default()).expect("reads");
    assert_eq!(file.header.case_count, Some(4));
    assert_eq!(
        file.header.endianness,
        if spec.big {
            Endian::Big
        } else {
            Endian::Little
        }
    );
    assert_eq!(file.variables.len(), 5, "{:?}", file.warnings);
    let names: Vec<&str> = file.variables.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "NAME",
            "Exam_Score_Total",
            "GRADE",
            "City_of_residence",
            "NOTE"
        ]
    );
    assert_eq!(file.variables[1].short_name, "SCORE");
    assert_eq!(file.variables[0].label.as_deref(), Some("Person name"));
    assert_eq!(file.variables[0].width, 5);
    assert_eq!(file.variables[3].width, 20);
    assert_eq!(file.variables[4].width, 300);
    assert_eq!(file.variables[1].width, 0);
    assert_eq!(file.variables[1].measure, Measure::Scale);
    assert_eq!(file.variables[0].measure, Measure::Nominal);
    assert_eq!(file.variables[1].alignment, Some(Alignment::Right));
    assert_eq!(file.variables[0].alignment, Some(Alignment::Left));
    assert_eq!(file.variables[1].display_width, Some(8));
    assert_eq!(file.variables[1].print_format.type_code, 5);
    assert_eq!(file.variables[1].print_format.width, 8);
    assert_eq!(file.variables[1].print_format.decimals, 2);
    assert_eq!(
        file.variables[1].missing,
        MissingValues::Discrete(vec![Value::Number(-9.0), Value::Number(999.0)])
    );
    assert_eq!(
        file.variables[2].missing,
        MissingValues::Range {
            low: 90.0,
            high: f64::MAX
        }
    );
    assert_eq!(file.documents, ["first line", "second line"]);
    assert_eq!(file.value_labels.len(), 1);
    assert_eq!(file.value_labels[0].variables, [2]);
    assert_eq!(file.value_labels[0].labels[1].label, "seven");
    assert_eq!(file.value_labels[0].labels[1].value, Value::Number(7.0));
    assert_eq!(file.rows.len(), 4);
    assert_eq!(file.rows[0][0], Value::String("Ann".into()));
    assert_eq!(file.rows[0][3], Value::String("Z\u{fc}rich".into()));
    assert_eq!(file.rows[0][4], Value::String(long_text(300)));
    assert_eq!(file.rows[1][1], Value::SystemMissing);
    assert_eq!(
        file.rows[1][3],
        Value::String("A city name of 20 ch".into())
    );
    assert_eq!(file.rows[1][4], Value::String("short".into()));
    // User-missing values are not altered.
    assert_eq!(file.rows[2][1], Value::Number(-9.0));
    assert_eq!(file.rows[2][4], Value::String(long_text(256)));
    assert_eq!(file.rows[3][1], Value::Number(1e300));
    assert_eq!(file.rows[3][2], Value::Number(-1.5));
    assert_eq!(file.rows[3][4], Value::String(String::new()));
    assert!(file.unknown_extensions.is_empty());
    assert_eq!(file.encoding_used, "windows-1252");
}

#[test]
fn uncompressed_little_endian() {
    check_mixed(&mixed_spec());
}

#[test]
fn uncompressed_big_endian() {
    let mut spec = mixed_spec();
    spec.big = true;
    check_mixed(&spec);
}

#[test]
fn bytecode_both_endians() {
    for big in [false, true] {
        let mut spec = mixed_spec();
        spec.big = big;
        spec.compression = 1;
        let file = read(&build(&spec), &Limits::default()).expect("reads");
        assert_eq!(file.header.compression, Compression::Bytecode);
        check_mixed(&spec);
    }
}

#[test]
fn zlib_both_endians_and_block_sizes() {
    for big in [false, true] {
        for block in [1, 7, 64, 100_000] {
            let mut spec = mixed_spec();
            spec.big = big;
            spec.compression = 2;
            spec.zlib_block = block;
            let file = read(&build(&spec), &Limits::default()).expect("reads");
            assert_eq!(file.header.compression, Compression::Zlib);
            check_mixed(&spec);
        }
    }
}

#[test]
fn encoding_record_utf8_wins_over_character_code() {
    let mut spec = mixed_spec();
    spec.encoding = Some("UTF-8");
    spec.char_code = 1252;
    let file = read(&build(&spec), &Limits::default()).expect("reads");
    assert_eq!(file.encoding.as_deref(), Some("UTF-8"));
    assert_eq!(file.encoding_used, "UTF-8");
    assert_eq!(file.rows[0][3], Value::String("Z\u{fc}rich".into()));
}

#[test]
fn character_code_selects_code_page() {
    // 0xE9 is "e acute" in windows-1252 and a Cyrillic letter in windows-1251.
    let mut spec = Spec::new(vec![Var::text("T", 4)], vec![vec![s("\u{e9}")]]);
    spec.char_code = 1251;
    let file = read(&build(&spec), &Limits::default()).expect("reads");
    assert_eq!(file.encoding_used, "windows-1251");
    assert_eq!(file.rows[0][0], Value::String("\u{439}".into()));
}

#[test]
fn unknown_encoding_falls_back_with_warning() {
    let mut spec = Spec::new(vec![Var::text("T", 4)], vec![vec![s("ab")]]);
    spec.encoding = Some("NO-SUCH-CODEPAGE");
    spec.char_code = 99999;
    let file = read(&build(&spec), &Limits::default()).expect("reads");
    assert_eq!(file.encoding_used, "windows-1252");
    assert!(!file.warnings.is_empty());
}

#[test]
fn long_string_value_labels_and_missing() {
    let mut text = Var::text("TXT", 12);
    text.missing = Miss::LongStr(vec!["NA"]);
    let mut short = Var::text("SH", 3);
    short.missing = Miss::Str(vec!["x", "yy"]);
    let rows = vec![vec![s("hello world"), s("a")], vec![s("NA"), s("x")]];
    let mut spec = Spec::new(vec![text, short], rows);
    spec.long_labels = vec![(
        "TXT",
        vec![("hello world", "Greeting"), ("NA", "Not available")],
    )];
    spec.labels = vec![(vec![1], vec![(s("a"), "A!")])];
    for compression in 0..=2 {
        spec.compression = compression;
        let file = read(&build(&spec), &Limits::default()).expect("reads");
        assert_eq!(
            file.variables[0].missing,
            MissingValues::Discrete(vec![Value::String("NA".into())])
        );
        assert_eq!(
            file.variables[1].missing,
            MissingValues::Discrete(vec![Value::String("x".into()), Value::String("yy".into())])
        );
        assert_eq!(file.value_labels.len(), 2);
        let long = file.labels_for(0).next().expect("long labels");
        assert_eq!(long.labels[0].value, Value::String("hello world".into()));
        assert_eq!(long.labels[1].label, "Not available");
        let short = file.labels_for(1).next().expect("short labels");
        assert_eq!(short.labels[0].value, Value::String("a".into()));
        assert_eq!(file.rows[1][0], Value::String("NA".into()));
    }
}

#[test]
fn missing_range_plus_value_and_formats() {
    let mut v = Var::num("D");
    v.missing = Miss::RangeVal(1.0, 2.0, 99.0);
    v.print = 0x0014_0B00; // DATE11
    let spec = Spec::new(vec![v], vec![vec![Cell::Num(1.0)]]);
    let file = read(&build(&spec), &Limits::default()).expect("reads");
    assert_eq!(
        file.variables[0].missing,
        MissingValues::RangeAndDiscrete {
            low: 1.0,
            high: 2.0,
            value: 99.0
        }
    );
    assert!(file.variables[0].is_date_time());
    assert_eq!(file.variables[0].print_format.type_code, 20);
}

#[test]
fn weight_variable_and_unknown_extension_kept_raw() {
    let mut spec = Spec::new(
        vec![Var::num("A"), Var::text("B", 10), Var::num("W")],
        vec![vec![Cell::Num(1.0), s("x"), Cell::Num(2.0)]],
    );
    spec.weight = 4; // A=1, B=2..3, W=4
    spec.raw_ext = vec![(17, 1, 3, b"abc".to_vec())];
    let file = read(&build(&spec), &Limits::default()).expect("reads");
    assert_eq!(file.header.weight_variable, Some(2));
    assert_eq!(file.unknown_extensions.len(), 1);
    assert_eq!(file.unknown_extensions[0].subtype, 17);
    assert_eq!(file.unknown_extensions[0].data, b"abc");
}

#[test]
fn unknown_case_count_reads_to_end_of_data() {
    for compression in 0..=2 {
        let rows = (0..50).map(|i| vec![Cell::Num(f64::from(i))]).collect();
        let mut spec = Spec::new(vec![Var::num("X")], rows);
        spec.header_ncases = Some(-1);
        spec.compression = compression;
        let file = read(&build(&spec), &Limits::default()).expect("reads");
        assert_eq!(file.header.case_count, None);
        assert_eq!(file.rows.len(), 50);
        assert_eq!(file.rows[49][0], Value::Number(49.0));
    }
}

#[test]
fn header_declaring_more_cases_than_data_is_an_error() {
    let mut spec = Spec::new(vec![Var::num("X")], vec![vec![Cell::Num(1.0)]]);
    spec.header_ncases = Some(5);
    let err = read(&build(&spec), &Limits::default()).expect_err("truncated");
    assert!(matches!(err.kind, ErrorKind::Truncated(_)), "{err}");
}

#[test]
fn enormous_case_count_hits_limit_before_allocating() {
    let mut spec = Spec::new(vec![Var::num("X")], vec![vec![Cell::Num(1.0)]]);
    spec.header_ncases = Some(i32::MAX);
    let err = read(&build(&spec), &Limits::default()).expect_err("limit");
    assert!(matches!(err.kind, ErrorKind::LimitExceeded(_)), "{err}");
}

#[test]
fn zlib_decompressed_size_limit() {
    let rows = (0..1000).map(|i| vec![Cell::Num(f64::from(i))]).collect();
    let mut spec = Spec::new(vec![Var::num("X")], rows);
    spec.compression = 2;
    spec.zlib_block = 4096;
    let limits = Limits {
        max_decompressed_bytes: 100,
        ..Limits::default()
    };
    let err = read(&build(&spec), &limits).expect_err("limit");
    assert!(matches!(err.kind, ErrorKind::LimitExceeded(_)), "{err}");
}

#[test]
fn rejects_non_sav_input() {
    assert!(read(b"", &Limits::default()).is_err());
    assert!(read(&[0u8; 200], &Limits::default()).is_err());
    let mut ebcdic = vec![0u8; 200];
    ebcdic[..4].copy_from_slice(&[0x5b, 0xc6, 0xd3, 0xf2]);
    let err = read(&ebcdic, &Limits::default()).expect_err("ebcdic");
    assert!(matches!(err.kind, ErrorKind::Unsupported(_)));
}

#[test]
fn date_time_helper() {
    use dms_sav::Format;
    let f = |t| Format {
        type_code: t,
        width: 8,
        decimals: 0,
    };
    for t in [20, 21, 22, 23, 24, 25, 28, 29, 30, 38, 39, 40, 41] {
        assert!(f(t).is_date_time(), "{t}");
    }
    for t in [0, 1, 5, 31, 26, 27] {
        assert!(!f(t).is_date_time(), "{t}");
    }
    assert!(f(26).is_calendar_part());
}
