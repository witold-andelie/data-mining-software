//! Builder-only QUEST oracle comparisons: a saved nugget cannot supply the trained model.
#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "oracle assertions")]

mod support;
use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_pmml::{Model, Node, Pmml, Predicate, Value};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use std::sync::Arc;

fn train(label: &str, case: &str) -> Result<Pmml, String> {
    let root = support::repo_root();
    let archive = dms_strfmt::container::read_archive(
        &std::fs::read(root.join(format!("corpus/{label}/{case}.str"))).unwrap(),
        &ArchiveLimits::default(),
    )
    .unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let builder = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "Quest!spss.tree-extension")
        .unwrap();
    let context = ExecContext {
        files: FileResolver {
            data_dirs: vec![root.join(format!("corpus/{label}"))],
        },
        attachments: Arc::new(
            archive
                .attachments
                .into_iter()
                .map(|a| (a.name, Arc::from(a.bytes)))
                .collect(),
        ),
        stream_properties: stream.properties.clone().map(Arc::new),
        ..ExecContext::default()
    };
    let results = Plan::new(&stream).evaluate(&[builder.uid.as_str()], &context);
    match results.get(&builder.uid) {
        Some(Ok(NodeOutput::Model(m))) => Ok((**m).clone()),
        other => Err(format!("{case}: builder failed: {other:?}")),
    }
}

fn close(a: f64, b: f64, context: &str) {
    assert!(
        a.is_finite() && b.is_finite() && (a - b).abs() <= 1e-9 * b.abs().max(1.0),
        "{context}: {a} != {b}"
    );
}

fn predicate(a: &Predicate, b: &Predicate, context: &str) {
    match (a, b) {
        (
            Predicate::Simple {
                field: af,
                operator: ao,
                value: av,
            },
            Predicate::Simple {
                field: bf,
                operator: bo,
                value: bv,
            },
        ) => {
            assert_eq!((af, ao), (bf, bo), "{context}");
            match (
                av.as_ref().and_then(|v| v.parse::<f64>().ok()),
                bv.as_ref().and_then(|v| v.parse::<f64>().ok()),
            ) {
                (Some(a), Some(b)) => close(a, b, context),
                _ => assert_eq!(av, bv, "{context}"),
            }
        }
        (
            Predicate::Compound {
                operator: ao,
                predicates: ap,
            },
            Predicate::Compound {
                operator: bo,
                predicates: bp,
            },
        ) => {
            assert_eq!(ao, bo, "{context}");
            assert_eq!(ap.len(), bp.len(), "{context}: {a:?} vs {b:?}");
            for (a, b) in ap.iter().zip(bp) {
                predicate(a, b, context);
            }
        }
        _ => assert_eq!(a, b, "{context}"),
    }
}

fn tree(a: &Node, b: &Node, case: &str) -> usize {
    let context = format!("{case} node {:?}", b.id);
    assert_eq!(a.id, b.id, "{context}");
    assert_eq!(a.default_child, b.default_child, "{context}");
    predicate(&a.predicate, &b.predicate, &context);
    assert_eq!(a.score, b.score, "{context}");
    close(a.record_count.unwrap(), b.record_count.unwrap(), &context);
    assert_eq!(a.distribution.len(), b.distribution.len(), "{context}");
    for (a, b) in a.distribution.iter().zip(&b.distribution) {
        assert_eq!(a.value, b.value, "{context}");
        close(a.record_count, b.record_count, &context);
        close(a.probability.unwrap(), b.probability.unwrap(), &context);
        close(a.confidence.unwrap(), b.confidence.unwrap(), &context);
    }
    assert_eq!(a.children.len(), b.children.len(), "{context}");
    1 + a
        .children
        .iter()
        .zip(&b.children)
        .map(|(a, b)| tree(a, b, case))
        .sum::<usize>()
}

