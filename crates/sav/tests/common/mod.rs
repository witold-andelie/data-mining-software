//! A small `.sav` encoder for building synthetic test files (written from the format
//! description, independent of the reader).
#![allow(dead_code, clippy::type_complexity)]

use flate2::{Compression, write::ZlibEncoder};
use std::io::Write;

#[derive(Clone, Debug)]
pub enum Cell {
    Num(f64),
    Sys,
    Str(String),
}

pub fn s(text: &str) -> Cell {
    Cell::Str(text.to_string())
}

#[derive(Clone, Debug)]
pub enum Miss {
    None,
    Num(Vec<f64>),
    Range(f64, f64),
    RangeVal(f64, f64, f64),
    /// Short strings: written in the variable record.
    Str(Vec<&'static str>),
    /// Long strings: written in the subtype 22 record.
    LongStr(Vec<&'static str>),
}

#[derive(Clone, Debug)]
pub struct Var {
    pub name: &'static str,
    pub long: Option<&'static str>,
    pub width: usize,
    pub label: Option<&'static str>,
    pub missing: Miss,
    pub print: i32,
    pub measure: i32,
    pub align: i32,
    pub dwidth: i32,
}

impl Var {
    pub fn num(name: &'static str) -> Self {
        Self {
            name,
            long: None,
            width: 0,
            label: None,
            missing: Miss::None,
            print: 0x0005_0802, // F8.2
            measure: 3,
            align: 1,
            dwidth: 8,
        }
    }
    pub fn text(name: &'static str, width: usize) -> Self {
        Self {
            width,
            print: 0x0001_0000 | ((width.min(255) as i32) << 8),
            measure: 1,
            align: 0,
            ..Self::num(name)
        }
    }
}

#[derive(Clone, Debug)]
pub struct Spec {
    pub big: bool,
    /// 0 none, 1 bytecode, 2 zlib
    pub compression: i32,
    pub vars: Vec<Var>,
    pub rows: Vec<Vec<Cell>>,
    /// (logical variable indices, labels)
    pub labels: Vec<(Vec<usize>, Vec<(Cell, &'static str)>)>,
    /// (variable name, [(value, label)])
    pub long_labels: Vec<(&'static str, Vec<(&'static str, &'static str)>)>,
    pub docs: Vec<&'static str>,
    pub encoding: Option<&'static str>,
    pub char_code: i32,
    pub header_ncases: Option<i32>,
    pub zlib_block: usize,
    pub extensions: bool,
    /// Extra raw extension records: (subtype, size, count, data).
    pub raw_ext: Vec<(i32, i32, i32, Vec<u8>)>,
    pub weight: i32,
}

impl Spec {
    pub fn new(vars: Vec<Var>, rows: Vec<Vec<Cell>>) -> Self {
        Self {
            big: false,
            compression: 0,
            vars,
            rows,
            labels: Vec::new(),
            long_labels: Vec::new(),
            docs: Vec::new(),
            encoding: None,
            char_code: 1252,
            header_ncases: None,
            zlib_block: 64,
            extensions: true,
            raw_ext: Vec::new(),
            weight: 0,
        }
    }
}

enum Elem {
    Num(f64),
    Sys,
    Raw([u8; 8]),
}

struct W {
    big: bool,
    out: Vec<u8>,
}

impl W {
    fn i32(&mut self, v: i32) {
        let b = if self.big {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        };
        self.out.extend_from_slice(&b);
    }
    fn i64(&mut self, v: i64) {
        let b = if self.big {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        };
        self.out.extend_from_slice(&b);
    }
    fn f64(&mut self, v: f64) {
        let b = f64b(self.big, v);
        self.out.extend_from_slice(&b);
    }
    fn bytes(&mut self, b: &[u8]) {
        self.out.extend_from_slice(b);
    }
    fn padded(&mut self, b: &[u8], len: usize) {
        let mut v = b.to_vec();
        v.resize(len, b' ');
        self.out.extend_from_slice(&v);
    }
}

fn f64b(big: bool, v: f64) -> [u8; 8] {
    if big {
        v.to_be_bytes()
    } else {
        v.to_le_bytes()
    }
}

fn enc(spec: &Spec, text: &str) -> Vec<u8> {
    if spec
        .encoding
        .is_some_and(|e| e.eq_ignore_ascii_case("utf-8"))
    {
        text.as_bytes().to_vec()
    } else {
        text.chars().map(|c| c as u32 as u8).collect()
    }
}

/// Physical variable: (name, width, label, missing, print, write)
struct Phys {
    name: String,
    width: usize,
    var: usize,
    first: bool,
}

fn physical(spec: &Spec) -> Vec<Phys> {
    let mut out = Vec::new();
    for (i, v) in spec.vars.iter().enumerate() {
        if v.width > 255 {
            let n = v.width.div_ceil(252);
            for k in 0..n {
                let width = if k + 1 < n {
                    255
                } else {
                    v.width - (n - 1) * 252
                };
                let name = if k == 0 {
                    v.name.to_string()
                } else {
                    format!("{}_{k}", v.name)
                };
                out.push(Phys {
                    name,
                    width,
                    var: i,
                    first: k == 0,
                });
            }
        } else {
            out.push(Phys {
                name: v.name.to_string(),
                width: v.width,
                var: i,
                first: true,
            });
        }
    }
    out
}

fn records_of(width: usize) -> usize {
    if width == 0 { 1 } else { width.div_ceil(8) }
}

pub fn build(spec: &Spec) -> Vec<u8> {
    let mut w = W {
        big: spec.big,
        out: Vec::new(),
    };
    let phys = physical(spec);
    // Header.
    w.bytes(if spec.compression == 2 {
        b"$FL3"
    } else {
        b"$FL2"
    });
    w.padded(b"@(#) SPSS DATA FILE synthetic test encoder", 60);
    w.i32(2);
    w.i32(phys.iter().map(|p| records_of(p.width) as i32).sum());
    w.i32(spec.compression);
    w.i32(spec.weight);
    w.i32(spec.header_ncases.unwrap_or(spec.rows.len() as i32));
    w.f64(100.0);
    w.padded(b"01 Feb 26", 9);
    w.padded(b"12:34:56", 8);
    w.padded(&enc(spec, "test file"), 64);
    w.bytes(&[0, 0, 0]);
    // Variable records; remember first record index of each logical variable.
    let mut first_record = vec![0usize; spec.vars.len()];
    let mut rec = 0usize;
    for p in &phys {
        let v = &spec.vars[p.var];
        if p.first {
            first_record[p.var] = rec + 1;
        }
        w.i32(2);
        w.i32(p.width as i32);
        let label = if p.first { v.label } else { None };
        w.i32(i32::from(label.is_some()));
        let (code, vals): (i32, Vec<[u8; 8]>) = if !p.first {
            (0, vec![])
        } else {
            match &v.missing {
                Miss::None | Miss::LongStr(_) => (0, vec![]),
                Miss::Num(x) => (x.len() as i32, x.iter().map(|x| f64b(w.big, *x)).collect()),
                Miss::Range(a, b) => (-2, vec![f64b(w.big, *a), f64b(w.big, *b)]),
                Miss::RangeVal(a, b, c) => {
                    (-3, vec![f64b(w.big, *a), f64b(w.big, *b), f64b(w.big, *c)])
                }
                Miss::Str(x) => (
                    x.len() as i32,
                    x.iter()
                        .map(|t| {
                            let mut a = [b' '; 8];
                            for (d, s) in a.iter_mut().zip(t.bytes()) {
                                *d = s;
                            }
                            a
                        })
                        .collect(),
                ),
            }
        };
        w.i32(code);
        w.i32(v.print);
        w.i32(v.print);
        w.padded(p.name.as_bytes(), 8);
        if let Some(l) = label {
            let b = enc(spec, l);
            w.i32(b.len() as i32);
            w.padded(&b, b.len().div_ceil(4) * 4);
        }
        for m in vals {
            w.bytes(&m);
        }
        rec += 1;
        for _ in 1..records_of(p.width) {
            w.i32(2);
            w.i32(-1);
            w.i32(0);
            w.i32(0);
            w.i32(0);
            w.i32(0);
            w.padded(b"", 8);
            rec += 1;
        }
    }
    // Value labels.
    for (vars, labels) in &spec.labels {
        w.i32(3);
        w.i32(labels.len() as i32);
        for (value, label) in labels {
            match value {
                Cell::Num(x) => w.f64(*x),
                Cell::Sys => w.f64(-f64::MAX),
                Cell::Str(t) => w.padded(&enc(spec, t), 8),
            }
            let b = enc(spec, label);
            w.bytes(&[b.len() as u8]);
            w.padded(&b, (b.len() + 1).div_ceil(8) * 8 - 1);
        }
        w.i32(4);
        w.i32(vars.len() as i32);
        for v in vars {
            w.i32(first_record[*v] as i32);
        }
    }
    if !spec.docs.is_empty() {
        w.i32(6);
        w.i32(spec.docs.len() as i32);
        for d in &spec.docs {
            w.padded(&enc(spec, d), 80);
        }
    }
    // Extensions.
    if spec.extensions {
        ext(&mut w, 3, 4, 8, |w| {
            for v in [
                1,
                2,
                3,
                -1,
                1,
                1,
                if spec.big { 1 } else { 2 },
                spec.char_code,
            ] {
                w.i32(v);
            }
        });
        ext(&mut w, 4, 8, 3, |w| {
            w.f64(-f64::MAX);
            w.f64(f64::MAX);
            w.f64(-f64::MAX);
        });
        ext(&mut w, 11, 4, (phys.len() * 3) as i32, |w| {
            for p in &phys {
                let v = &spec.vars[p.var];
                w.i32(v.measure);
                w.i32(v.dwidth);
                w.i32(v.align);
            }
        });
    }
    let longs: Vec<&Var> = spec.vars.iter().filter(|v| v.long.is_some()).collect();
    if !longs.is_empty() {
        let mut data = Vec::new();
        for (i, v) in longs.iter().enumerate() {
            if i > 0 {
                data.push(9);
            }
            data.extend_from_slice(format!("{}={}", v.name, v.long.unwrap_or("")).as_bytes());
        }
        raw_ext(&mut w, 13, 1, &data);
    }
    let vls: Vec<&Var> = spec.vars.iter().filter(|v| v.width > 255).collect();
    if !vls.is_empty() {
        let mut data = Vec::new();
        for v in vls {
            data.extend_from_slice(format!("{}={:05}", v.name, v.width).as_bytes());
            data.extend_from_slice(&[0, 9]);
        }
        raw_ext(&mut w, 14, 1, &data);
    }
    let enc_name = spec.encoding;
    if let Some(name) = enc_name {
        raw_ext(&mut w, 20, 1, name.as_bytes());
    }
    if !spec.long_labels.is_empty() {
        let mut d = W {
            big: spec.big,
            out: Vec::new(),
        };
        for (name, labels) in &spec.long_labels {
            let width = spec
                .vars
                .iter()
                .find(|v| v.name == *name)
                .map_or(9, |v| v.width);
            d.i32(name.len() as i32);
            d.bytes(name.as_bytes());
            d.i32(width as i32);
            d.i32(labels.len() as i32);
            for (value, label) in labels {
                d.i32(width as i32);
                d.padded(&enc(spec, value), width);
                let b = enc(spec, label);
                d.i32(b.len() as i32);
                d.bytes(&b);
            }
        }
        raw_ext(&mut w, 21, 1, &d.out);
    }
    let lm: Vec<&Var> = spec
        .vars
        .iter()
        .filter(|v| matches!(v.missing, Miss::LongStr(_)))
        .collect();
    if !lm.is_empty() {
        let mut d = W {
            big: spec.big,
            out: Vec::new(),
        };
        for v in lm {
            if let Miss::LongStr(vals) = &v.missing {
                d.i32(v.name.len() as i32);
                d.bytes(v.name.as_bytes());
                d.bytes(&[vals.len() as u8]);
                for t in vals {
                    d.i32(8);
                    d.padded(t.as_bytes(), 8);
                }
            }
        }
        raw_ext(&mut w, 22, 1, &d.out);
    }
    for (sub, size, count, data) in &spec.raw_ext {
        w.i32(7);
        w.i32(*sub);
        w.i32(*size);
        w.i32(*count);
        w.bytes(data);
    }
    w.i32(999);
    w.i32(0);

    // Data.
    let mut elems: Vec<Elem> = Vec::new();
    for row in &spec.rows {
        for (vi, v) in spec.vars.iter().enumerate() {
            let cell = row.get(vi).cloned().unwrap_or(Cell::Sys);
            match (v.width, cell) {
                (0, Cell::Num(x)) => elems.push(Elem::Num(x)),
                (0, _) => elems.push(Elem::Sys),
                (width, cell) => {
                    let bytes = match cell {
                        Cell::Str(t) => enc(spec, &t),
                        _ => vec![],
                    };
                    if width > 255 {
                        let n = width.div_ceil(252);
                        for k in 0..n {
                            let segw = if k + 1 < n {
                                255
                            } else {
                                width - (n - 1) * 252
                            };
                            let part = bytes.get(k * 255..).unwrap_or_default();
                            let part = part.get(..part.len().min(255)).unwrap_or_default();
                            push_string(&mut elems, part, segw);
                        }
                    } else {
                        push_string(&mut elems, &bytes, width);
                    }
                }
            }
        }
    }
    match spec.compression {
        0 => {
            for e in &elems {
                w.bytes(&elem_bytes(e, spec.big));
            }
        }
        1 => {
            let stream = bytecode(&elems, spec.big);
            w.bytes(&stream);
        }
        _ => {
            let stream = bytecode(&elems, spec.big);
            let start = w.out.len();
            let blocks: Vec<Vec<u8>> = stream
                .chunks(spec.zlib_block.max(1))
                .map(|c| {
                    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
                    let _ = e.write_all(c);
                    e.finish().unwrap_or_default()
                })
                .collect();
            let total: usize = blocks.iter().map(Vec::len).sum();
            let trailer_ofs = start + 24 + total;
            let trailer_len = 24 + 24 * blocks.len();
            w.i64(start as i64);
            w.i64(trailer_ofs as i64);
            w.i64(trailer_len as i64);
            for b in &blocks {
                w.bytes(b);
            }
            w.i64(-100);
            w.i64(0);
            w.i32(spec.zlib_block as i32);
            w.i32(blocks.len() as i32);
            let mut uofs = start;
            let mut cofs = start + 24;
            for (i, b) in blocks.iter().enumerate() {
                let usize_ = stream
                    .chunks(spec.zlib_block.max(1))
                    .nth(i)
                    .map_or(0, <[u8]>::len);
                w.i64(uofs as i64);
                w.i64(cofs as i64);
                w.i32(usize_ as i32);
                w.i32(b.len() as i32);
                uofs += usize_;
                cofs += b.len();
            }
        }
    }
    w.out
}

fn push_string(elems: &mut Vec<Elem>, bytes: &[u8], width: usize) {
    let mut v = bytes
        .get(..bytes.len().min(width))
        .unwrap_or_default()
        .to_vec();
    v.resize(records_of(width) * 8, b' ');
    for c in v.chunks(8) {
        let mut a = [b' '; 8];
        for (d, s) in a.iter_mut().zip(c) {
            *d = *s;
        }
        elems.push(Elem::Raw(a));
    }
}

fn elem_bytes(e: &Elem, big: bool) -> [u8; 8] {
    match e {
        Elem::Num(x) => f64b(big, *x),
        Elem::Sys => f64b(big, -f64::MAX),
        Elem::Raw(r) => *r,
    }
}

fn bytecode(elems: &[Elem], big: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut cmds: Vec<u8> = Vec::new();
    let mut data: Vec<u8> = Vec::new();
    let flush = |out: &mut Vec<u8>, cmds: &mut Vec<u8>, data: &mut Vec<u8>| {
        cmds.resize(8, 0);
        out.extend_from_slice(cmds);
        out.extend_from_slice(data);
        cmds.clear();
        data.clear();
    };
    for e in elems {
        let code = match e {
            Elem::Num(x) if x.fract() == 0.0 && (-99.0..=151.0).contains(x) => (*x + 100.0) as u8,
            Elem::Sys => 255,
            Elem::Raw(r) if *r == [b' '; 8] => 254,
            other => {
                data.extend_from_slice(&elem_bytes(other, big));
                253
            }
        };
        cmds.push(code);
        if cmds.len() == 8 {
            flush(&mut out, &mut cmds, &mut data);
        }
    }
    cmds.push(252);
    flush(&mut out, &mut cmds, &mut data);
    out
}

fn ext(w: &mut W, subtype: i32, size: i32, count: i32, body: impl FnOnce(&mut W)) {
    w.i32(7);
    w.i32(subtype);
    w.i32(size);
    w.i32(count);
    body(w);
}

fn raw_ext(w: &mut W, subtype: i32, size: i32, data: &[u8]) {
    w.i32(7);
    w.i32(subtype);
    w.i32(size);
    w.i32(data.len() as i32);
    w.bytes(data);
}
