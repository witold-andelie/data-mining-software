//! GLE Phase I builder and nugget wiring (`spss.gle!gle_builder` / `spss.gle!gle_applier`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use dms_exec::nodes::{apply, build};
use dms_exec::{ExecContext, NodeOutput, OperatorError};
use dms_pmml::{GeneralModelType, MiningFunction, Model};
use dms_stream::{Extension, StreamNode};
use dms_strfmt::fastinfoset::{Attribute, Element, Node, QualifiedName};
use dms_types::{DataModel, Field, Measurement, Role, Storage, Table, Value, ValueSpec};
use std::sync::Arc;

#[cfg(feature = "corpus")]
mod support;

fn name(local: &str) -> QualifiedName {
    QualifiedName {
        prefix: None,
        namespace: None,
        local: Arc::from(local),
    }
}

fn element(local: &str, attributes: Vec<(&str, &str)>, children: Vec<Node>) -> Element {
    Element {
        name: name(local),
        namespace_declarations: Vec::new(),
        attributes: attributes
            .into_iter()
            .map(|(n, v)| Attribute {
                name: name(n),
                value: Arc::from(v),
            })
            .collect(),
        children,
    }
}

fn p(local: &str, text: &str) -> Node {
    Node::Element(element(
        local,
        Vec::new(),
        vec![Node::Text(Arc::from(text))],
    ))
}

fn builder(settings: Vec<Node>) -> StreamNode {
    StreamNode {
        uid: "gle".into(),
        class: "com.spss.clementine.component.node.ExtensionBuildModelNode".into(),
        extension: Some(Extension::Element {
            extension_id: "spss.gle".into(),
            element_id: "gle_builder".into(),
        }),
        model_builder: None,
        name: Some("gle".into()),
        label: None,
        position: None,
        properties_attachment: None,
        containers: Vec::new(),
        settings: Some(element("properties", Vec::new(), settings)),
        binning_output: None,
        result_items: Vec::new(),
        subdiagram: None,
    }
}

fn nugget(settings: Vec<Node>) -> StreamNode {
    StreamNode {
        uid: "applygle".into(),
        class: "com.spss.clementine.component.node.ExtensionApplyModelNode".into(),
        extension: Some(Extension::Element {
            extension_id: "spss.gle".into(),
            element_id: "gle_applier".into(),
        }),
        model_builder: Some("spss.gle!gle_builder".into()),
        name: Some("applygle".into()),
        label: None,
        position: None,
        properties_attachment: None,
        containers: Vec::new(),
        settings: Some(element("properties", Vec::new(), settings)),
        binning_output: None,
        result_items: Vec::new(),
        subdiagram: None,
    }
}

fn numeric(name: &str, role: Role) -> Field {
    let mut field = Field::new(name, Storage::Real);
    field.measurement = Measurement::Continuous;
    field.role = role;
    field
}

fn flag() -> Field {
    let mut field = Field::new("y", Storage::String);
    field.measurement = Measurement::Flag;
    field.role = Role::Target;
    field.values = ValueSpec::Flag {
        true_value: Value::string("T"),
        false_value: Value::string("F"),
    };
    field
}

fn line_table() -> Table {
    let rows: Vec<Vec<Value>> = (1..=4)
        .map(|i| vec![Value::Real(i as f64), Value::Real(1.0 + 2.0 * i as f64)])
        .collect();
    Table::new(
        DataModel::new(vec![numeric("x", Role::Input), numeric("y", Role::Target)]).unwrap(),
        rows,
    )
    .unwrap()
}

fn flag_table() -> Table {
    let rows = vec![
        vec![Value::Real(0.0), Value::string("F")],
        vec![Value::Real(0.0), Value::string("F")],
        vec![Value::Real(1.0), Value::string("T")],
        vec![Value::Real(1.0), Value::string("T")],
        vec![Value::Real(1.0), Value::string("T")],
        vec![Value::Real(1.0), Value::string("F")],
    ];
    Table::new(
        DataModel::new(vec![numeric("x", Role::Input), flag()]).unwrap(),
        rows,
    )
    .unwrap()
}

fn train(node: StreamNode, table: Table) -> dms_pmml::Pmml {
    match build::build(&node)
        .unwrap()
        .produce(&[Arc::new(table)], None, &ExecContext::default())
        .unwrap()
    {
        NodeOutput::Model(model) => (*model).clone(),
        other => panic!("expected a model, got {other:?}"),
    }
}

