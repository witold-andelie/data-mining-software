//! Synthetic contracts require no DSN. Live round trips are under `corpus` only.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests"
)]
use dms_exec::nodes::database::{build_export, build_import};
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::{Element, Node, QualifiedName};
#[cfg(all(feature = "corpus", windows))]
use std::sync::Arc;

fn element(name: &str, children: Vec<Node>) -> Element {
    Element {
        name: QualifiedName {
            prefix: None,
            namespace: None,
            local: name.into(),
        },
        namespace_declarations: vec![],
        attributes: vec![],
        children,
    }
}
fn p(name: &str, value: &str) -> Node {
    Node::Element(element(name, vec![Node::Text(value.into())]))
}
fn node(export: bool, extra: Vec<Node>) -> StreamNode {
    let mut children = vec![
        Node::Element(element(
            "connection",
            vec![
                p("datasource", "dms_access"),
                p("type", "odbc"),
                p("password", "false"),
                p("useCredential", "false"),
            ],
        )),
        p("tableName", "customers"),
    ];
    if export {
        children.push(p("databaseExportType", "create"));
    }
    children.extend(extra);
    StreamNode {
        uid: "db".into(),
        class: if export {
            "ODBCExportNode"
        } else {
            "ODBCImportNode"
        }
        .into(),
        extension: None,
        model_builder: None,
        name: None,
        label: None,
        position: None,
        properties_attachment: None,
        containers: vec![],
        settings: Some(element("properties", children)),
        binning_output: None,
        result_items: vec![],
        subdiagram: None,
    }
}

#[test]
fn default_operators_and_catalog_aliases() {
    assert!(
        build_import(&node(false, vec![]))
            .unwrap()
            .arity()
            .accepts(0)
    );
    assert!(
        build_export(&node(true, vec![]))
            .unwrap()
            .arity()
            .accepts(1)
    );
    for (export, classes) in [
        (false, ["ODBCImportNode", "DBImportNode"]),
        (true, ["ODBCExportNode", "DBExportNode"]),
    ] {
        for class in classes {
            let mut n = node(export, vec![]);
            n.class = class.into();
            let id = dms_catalog::by_modeler_identifier(&n.type_identifier()).unwrap();
            assert!(
                dms_exec::registry::factory(id.id).unwrap()(&n).is_ok(),
                "{class}"
            );
        }
    }
}

#[test]
fn unsupported_settings_fail_before_connecting() {
    for (export, key, value) in [
        (false, "importMode", "other"),
        (false, "trimMode", "both"),
        (false, "delimitMode", "always"),
        (false, "datetimeMode", "string"),
        (false, "schemaName", "dbo"),
        (false, "enabled", "false"),
        (true, "databaseExportType", "append"),
        (true, "databaseExportType", "merge"),
        (true, "deleteExistingRows", "true"),
        (true, "dropExistingTable", "false"),
        (true, "dbxBulkLoadOption", "on"),
        (true, "dbxUseCreateTableTemplate", "true"),
        (true, "dbxUseCreateIndexTemplate", "true"),
        (true, "dbxIndexList", "ix"),
        (true, "keyColumns", "id"),
        (true, "databaseCustomTypes", "id"),
        (true, "dbxBatchSize", "20"),
        (true, "useBatchCommit", "false"),
        (true, "defaultStringSize", "20"),
        (true, "dbxExportTablespace", "foo"),
        (true, "dbxOdbcBindType", "Column"),
        (true, "dbxCreateTableTemplate", "DROP TABLE customers"),
        (true, "unknownSetting", "true"),
    ] {
        let mut n = node(export, vec![]);
        let s = n.settings.as_mut().unwrap();
        s.children
            .retain(|c| !matches!(c,Node::Element(e) if &*e.name.local == key));
        s.children.push(p(key, value));
        let result = if export {
            build_export(&n)
        } else {
            build_import(&n)
        };
        let error = match result {
            Ok(_) => panic!("accepted {key}"),
            Err(e) => format!("{e:?}"),
        };
        assert!(error.contains("GAP:"), "{key}: {error}");
    }
}

