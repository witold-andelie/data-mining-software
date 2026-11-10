//! Every pinned instance of the 15 editable node kinds; no data source is executed.
#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "corpus assertions"
)]
use dms_exec::schema::{self, ParamSource};
use dms_stream::{StreamDoc, settings_xml::normalize};
use std::{collections::BTreeMap, path::Path};

#[test]
fn every_parameter_reads_and_writes_without_changing_the_import() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let manifest = std::fs::read_to_string(root.join("xtask/corpus.tsv")).unwrap();
    let mut counts: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let mut errors = Vec::new();
    let mut files = 0usize;
    for line in manifest
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let columns: Vec<_> = line.split('\t').collect();
        let label = columns[0];
        if label == "database" {
            continue;
        } // Explicit task restriction: this label is excluded.
        let relative = columns[1];
        if !relative.to_ascii_lowercase().ends_with(".str") {
            continue;
        }
        let path = root.join("corpus").join(label).join(relative);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let doc = StreamDoc::import_str(&bytes, relative)
            .unwrap_or_else(|e| panic!("{label}/{relative}: {e}"));
        files += 1;
        for node in doc.stream.all_nodes() {
            let identifier = node.type_identifier();
            if identifier.starts_with("Apply") || node.model_builder.is_some() {
                continue;
            }
            let Some(catalog) = dms_catalog::by_modeler_identifier(&identifier) else {
                continue;
            };
            let Some(specs) = schema::schema(catalog.id) else {
                continue;
            };
            let count = counts.entry(catalog.id).or_default();
            count.0 += 1;
            for spec in specs {
                count.1 += 1;
                let result: Result<(), String> = (|| {
                    match spec.source {
                        ParamSource::Properties => {
                            let value = schema::read(node.settings.as_ref(), spec)?;
                            let written = schema::write(node.settings.as_ref(), spec, &value)?;
                            let original = node.settings.as_ref().map(normalize);
                            // An absent properties root has the same empty parameter state.
                            if let Some(original) = original {
                                if normalize(&written) != original {
                                    return Err(
                                        "settings changed after writing their own value".into()
                                    );
                                }
                            } else if !written.children.is_empty() {
                                return Err("absent settings gained content".into());
                            }
                        }
                        ParamSource::Attachment => {
                            let bytes = node
                                .properties_attachment
                                .as_ref()
                                .and_then(|p| doc.attachments.get(p))
                                .map(|v| v.as_ref());
                            let value = schema::read_attachment(bytes, spec)?;
                            let bytes = bytes.ok_or("missing attachment")?;
                            if schema::write_attachment(bytes, spec, &value)? != bytes {
                                return Err(
                                    "attachment bytes changed after writing their own value".into(),
                                );
                            }
                        }
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    errors.push(format!(
                        "{label}/{relative} {} {}.{}: {error}",
                        node.uid, catalog.id, spec.key
                    ));
                }
            }
        }
    }
    println!("files={files} (database excluded by task instruction)");
    for (id, (instances, parameters)) in &counts {
        println!(
            "{id}: instances={instances} parameters={} values={parameters}",
            schema::schema(id).unwrap().len()
        );
    }
    for error in &errors {
        eprintln!("{error}");
    }
    assert_eq!(
        counts.len(),
        15,
        "all 15 node kinds must occur in the pinned corpus"
    );
    assert!(
        errors.is_empty(),
        "{} parameter failures (listed above)",
        errors.len()
    );
}
