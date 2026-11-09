//! Read-only summary coverage for locally available oracle exports.
#![cfg(feature = "corpus")]

use dms_pmml::summary::summarize;
use dms_pmml::{Model, Pmml};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[test]
fn summaries_cover_oracle_models() -> Result<(), Box<dyn std::error::Error>> {
    let oracle = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../oracle");
    let mut counts = BTreeMap::<String, usize>::new();
    let mut skipped = 0usize;
    let mut parsed = 0usize;
    // oracle/<label>/<flow>/models/*.xml: discover both directory levels without a name list.
    for label in fs::read_dir(oracle)? {
        let label = label?.path();
        if !label.is_dir() {
            continue;
        }
        for flow in fs::read_dir(label)? {
            let models = flow?.path().join("models");
            if !models.is_dir() {
                continue;
            }
            for file in fs::read_dir(models)? {
                let path = file?.path();
                if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("xml") {
                    continue;
                }
                let xml = fs::read_to_string(path)?;
                let pmml = match Pmml::parse(&xml) {
                    Ok(pmml) => pmml,
                    Err(_) => {
                        skipped = skipped.saturating_add(1);
                        continue;
                    }
                };
                let summary = summarize(&pmml);
                assert!(!summary.kind.is_empty(), "summary kind must not be empty");
                if !matches!(pmml.model, Model::Unsupported(_)) {
                    assert!(
                        !summary.sections.is_empty(),
                        "supported model must have a section"
                    );
                }
                let count = counts.entry(summary.kind).or_default();
                *count = count.saturating_add(1);
                parsed = parsed.saturating_add(1);
            }
        }
    }
    for (kind, count) in counts {
        println!("{kind}: {count}");
    }
    println!("Parsed files: {parsed}");
    println!("Skipped files: {skipped}");
    assert!(parsed > 0, "no oracle PMML files were parsed");
    Ok(())
}
