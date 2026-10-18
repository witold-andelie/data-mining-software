//! Real-file corpus: every pinned `.str` decodes (`cargo test -p dms-strfmt --features corpus`,
//! after `cargo xtask corpus sync ...`). The files are IBM demo streams, the team's streams and
//! differential probe output; they never enter git.
#![cfg(feature = "corpus")]

use dms_strfmt::{ArchiveLimits, read_archive};
use std::path::{Path, PathBuf};

fn corpus_streams() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    let manifest = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../xtask/corpus.tsv"),
    )
    .expect("xtask/corpus.tsv");
    let streams: Vec<PathBuf> = manifest
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let label = fields.next()?;
            let path = fields.next()?;
            path.ends_with(".str").then(|| root.join(label).join(path))
        })
        .collect();
    assert!(!streams.is_empty(), "no .str files in the corpus manifest");
    for path in &streams {
        assert!(
            path.is_file(),
            "{} missing: run `cargo xtask corpus sync`",
            path.display()
        );
    }
    streams
}

#[test]
fn every_corpus_stream_decodes() {
    let mut failures = Vec::new();
    for path in corpus_streams() {
        let bytes = std::fs::read(&path).unwrap();
        match read_archive(&bytes, &ArchiveLimits::default()) {
            Ok(archive) => {
                let root = archive
                    .document
                    .root()
                    .map(|root| root.name.local.to_string());
                if root.as_deref() != Some("clementineStream") {
                    failures.push(format!("{}: root element {root:?}", path.display()));
                }
            }
            Err(error) => failures.push(format!("{}: {error}", path.display())),
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Never-crash on real data: truncations and bit flips of the stream document.
#[test]
fn corpus_mutations_never_panic() {
    for path in corpus_streams().into_iter().take(8) {
        let bytes = std::fs::read(&path).unwrap();
        let archive = read_archive(&bytes, &ArchiveLimits::default()).unwrap();
        let _ = archive;
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        let mut document = Vec::new();
        for index in 0..zip.len() {
            let mut file = zip.by_index(index).unwrap();
            if !file.name().unwrap().starts_with("data/") {
                std::io::Read::read_to_end(&mut file, &mut document).unwrap();
            }
        }
        let step = (document.len() / 64).max(1);
        for length in (0..document.len()).step_by(step) {
            let _ = dms_strfmt::fastinfoset::decode(&document[..length], &Default::default());
        }
        for index in (0..document.len()).step_by(step) {
            let mut mutated = document.clone();
            mutated[index] ^= 0x55;
            let _ = dms_strfmt::fastinfoset::decode(&mutated, &Default::default());
        }
    }
}

/// `ts_arima.str` `data/0003.dat`: length-prefixed ZIP with data descriptors. Python zipfile
/// rejects the extra field; walking local headers yields the per-target PMML.
#[test]
fn time_series_nugget_zip_walks() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/models_next/ts_arima.str");
    let bytes = std::fs::read(&path).expect("corpus/models_next/ts_arima.str");
    let archive = read_archive(&bytes, &ArchiveLimits::default()).expect("archive");
    let attachment = archive
        .attachments
        .iter()
        .find(|attachment| attachment.name == "data/0003.dat")
        .expect("data/0003.dat");
    let members =
        dms_strfmt::read_length_prefixed_zip(&attachment.bytes, &ArchiveLimits::default())
            .expect("walk");
    let names: Vec<&str> = members.iter().map(|m| m.name.as_str()).collect();
    assert!(names.contains(&"0.xml"), "{names:?}");
    assert!(names.contains(&"1.xml"), "{names:?}");
    assert!(names.contains(&"datamodel.xml"), "{names:?}");
    let pmml = members.iter().find(|m| m.name == "0.xml").expect("0.xml");
    let text = std::str::from_utf8(&pmml.bytes).expect("utf8");
    assert!(text.contains("TimeSeriesModel"), "{text}");
    assert!(text.contains("constantTerm"), "{text}");
}