#[test]
fn gle_builder_trains_normal_identity() {
    let pmml = train(
        builder(vec![p("_distribution", "NormalIdentity")]),
        line_table(),
    );
    let Model::GeneralRegression(model) = &pmml.model else {
        panic!("expected GeneralRegressionModel");
    };
    assert_eq!(model.model_type, GeneralModelType::GeneralizedLinear);
    assert_eq!(model.header.function, MiningFunction::Regression);
    assert_eq!(model.link_function.as_deref(), Some("identity"));
    assert!((model.param_matrix[0].beta - 1.0).abs() < 1e-8);
    assert!((model.param_matrix[1].beta - 2.0).abs() < 1e-8);
}

#[test]
fn gle_nugget_scores_trained_mean() {
    let pmml = train(
        builder(vec![p("_distribution", "NormalIdentity")]),
        line_table(),
    );
    let scored = apply::build(&nugget(vec![]))
        .unwrap()
        .produce(
            &[Arc::new(line_table())],
            Some(Arc::new(pmml)),
            &ExecContext::default(),
        )
        .unwrap();
    let table = scored.data().unwrap();
    let pred = table.column("$G-y").unwrap().cloned().collect::<Vec<_>>();
    assert_eq!(
        pred,
        vec![
            Value::Real(3.0),
            Value::Real(5.0),
            Value::Real(7.0),
            Value::Real(9.0),
        ]
    );
}

#[test]
fn gle_binomial_writes_class_and_probabilities() {
    let pmml = train(
        builder(vec![p("_distribution", "BinomialLogit")]),
        flag_table(),
    );
    let Model::GeneralRegression(model) = &pmml.model else {
        panic!("expected GeneralRegressionModel");
    };
    assert_eq!(model.header.function, MiningFunction::Classification);
    assert_eq!(model.model_type, GeneralModelType::MultinomialLogistic);
    assert_eq!(model.link_function.as_deref(), Some("logit"));
    let scored = apply::build(&nugget(vec![p("_GenerateSQL", "native")]))
        .unwrap()
        .produce(
            &[Arc::new(flag_table())],
            Some(Arc::new(pmml)),
            &ExecContext::default(),
        )
        .unwrap();
    let table = scored.data().unwrap();
    assert!(table.model().position("$L-y").is_some());
    assert!(table.model().position("$LC-y").is_some());
    assert!(table.model().position("$LP-y").is_none());
    assert!(table.model().position("$LP-F").is_none());
}

#[test]
fn gle_rejects_out_of_scope_settings() {
    for (name, value) in [
        ("_distribution", "NegbinLog"),
        ("_useVariableSelection", "true"),
        ("_detectTwoWayInteraction", "true"),
        ("_useTrials", "true"),
        ("_useOffset", "Value"),
        ("_nonNegLeastSquares", "true"),
        ("_usePredefinedInputs", "false"),
    ] {
        let Err(err) = build::build(&builder(vec![p(name, value)])) else {
            panic!("{name}={value} should be a settings GAP");
        };
        let OperatorError::Settings(message) = err else {
            panic!("{name}={value} should be a settings GAP, got {err:?}");
        };
        assert!(message.contains("GAP:"), "{name}={value} -> {message}");
    }
}

#[test]
fn applygle_rejects_unknown_sql_generation() {
    let Err(err) = apply::build(&nugget(vec![p("_GenerateSQL", "puresql")])) else {
        panic!("applygle should reject _GenerateSQL=puresql");
    };
    let OperatorError::Settings(message) = err else {
        panic!("expected settings error, got {err:?}");
    };
    assert!(message.contains("GAP:"), "{message}");
}

#[test]
fn catalog_maps_stream_identifiers() {
    assert_eq!(
        dms_catalog::by_modeler_identifier("spss.gle!gle_builder").map(|n| n.id),
        Some("gle")
    );
    assert_eq!(
        dms_catalog::by_modeler_identifier("spss.gle!gle_applier").map(|n| n.id),
        Some("gle")
    );
    assert!(dms_catalog::by_modeler_identifier("gle").is_none());
    assert!(dms_catalog::by_modeler_identifier("applygle").is_none());
    assert!(dms_exec::registry::is_implemented("gle"));
}

