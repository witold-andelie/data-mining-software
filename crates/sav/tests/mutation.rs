//! Mutation tests: truncations and bit flips of valid files must never panic.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use common::{Cell, Miss, Spec, Var, build, s};
use dms_sav::{Limits, read};

fn limits() -> Limits {
    Limits {
        max_cells: 1_000_000,
        max_decompressed_bytes: 1 << 24,
        ..Limits::default()
    }
}

fn specs() -> Vec<Spec> {
    let mut out = Vec::new();
    for big in [false, true] {
        for compression in 0..=2 {
            let mut name = Var::text("NAME", 5);
            name.label = Some("label");
            let mut score = Var::num("SCORE");
            score.long = Some("Long_Score");
            score.missing = Miss::Range(1.0, 2.0);
            let mut long = Var::text("LONGTXT", 12);
            long.missing = Miss::LongStr(vec!["NA"]);
            let vl = Var::text("VL", 300);
            let rows = (0..6)
                .map(|i| {
                    vec![
                        s("abc"),
                        Cell::Num(f64::from(i)),
                        s("hello world"),
                        s(&"q".repeat(40 * i as usize)),
                    ]
                })
                .collect();
            let mut spec = Spec::new(vec![name, score, long, vl], rows);
            spec.big = big;
            spec.compression = compression;
            spec.zlib_block = 50;
            spec.labels = vec![(vec![1], vec![(Cell::Num(1.0), "one")])];
            spec.long_labels = vec![("LONGTXT", vec![("hello world", "hw")])];
            spec.docs = vec!["doc"];
            spec.encoding = Some("UTF-8");
            out.push(spec);
        }
    }
    out
}

#[test]
fn every_truncation_is_ok_or_err() {
    for spec in specs() {
        let bytes = build(&spec);
        assert!(read(&bytes, &limits()).is_ok());
        for len in 0..bytes.len() {
            let _ = read(&bytes[..len], &limits());
        }
    }
}

#[test]
fn bit_flips_are_ok_or_err() {
    // Deterministic xorshift so failures reproduce.
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for spec in specs() {
        let bytes = build(&spec);
        // Every bit of the first 600 bytes (header, dictionary), then random flips.
        for bit in 0..(bytes.len().min(600) * 8) {
            let mut m = bytes.clone();
            m[bit / 8] ^= 1 << (bit % 8);
            let _ = read(&m, &limits());
        }
        for _ in 0..4000 {
            let mut m = bytes.clone();
            for _ in 0..=(next() % 3) {
                let at = (next() % (m.len() as u64 * 8)) as usize;
                m[at / 8] ^= 1 << (at % 8);
            }
            let _ = read(&m, &limits());
        }
    }
}

#[test]
fn random_bytes_after_a_valid_header_do_not_panic() {
    let mut state = 12345u64;
    let mut next = move || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) as u8
    };
    let spec = &specs()[0];
    let bytes = build(spec);
    for _ in 0..500 {
        let mut m = bytes.clone();
        for b in m.iter_mut().skip(176) {
            if next() < 8 {
                *b = next();
            }
        }
        let _ = read(&m, &limits());
    }
}