#[test]
fn secrets_and_connection_overrides_never_reach_errors() {
    for key in [
        "password",
        "epassword",
        "user",
        "credential",
        "useCredential",
        "catalog",
        "type",
    ] {
        for export in [false, true] {
            let mut n = node(export, vec![]);
            let connection = n
                .settings
                .as_mut()
                .unwrap()
                .children
                .iter_mut()
                .find_map(|n| match n {
                    Node::Element(e) if &*e.name.local == "connection" => Some(e),
                    _ => None,
                })
                .unwrap();
            connection
                .children
                .retain(|n| !matches!(n,Node::Element(e) if &*e.name.local==key));
            connection.children.push(p(key, "secret-sentinel-123"));
            let result = if export {
                build_export(&n)
            } else {
                build_import(&n)
            };
            let error = match result {
                Ok(_) => panic!("accepted {key}"),
                Err(e) => format!("{e:?}"),
            };
            assert!(error.contains("GAP:"));
            assert!(!error.contains("secret-sentinel-123"));
        }
    }
    for key in ["password", "epassword"] {
        let n = node(false, vec![p(key, "secret-sentinel-123")]);
        let error = format!("{:?}", build_import(&n).err().unwrap());
        assert!(!error.contains("secret-sentinel-123"));
    }
}

#[test]
fn nested_and_duplicate_settings_fail_closed() {
    let n = node(
        false,
        vec![Node::Element(element(
            "typeTable",
            vec![Node::Element(element("rows", vec![p("row", "override")]))],
        ))],
    );
    assert!(build_import(&n).is_err());
    assert!(build_import(&node(false, vec![p("tableName", "other")])).is_err());
    assert!(
        build_import(&node(
            false,
            vec![Node::Element(element("schemaName", vec![p("value", "")]))]
        ))
        .is_err()
    );
}