#[cfg(feature = "corpus")]
mod oracle {
    use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
    use dms_stream::Stream;
    use dms_strfmt::ArchiveLimits;
    use dms_types::Value;
    use std::sync::Arc;

    #[test]
    fn gle_streams_match_oracle_terminal_inputs() {
        let root = super::support::repo_root();
        for case in [
            "gle_normal_identity",
            "gle_binom_logit",
            "gle_apply_native",
            "gle_apply_udf",
        ] {
            let bytes = std::fs::read(root.join(format!("corpus/gle/{case}.str"))).unwrap();
            let archive =
                dms_strfmt::container::read_archive(&bytes, &ArchiveLimits::default()).unwrap();
            let stream = Stream::from_document(&archive.document).unwrap();
            let builder = stream
                .diagram
                .nodes
                .iter()
                .find(|n| n.type_identifier() == "spss.gle!gle_builder");
            let nugget = stream
                .diagram
                .nodes
                .iter()
                .find(|n| n.type_identifier() == "spss.gle!gle_applier");
            assert!(builder.is_some(), "{case} builder");
            assert!(nugget.is_some(), "{case} nugget");
            assert_eq!(
                nugget.unwrap().model_builder.as_deref(),
                Some("spss.gle!gle_builder"),
                "{case}"
            );
            let context = ExecContext {
                files: FileResolver {
                    data_dirs: vec![root.join("corpus/gle")],
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
            let manifest: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(root.join(format!("oracle/gle/{case}/manifest.json")))
                    .unwrap(),
            )
            .unwrap();
            let entries = manifest["data"].as_array().unwrap();
            let targets: Vec<_> = entries
                .iter()
                .map(|entry| entry["id"].as_str().unwrap())
                .collect();
            let results = Plan::new(&stream).evaluate(&targets, &context);
            for entry in entries {
                let uid = entry["id"].as_str().unwrap();
                let Some(Ok(NodeOutput::Data(actual))) = results.get(uid) else {
                    panic!("{case}/{uid}: {:?}", results.get(uid));
                };
                let text = std::fs::read_to_string(super::support::local_path(
                    entry["file"].as_str().unwrap(),
                ))
                .unwrap();
                let mut lines = text.trim_start_matches('\u{feff}').lines();
                let names = super::support::parse_csv_line(lines.next().unwrap());
                assert_eq!(
                    actual
                        .model()
                        .fields()
                        .iter()
                        .map(|f| f.name.as_str())
                        .collect::<Vec<_>>(),
                    names,
                    "{case}/{uid} fields"
                );
                let rows: Vec<_> = lines.map(super::support::parse_csv_line).collect();
                assert_eq!(actual.rows().len(), rows.len(), "{case}/{uid} rows");
                for (r, (got, want)) in actual.rows().iter().zip(&rows).enumerate() {
                    assert_eq!(got.len(), want.len(), "{case}/{uid} row {r}");
                    for (c, (got, want)) in got.iter().zip(want).enumerate() {
                        let matches = match got {
                            Value::Null => want.is_empty() || want == "$null$",
                            Value::String(s) => s.as_ref() == want,
                            Value::Integer(_) | Value::Real(_) => {
                                let a = got.as_f64().unwrap();
                                let b = want.parse::<f64>().unwrap();
                                a.is_finite()
                                    && b.is_finite()
                                    && (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
                            }
                            other => other.to_string() == *want,
                        };
                        assert!(
                            matches,
                            "{case}/{uid} r{r} c{c} {} {got:?} vs {want}",
                            names[c]
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn gle_binomial_identity_writes_g_fields() {
    let pmml = train(
        builder(vec![
            p("_distribution", "CUSTOM"),
            p("_distributionOfTarget", "BINOMIAL"),
            p("_linkFunction", "IDENTITY"),
        ]),
        flag_table(),
    );
    let Model::GeneralRegression(model) = &pmml.model else {
        panic!("expected GeneralRegressionModel");
    };
    assert_eq!(model.model_type, GeneralModelType::GeneralizedLinear);
    let scored = apply::build(&nugget(vec![]))
        .unwrap()
        .produce(
            &[Arc::new(flag_table())],
            Some(Arc::new(pmml)),
            &ExecContext::default(),
        )
        .unwrap();
    let table = scored.data().unwrap();
    assert!(table.model().position("$G-y").is_some());
    assert!(table.model().position("$GC-y").is_some());
    assert!(table.model().position("$L-y").is_none());
}