#[test]
fn trained_quest_matches_tree_and_scored_records() {
    assert!(dms_exec::registry::is_implemented("quest"));
    // `quest_edge/*` (probe `quest_edge_streams`): one-record QDA superclasses, a class mean
    // halfway between the 2-means centres, a Levene-selected field and an ordinal primary split;
    // `quest_tie/*`: tied selection p values.
    for (label, case, records) in [
        ("quest_nofs", "quest_nofs_default", 91),
        ("quest_nofs", "quest_nofs_depth3", 91),
        ("quest_nofs", "quest_nofs_unpruned", 91),
        ("quest_nofs", "quest_nofs_binary", 91),
        ("quest_edge", "quest_edge_singleton2", 42),
        ("quest_edge", "quest_edge_singleton3", 42),
        ("quest_edge", "quest_edge_tie3", 60),
        ("quest_edge", "quest_edge_levene", 80),
        ("quest_edge", "quest_edge_ordinal", 60),
        ("quest_tie", "quest_tie_numeric_pq", 60),
        ("quest_tie", "quest_tie_numeric_qp", 60),
        ("quest_tie", "quest_tie_set_cd", 60),
        ("quest_tie", "quest_tie_set_dc", 60),
    ] {
        let ours = train(label, case).unwrap();
        let root = support::repo_root().join(format!("oracle/{label}/{case}"));
        let oracle = Pmml::parse(
            &std::fs::read_to_string(root.join(format!("models/01_{label}.xml"))).unwrap(),
        )
        .unwrap();
        let (Model::Tree(a), Model::Tree(b)) = (&ours.model, &oracle.model) else {
            panic!("not a tree");
        };
        assert_eq!(a.header.algorithm, b.header.algorithm);
        assert_eq!(a.header.function, b.header.function);

        assert_eq!(a.missing_value_strategy, b.missing_value_strategy);
        assert_eq!(a.no_true_child_strategy, b.no_true_child_strategy);
        let nodes = tree(&a.root, &b.root, case);
        assert_eq!(a.header.mining_schema, b.header.mining_schema);
        let names: Vec<_> = ours
            .data_dictionary
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        let mut scored = 0;
        for path in std::fs::read_dir(root.join("data"))
            .unwrap()
            .map(|e| e.unwrap().path())
        {
            if path.extension().is_none_or(|s| s != "csv") {
                continue;
            }
            let text = std::fs::read_to_string(path).unwrap();
            let mut lines = text.lines();
            let header = support::parse_csv_line(lines.next().unwrap());
            let Some(predicted) = header.iter().position(|s| s.starts_with("$R-")) else {
                continue;
            };
            let confidence = header.iter().position(|s| s.starts_with("$RC-")).unwrap();
            for line in lines.filter(|s| !s.is_empty()) {
                let cells = support::parse_csv_line(line);
                let row: Vec<_> = names
                    .iter()
                    .map(|name| {
                        let p = header.iter().position(|s| s == name).unwrap();
                        if cells[p].is_empty() {
                            Value::Missing
                        } else {
                            cells[p]
                                .parse()
                                .map(Value::Number)
                                .unwrap_or_else(|_| Value::Text(cells[p].clone()))
                        }
                    })
                    .collect();
                let prediction = ours.bind(&names).unwrap().score(&row).unwrap();
                assert_eq!(
                    prediction.value,
                    Value::Text(cells[predicted].clone()),
                    "{case} record {scored}"
                );
                close(
                    prediction.confidence.unwrap(),
                    cells[confidence].parse().unwrap(),
                    &format!("{case} record {scored}"),
                );
                scored += 1;
            }
        }
        assert_eq!(scored, records, "{case}");
        println!("TRAINED {case}: {nodes} nodes, {scored} scored records equal Modeler");
    }
}

#[test]
fn overfit_prevention_set_streams_train() {
    for case in ["quest_default", "quest_depth3", "quest_unpruned"] {
        let pmml = train("models_next", case).unwrap_or_else(|error| panic!("{case}: {error}"));
        let Model::Tree(tree) = &pmml.model else {
            panic!("{case}: not a tree");
        };
        assert!(
            tree.root.record_count.unwrap_or(0.0) > 0.0,
            "{case}: empty training set"
        );
    }
}
