//! Hand-encoded fast infoset documents (from the X.891 Annex C rules) and mutation tests.

use dms_strfmt::fastinfoset::{ErrorKind, Limits, decode};

/// `<a x="1">hi<b/></a>`
///
/// - `E0 00 00 01 00`: identification, version 1, padding bit, no optional components
/// - `7C 00 61`: element with attributes, literal name (C.18), local name "a" (C.13, C.22)
/// - `78 00 78`: attribute, literal name (C.17), local name "x"
/// - `00 31`: literal value, not added to the table, UTF-8, length 1 (C.14, C.19, C.23): "1"
/// - `F0`: end of attributes, padding before the first child
/// - `91 68 69`: character chunk, literal, added to the table, UTF-8, length 2 (C.15, C.20, C.24)
/// - `3C 00 62`: element without attributes, literal name "b"
/// - `FF`: end of `b`, end of `a`; `F0`: end of document and padding
const SIMPLE: &[u8] = &[
    0xE0, 0x00, 0x00, 0x01, 0x00, 0x7C, 0x00, 0x61, 0x78, 0x00, 0x78, 0x00, 0x31, 0xF0, 0x91, 0x68,
    0x69, 0x3C, 0x00, 0x62, 0xFF, 0xF0,
];

/// `<a x="">hi<b/><b/>hi</a>` using vocabulary table indexes.
///
/// - `FF` after the attribute name: string index zero, the empty string (C.14, C.26)
/// - `01`: element without attributes named by ELEMENT NAME index 2 ("b") (C.27)
/// - `A0`: character chunk by CONTENT CHARACTER CHUNK index 1 ("hi") (C.15, C.28)
/// - the document terminator follows `a`'s on the same octet (`FF`), so no padding octet
const INDEXED: &[u8] = &[
    0xE0, 0x00, 0x00, 0x01, 0x00, 0x7C, 0x00, 0x61, 0x78, 0x00, 0x78, 0xFF, 0xF0, 0x91, 0x68, 0x69,
    0x3C, 0x00, 0x62, 0xF0, 0x01, 0xF0, 0xA0, 0xFF,
];

#[test]
fn decodes_literals_attributes_and_text() {
    let document = decode(SIMPLE, &Limits::default()).unwrap();
    assert_eq!(
        document.to_xml(),
        r#"<?xml version="1.0"?><a x="1">hi<b/></a>"#
    );
    let root = document.root().unwrap();
    assert_eq!(root.attribute("x"), Some("1"));
    assert_eq!(root.text(), "hi");
    assert_eq!(root.child("b").map(|b| b.children.len()), Some(0));
}

#[test]
fn resolves_vocabulary_table_indexes() {
    let document = decode(INDEXED, &Limits::default()).unwrap();
    assert_eq!(
        document.to_xml(),
        r#"<?xml version="1.0"?><a x="">hi<b/><b/>hi</a>"#
    );
}

#[test]
fn accepts_a_leading_xml_declaration() {
    let mut input = b"<?xml encoding='finf'?>".to_vec();
    input.extend_from_slice(SIMPLE);
    let document = decode(&input, &Limits::default()).unwrap();
    assert_eq!(
        document.xml_declaration.as_deref(),
        Some("<?xml encoding='finf'?>")
    );
    assert_eq!(document.root().map(|a| a.attribute("x")), Some(Some("1")));
}

#[test]
fn reports_errors_with_positions() {
    let err = decode(b"<?xml version=\"1.0\"?><a/>", &Limits::default()).unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFastInfoset);

    let mut trailing = SIMPLE.to_vec();
    trailing.push(0);
    assert_eq!(
        decode(&trailing, &Limits::default()).unwrap_err().kind,
        ErrorKind::TrailingData
    );

    // ELEMENT NAME index 3 does not exist yet.
    let mut bad_index = INDEXED.to_vec();
    bad_index[20] = 0x02;
    let err = decode(&bad_index, &Limits::default()).unwrap_err();
    assert_eq!(err.offset, 20);
    assert!(matches!(
        err.kind,
        ErrorKind::IndexOutOfRange { index: 3, .. }
    ));
}

#[test]
fn enforces_the_depth_limit() {
    let limits = Limits { max_depth: 1 };
    let err = decode(SIMPLE, &limits).unwrap_err();
    assert_eq!(err.kind, ErrorKind::LimitExceeded("element nesting depth"));
}

/// Never-crash: every truncation and every single-bit flip returns a result.
#[test]
fn mutations_never_panic() {
    for vector in [SIMPLE, INDEXED] {
        for length in 0..vector.len() {
            let _ = decode(&vector[..length], &Limits::default());
        }
        for index in 0..vector.len() {
            for bit in 0..8 {
                let mut mutated = vector.to_vec();
                mutated[index] ^= 1 << bit;
                let _ = decode(&mutated, &Limits::default());
            }
        }
    }
}

/// A deep document is decoded without recursion.
#[test]
fn deep_nesting_uses_no_call_stack() {
    let depth = 100_000;
    let mut input = vec![0xE0, 0x00, 0x00, 0x01, 0x00, 0x3C, 0x00, 0x62];
    // Every further <b/> refers to ELEMENT NAME index 1: '0' '0' C.27 '0' '00000'.
    input.extend(std::iter::repeat_n(0x00, depth - 1));
    // Close all elements and the document: 'F' nibbles, then padding.
    let terminators = depth + 1;
    input.extend(std::iter::repeat_n(0xFF, terminators / 2));
    if terminators % 2 == 1 {
        input.push(0xF0);
    }
    let limits = Limits { max_depth: depth };
    let document = decode(&input, &limits).unwrap();
    assert!(document.root().is_some());
}
