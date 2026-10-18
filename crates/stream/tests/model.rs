//! Stream model over synthetic documents (no Modeler files needed).

use dms_stream::{NodeRef, Stream, StreamError};
use dms_strfmt::fastinfoset::{Attribute, Document, Element, Node, QualifiedName};
use std::sync::Arc;

fn name(local: &str) -> QualifiedName {
    QualifiedName {
        prefix: None,
        namespace: None,
        local: Arc::from(local),
    }
}

fn el(local: &str, attributes: &[(&str, &str)], children: Vec<Node>) -> Node {
    Node::Element(Element {
        name: name(local),
        namespace_declarations: Vec::new(),
        attributes: attributes
            .iter()
            .map(|(key, value)| Attribute {
                name: name(key),
                value: Arc::from(*value),
            })
            .collect(),
        children,
    })
}

fn text(value: &str) -> Node {
    Node::Text(Arc::from(value))
}

fn node(id: &str, uid: &str, class: &str, extra: &[(&str, &str)], children: Vec<Node>) -> Node {
    let mut attributes = vec![("id", id), ("uid", uid), ("class", class)];
    attributes.extend_from_slice(extra);
    el("node", &attributes, children)
}

fn document(diagram: Node) -> Document {
    Document {
        children: vec![el(
            "clementineStream",
            &[("version", "19.0")],
            vec![el("stream", &[("saveVersion", "19.0")], vec![diagram])],
        )],
        ..Document::default()
    }
}

fn settings(label: &str, x: &str, y: &str) -> Node {
    el(
        "properties",
        &[],
        vec![
            el("label", &[], vec![text(label)]),
            el(
                "position",
                &[],
                vec![el("x", &[], vec![text(x)]), el("y", &[], vec![text(y)])],
            ),
        ],
    )
}

#[test]
fn resolves_links_to_uids_and_keeps_unresolved_ids() {
    let diagram = el(
        "diagram",
        &[],
        vec![
            el(
                "nodes",
                &[],
                vec![
                    node(
                        "0",
                        "uidA",
                        "x.y.DelimitedFileImportNode",
                        &[],
                        vec![settings("Source", "10", "20")],
                    ),
                    node("1", "uidB", "x.y.TableNode", &[], vec![]),
                ],
            ),
            el(
                "links",
                &[],
                vec![
                    el("link", &[("srcId", "0"), ("dstId", "1")], vec![]),
                    el("link", &[("srcId", "1"), ("dstId", "9")], vec![]),
                ],
            ),
            el(
                "terminalNodeOrder",
                &[],
                vec![el("node", &[("id", "1")], vec![])],
            ),
        ],
    );
    let stream = Stream::from_document(&document(diagram)).unwrap();
    assert_eq!(stream.file_version.as_deref(), Some("19.0"));
    let nodes = &stream.diagram.nodes;
    assert_eq!(nodes[0].type_identifier(), "DelimitedFileImportNode");
    assert_eq!(nodes[0].label.as_deref(), Some("Source"));
    assert_eq!(nodes[0].position, Some((10, 20)));
    assert_eq!(stream.diagram.links[0].from, NodeRef::Uid("uidA".into()));
    assert_eq!(stream.diagram.links[0].to, NodeRef::Uid("uidB".into()));
    assert_eq!(stream.diagram.links[1].to, NodeRef::Unresolved("9".into()));
    assert_eq!(
        stream.diagram.terminal_order,
        vec![NodeRef::Uid("uidB".into())]
    );
}

#[test]
fn identifies_extension_nodes_containers_and_supernodes() {
    let inner = el(
        "diagram",
        &[],
        vec![el(
            "nodes",
            &[],
            vec![node("0", "uidInner", "x.y.DeriveNode", &[], vec![])],
        )],
    );
    let diagram = el(
        "diagram",
        &[],
        vec![el(
            "nodes",
            &[],
            vec![
                node(
                    "0",
                    "uidExt",
                    "x.y.CFBuildModelNode",
                    &[
                        ("extensionID", "spss.alm-extension"),
                        ("extensionObjectID", "LinearRegression"),
                        ("properties", "data/0001.dat"),
                    ],
                    vec![el(
                        "container",
                        &[("containerData", "data/0002.dat")],
                        vec![el("containerName", &[], vec![text("PMML")])],
                    )],
                ),
                node(
                    "1",
                    "uidNugget",
                    "x.y.CFApplyModelNode",
                    &[],
                    vec![el(
                        "modelBuildInfo",
                        &[("modelBuilderId", "LinearRegression!spss.alm-extension")],
                        vec![],
                    )],
                ),
                node("2", "uidSuper", "x.y.MutableSuperNode", &[], vec![inner]),
            ],
        )],
    );
    let stream = Stream::from_document(&document(diagram)).unwrap();
    let nodes = &stream.diagram.nodes;
    assert_eq!(
        nodes[0].type_identifier(),
        "LinearRegression!spss.alm-extension"
    );
    assert_eq!(
        nodes[0].properties_attachment.as_deref(),
        Some("data/0001.dat")
    );
    assert_eq!(nodes[0].containers[0].name.as_deref(), Some("PMML"));
    assert_eq!(nodes[0].containers[0].member, "data/0002.dat");
    assert_eq!(
        nodes[1].model_builder.as_deref(),
        Some("LinearRegression!spss.alm-extension")
    );
    let all: Vec<&str> = stream.all_nodes().iter().map(|n| n.uid.as_str()).collect();
    assert_eq!(all, ["uidExt", "uidNugget", "uidSuper", "uidInner"]);
}

#[test]
fn reads_analytic_server_content_containers() {
    // `ts_arima.str` ts_applier: nested content[@content], not containerData.
    let diagram = el(
        "diagram",
        &[],
        vec![el(
            "nodes",
            &[],
            vec![node(
                "0",
                "uidTs",
                "com.spss.clementine.component.node.ExtensionApplyModelNode",
                &[
                    ("extensionID", "spss.ts"),
                    ("extensionElementID", "ts_applier"),
                ],
                vec![
                    el(
                        "modelBuildInfo",
                        &[("modelBuilderId", "spss.ts!ts_builder")],
                        vec![],
                    ),
                    el(
                        "contentContainers",
                        &[],
                        vec![el(
                            "containers",
                            &[],
                            vec![el(
                                "container",
                                &[("name", "PMML")],
                                vec![el("content", &[("content", "data/0003.dat")], vec![])],
                            )],
                        )],
                    ),
                ],
            )],
        )],
    );
    let stream = Stream::from_document(&document(diagram)).unwrap();
    let node = &stream.diagram.nodes[0];
    assert_eq!(node.type_identifier(), "spss.ts!ts_applier");
    assert_eq!(node.containers[0].name.as_deref(), Some("PMML"));
    assert_eq!(node.containers[0].member, "data/0003.dat");
}

#[test]
fn rejects_non_streams_and_duplicate_uids() {
    let other = Document {
        children: vec![el("PMML", &[], vec![])],
        ..Document::default()
    };
    assert_eq!(
        Stream::from_document(&other).unwrap_err(),
        StreamError::NotAStream("PMML".into())
    );
    let diagram = el(
        "diagram",
        &[],
        vec![el(
            "nodes",
            &[],
            vec![
                node("0", "same", "x.A", &[], vec![]),
                node("1", "same", "x.B", &[], vec![]),
            ],
        )],
    );
    assert_eq!(
        Stream::from_document(&document(diagram)).unwrap_err(),
        StreamError::DuplicateUid("same".into())
    );
}
