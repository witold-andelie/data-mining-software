//! Every pinned stream: import -> TOML -> load, including every attachment and nested node.
#![cfg(feature = "corpus")]
use dms_stream::{
    Diagram, Stream, StreamDoc,
    native::{from_toml, to_toml},
    settings_xml::{normalize, xml_roundtrips},
};
use dms_strfmt::{
    ArchiveLimits,
    fastinfoset::{Element, Node},
    read_archive,
};
use std::path::Path;

fn elements_equal(a: Option<&Element>, b: Option<&Element>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            let a = normalize(a);
            let b = normalize(b);
            let mut pending = vec![(&a, &b)];
            while let Some((a, b)) = pending.pop() {
                if a.name != b.name
                    || a.attributes != b.attributes
                    || a.namespace_declarations != b.namespace_declarations
                    || a.children.len() != b.children.len()
                {
                    return false;
                }
                for (a, b) in a.children.iter().zip(&b.children) {
                    match (a, b) {
                        (Node::Element(a), Node::Element(b)) => pending.push((a, b)),
                        _ if a == b => {}
                        _ => return false,
                    }
                }
            }
            true
        }
        _ => false,
    }
}
fn streams_equal(a: &Stream, b: &Stream) -> bool {
    if a.file_version != b.file_version
        || a.save_version != b.save_version
        || !elements_equal(a.properties.as_ref(), b.properties.as_ref())
    {
        return false;
    }
    let mut pending: Vec<(&Diagram, &Diagram)> = vec![(&a.diagram, &b.diagram)];
    while let Some((a, b)) = pending.pop() {
        if a.links != b.links
            || a.terminal_order != b.terminal_order
            || a.nodes.len() != b.nodes.len()
        {
            return false;
        }
        for (a, b) in a.nodes.iter().zip(&b.nodes) {
            if a.uid != b.uid
                || a.class != b.class
                || a.extension != b.extension
                || a.model_builder != b.model_builder
                || a.name != b.name
                || a.label != b.label
                || a.position != b.position
                || a.properties_attachment != b.properties_attachment
                || a.containers != b.containers
                || a.result_items.len() != b.result_items.len()
                || !elements_equal(a.settings.as_ref(), b.settings.as_ref())
                || !elements_equal(a.binning_output.as_ref(), b.binning_output.as_ref())
            {
                return false;
            }
            for (a, b) in a.result_items.iter().zip(&b.result_items) {
                if !elements_equal(Some(a), Some(b)) {
                    return false;
                }
            }
            match (&a.subdiagram, &b.subdiagram) {
                (Some(a), Some(b)) => pending.push((a, b)),
                (None, None) => {}
                _ => return false,
            }
        }
    }
    true
}
#[test]
fn every_pinned_str_survives_native_roundtrip() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest = std::fs::read_to_string(root.join("xtask/corpus.tsv")).expect("corpus manifest");
    let (mut files, mut xml_settings, mut tree_settings) = (0usize, 0usize, 0usize);
    for line in manifest
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
    {
        let fields: Vec<&str> = line.split('\t').collect();
        let [label, path, _] = fields.as_slice() else {
            panic!("malformed manifest line");
        };
        if !path.to_ascii_lowercase().ends_with(".str") {
            continue;
        }
        let bytes =
            std::fs::read(root.join("corpus").join(label).join(path)).expect("read pinned stream");
        let archive =
            read_archive(&bytes, &ArchiveLimits::default()).expect("decode pinned stream");
        let direct = Stream::from_document(&archive.document).expect("interpret pinned stream");
        let imported = StreamDoc::import_str(
            &bytes,
            Path::new(path).file_name().unwrap().to_str().unwrap(),
        )
        .unwrap_or_else(|_| panic!("{label}/{path}: import failed"));
        assert!(
            streams_equal(&direct, &imported.stream),
            "{label}/{path}: import changed Stream"
        );
        for e in direct
            .properties
            .iter()
            .chain(direct.all_nodes().into_iter().flat_map(|n| {
                n.settings
                    .iter()
                    .chain(n.binning_output.iter())
                    .chain(&n.result_items)
            }))
        {
            if xml_roundtrips(e) {
                xml_settings += 1;
            } else {
                tree_settings += 1;
            }
        }
        let encoded = to_toml(&imported).unwrap_or_else(|_| panic!("{label}/{path}: write failed"));
        let decoded = from_toml(&encoded).unwrap_or_else(|_| panic!("{label}/{path}: read failed"));
        assert!(
            streams_equal(&direct, &decoded.stream),
            "{label}/{path}: native roundtrip changed Stream"
        );
        assert!(
            imported.attachments == decoded.attachments,
            "{label}/{path}: native attachments changed"
        );
        for a in &archive.attachments {
            assert!(
                decoded
                    .attachments
                    .get(&a.name)
                    .is_some_and(|b| b.as_ref() == a.bytes),
                "{label}/{path}: imported attachment changed"
            );
        }
        assert!(
            imported.meta == decoded.meta,
            "{label}/{path}: metadata changed"
        );
        let encoded_again =
            to_toml(&decoded).unwrap_or_else(|_| panic!("{label}/{path}: second write failed"));
        assert!(
            encoded == encoded_again,
            "{label}/{path}: serialization is not byte deterministic"
        );
        files += 1;
    }
    println!("files={files} xml_settings={xml_settings} settings_tree={tree_settings}");
    assert!(files > 0);
}