// The oracles were recorded through the DSN `dms_access`, a Microsoft Access database. The
// Access ODBC driver exists only for Windows, so this live comparison cannot run elsewhere.
#[cfg(all(feature = "corpus", windows))]
mod live {
    use super::*;
    use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
    use dms_stream::Stream;
    use dms_types::{Storage, Table};
    use std::path::{Path, PathBuf};

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    }
    fn stream(case: &str) -> Stream {
        let bytes = std::fs::read(root().join(format!("corpus/database/{case}.str"))).unwrap();
        let archive = dms_strfmt::container::read_archive(&bytes, &Default::default()).unwrap();
        Stream::from_document(&archive.document).unwrap()
    }
    fn context(s: &Stream) -> ExecContext {
        ExecContext {
            files: FileResolver {
                data_dirs: vec![root().join("corpus/database")],
            },
            stream_properties: s.properties.clone().map(Arc::new),
            decimal_places: Some(15),
            export_dir: Some(root().join("target/database-tests")),
            ..Default::default()
        }
    }
    fn evaluate(s: &Stream, uid: &str) -> Arc<Table> {
        match Plan::new(s)
            .evaluate(&[uid], &context(s))
            .remove(uid)
            .unwrap()
            .unwrap_or_else(|e| {
                panic!(
                    "database DSN dms_access required (64-bit Access driver/database access): {e}"
                )
            }) {
            NodeOutput::Data(t) => t,
            other => panic!("expected table: {other:?}"),
        }
    }
    fn oracle(case: &str, table: &Table) {
        let dir = root().join(format!("oracle/database/{case}/data"));
        let file = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|s| s == "csv"))
            .unwrap();
        // Numeric print width differs; parse oracle values with the source's runtime storage.
        let bytes = std::fs::read(file).unwrap();
        let options = dms_io::delimited::DelimitedOptions {
            text: dms_io::text::TextOptions {
                overrides: table
                    .model()
                    .fields()
                    .iter()
                    .map(|f| dms_io::text::StorageOverride {
                        field: f.name.clone(),
                        storage: f.storage,
                        date_format: None,
                        time_format: None,
                    })
                    .collect(),
                ..Default::default()
            },
            ..Default::default()
        };
        let expected = dms_io::delimited::read_table(&bytes, &options).unwrap();
        assert_eq!(
            table
                .model()
                .fields()
                .iter()
                .map(|f| &f.name)
                .collect::<Vec<_>>(),
            expected
                .model()
                .fields()
                .iter()
                .map(|f| &f.name)
                .collect::<Vec<_>>()
        );
        assert_eq!(table.rows(), expected.rows(), "{case}");
    }

    #[test]
    fn saved_database_settings_build() {
        for case in ["db_table", "db_query", "db_export", "db_export_readback"] {
            let s = stream(case);
            for node in &s.diagram.nodes {
                let id = dms_catalog::by_modeler_identifier(&node.type_identifier()).unwrap();
                if matches!(id.id, "database_import" | "database_export") {
                    assert!(
                        dms_exec::registry::factory(id.id).unwrap()(node).is_ok(),
                        "{case}"
                    );
                }
            }
        }
    }

    #[test]
    fn database_oracles() {
        // Some drivers ignore SQL_ATTR_LOGIN_TIMEOUT. Bound the entire live test with
        // a subprocess, so an inaccessible DSN cannot hang the test harness indefinitely.
        if std::env::var_os("DMS_DATABASE_TEST_WORKER").is_some() {
            roundtrip();
            return;
        }
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};
        let log = root().join("target/database-live-test.log");
        let output = std::fs::File::create(&log).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "live::database_oracles", "--nocapture"])
            .env("DMS_DATABASE_TEST_WORKER", "1")
            .stdout(Stdio::from(output.try_clone().unwrap()))
            .stderr(Stdio::from(output))
            .spawn()
            .unwrap();
        let start = Instant::now();
        let status = loop {
            if let Some(s) = child.try_wait().unwrap() {
                break s;
            }
            if start.elapsed() > Duration::from_secs(90) {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!(
                    "DSN dms_access did not respond in 90s: verify 64-bit Access driver/database permissions; {}",
                    log.display()
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(
            status.success(),
            "DSN dms_access live comparison failed:\n{}",
            std::fs::read_to_string(log).unwrap()
        );
    }
    fn roundtrip() {
        for case in ["db_table", "db_query", "db_export_readback"] {
            let s = stream(case);
            let source = s.diagram.nodes.first().unwrap();
            let table = evaluate(&s, &source.uid);
            oracle(case, &table);
            if case == "db_table" {
                assert_eq!(
                    table
                        .model()
                        .fields()
                        .iter()
                        .map(|f| f.storage)
                        .collect::<Vec<_>>(),
                    vec![
                        Storage::Integer,
                        Storage::String,
                        Storage::String,
                        Storage::Real,
                        Storage::Timestamp,
                        Storage::Real
                    ]
                );
            }
        }
        let s = stream("db_export");
        let source = s.diagram.nodes.first().unwrap();
        let input = evaluate(&s, &source.uid);
        oracle("db_export", &input);
        let export = s
            .diagram
            .nodes
            .iter()
            .find(|n| n.type_identifier() == "ODBCExportNode")
            .unwrap();
        let passed = evaluate(&s, &export.uid);
        assert_eq!(passed.rows(), input.rows());
        let expected = dms_io::odbc::columns("dms_access", "exp_create").unwrap();
        let actual = dms_io::odbc::columns("dms_access", "dms_exp_create").unwrap();
        assert!(!expected.is_empty());
        assert_eq!(actual, expected, "SQLColumns metadata differs");
        let mut s = stream("db_export_readback");
        let source = s.diagram.nodes.first_mut().unwrap();
        let settings = source.settings.as_mut().unwrap();
        settings
            .children
            .retain(|n| !matches!(n,Node::Element(e) if &*e.name.local=="tableName"));
        settings.children.push(p("tableName", "dms_exp_create"));
        let uid = source.uid.clone();
        let table = evaluate(&s, &uid);
        oracle("db_export_readback", &table);
        assert_eq!(
            table.model().fields().first().unwrap().storage,
            Storage::Real
        );
    }
}
