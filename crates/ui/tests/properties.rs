//! Headless property editing through pointer and keyboard events; no snapshots.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]
use dms_engine::session::{Document, Session};
use dms_exec::schema::ParamValue;
use dms_ui::properties::Properties;
use dms_ui::workbench::Pick;
use dms_ui::{Platform, ShellState, Workbench, synthetic};
use egui::{Key, Modifiers, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

struct State {
    form: Properties,
    session: Session,
}
fn harness(class: &str, xml: &str) -> Harness<'static, State> {
    let mut stream = synthetic::stream(1);
    let node = &mut stream.diagram.nodes[0];
    node.uid = "node".into();
    node.class = format!("test.{class}");
    node.label = Some("Example".into());
    node.settings = Some(dms_stream::settings_xml::element_from_xml(xml).unwrap());
    harness_session(Session::with_document(Document::from_stream(
        "test.str".into(),
        stream,
    )))
}
fn harness_session(session: Session) -> Harness<'static, State> {
    let mut h = Harness::builder()
        .with_size(vec2(800.0, 1100.0))
        .with_os(egui::os::OperatingSystem::Mac)
        .build_ui_state(
            |ui, s: &mut State| {
                if ui.button("Undo").clicked() {
                    s.session.execute("edit.undo", &[]).unwrap();
                }
                if ui.button("Redo").clicked() {
                    s.session.execute("edit.redo", &[]).unwrap();
                }
                s.form.show(ui, &mut s.session, "node");
            },
            State {
                form: Properties::default(),
                session,
            },
        );
    h.run();
    h
}

fn xml(h: &Harness<'_, State>) -> String {
    h.state().session.node_settings("node").unwrap().to_xml()
}
fn replace(h: &mut Harness<'_, State>, label: &str, text: &str) {
    h.get_by_label(label).click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run();
}
fn choose(h: &mut Harness<'_, State>, label: &str, value: &str) {
    h.get_by_label(label).click();
    h.run();
    h.get_by_label(value).click();
    h.run();
}

#[test]
fn select_mode_changes_one_parameter_and_undo_redo_refresh_the_form() {
    let mut h = harness(
        "SelectNode",
        "<properties><selectMode>include</selectMode><selectCondition>x &gt; 0</selectCondition><unknown>keep</unknown></properties>",
    );
    let before = xml(&h);
    choose(&mut h, "Matching records", "Remove matches");
    assert_eq!(
        h.state().session.param_value("node", "mode").unwrap(),
        ParamValue::Text("discard".into())
    );
    assert!(xml(&h).contains("<selectMode>discard</selectMode>"));
    assert!(xml(&h).contains("<unknown>keep</unknown>"));
    assert!(h.state().session.undo_label().is_some());
    h.get_by_label("Undo").click();
    h.run();
    assert_eq!(xml(&h), before);
    assert!(h.state().session.undo_label().is_none());
    h.get_by_label("Matching records").click();
    h.run();
    h.get_by_label("Keep matches");
    h.key_press(Key::Escape);
    h.run();
    h.get_by_label("Redo").click();
    h.run();
    assert_eq!(
        h.state().session.param_value("node", "mode").unwrap(),
        ParamValue::Text("discard".into())
    );
}

#[test]
fn invalid_clem_shows_position_without_changing_settings_then_valid_expression_commits() {
    let mut h = harness(
        "SelectNode",
        "<properties><selectCondition>x &gt; 0</selectCondition></properties>",
    );
    let before = xml(&h);
    replace(&mut h, "Record condition", "x + )");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    assert!(
        h.query_by_label_contains("syntax error at character")
            .is_some()
    );
    assert_eq!(xml(&h), before);
    assert!(!h.state().session.is_dirty());
    replace(&mut h, "Record condition", "x > 10");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    assert_eq!(
        h.state().session.param_value("node", "condition").unwrap(),
        ParamValue::Text("x > 10".into())
    );
    h.get_by_label("Undo").click();
    h.run();
    assert_eq!(xml(&h), before);
}

#[test]
fn partition_rejects_out_of_range_and_unparseable_integers() {
    let mut h = harness(
        "PartitionNode",
        "<properties><trainingPartitionSize>50</trainingPartitionSize></properties>",
    );
    let before = xml(&h);
    replace(&mut h, "Training share", "101");
    h.key_press(Key::Enter);
    h.run();
    h.get_by_label("Enter a value from 0 to 100.");
    assert_eq!(xml(&h), before);
    replace(&mut h, "Training share", "abc");
    h.get_by_label("Third partition").click();
    h.run();
    h.get_by_label("Enter a whole number.");
    assert_eq!(
        h.state()
            .session
            .param_value("node", "training_size")
            .unwrap(),
        ParamValue::Int(50)
    );
    replace(&mut h, "Training share", "60");
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(
        h.state()
            .session
            .param_value("node", "training_size")
            .unwrap(),
        ParamValue::Int(60)
    );
}

#[test]
fn type_table_measurement_changes_immediately_and_preserves_other_columns() {
    let mut h = harness(
        "TypeNode",
        "<properties><typeTable><rows><row><name>age</name><measure>range</measure><direction>in</direction><future>saved</future></row></rows></typeTable></properties>",
    );
    let before = xml(&h);
    choose(&mut h, "age: Measurement", "Ordered categories");
    assert!(xml(&h).contains("<measure>orderedSet</measure>"));
    assert!(xml(&h).contains("<direction>in</direction>"));
    assert!(xml(&h).contains("<future>saved</future>"));
    h.get_by_label("Undo").click();
    h.run();
    assert_eq!(xml(&h), before);
    assert!(h.state().session.undo_label().is_none());
}

#[test]
fn unknown_node_settings_are_read_only_xml() {
    let mut h = harness(
        "UnknownNode",
        "<properties><custom>retained</custom></properties>",
    );
    let before = xml(&h);
    h.get_by_label("Settings for this node type cannot be edited yet.");
    h.get_by_label("Settings XML").click();
    h.run();
    assert_eq!(
        h.get_by_role(egui::Role::MultilineTextInput)
            .value()
            .as_deref(),
        Some((before.clone() + "\n").as_str())
    );
    h.get_by_role(egui::Role::MultilineTextInput).click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(egui::Event::Text("replacement".into()));
    h.run();
    assert_eq!(xml(&h), before);
    assert!(!h.state().session.is_dirty());
}

#[test]
fn visibility_follows_committed_values_and_unknown_elements_are_listed() {
    let mut h = harness(
        "PartitionNode",
        "<properties><futureOption>kept</futureOption></properties>",
    );
    assert!(h.query_by_label("Validation share").is_none());
    h.get_by_label("Other saved settings");
    h.get_by_label("futureOption");
    h.get_by_label("Third partition").click();
    h.run();
    h.get_by_label("Validation share");
    h.get_by_label("Undo").click();
    h.run();
    assert!(h.query_by_label("Validation share").is_none());
}

#[test]
fn file_picker_uses_schema_extensions_and_one_undo_step_in_workbench() {
    let mut stream = synthetic::stream(1);
    stream.diagram.nodes[0].label = Some("Source".into());
    let mut w = Workbench::with_document(
        Document::from_stream("test.str".into(), stream),
        ShellState {
            show_palette: false,
            ..Default::default()
        },
    );
    w.focus_node("synthetic00000");
    let mut h = Harness::builder()
        .with_size(vec2(1100.0, 1000.0))
        .with_os(egui::os::OperatingSystem::Mac)
        .build_ui_state(
            |ui, w: &mut Workbench| {
                let commands = w.ui(
                    ui,
                    &Platform {
                        mac: true,
                        native_menu: true,
                        ..Default::default()
                    },
                );
                for id in commands {
                    w.dispatch(ui.ctx(), id, &mut |kind| match kind {
                        Pick::DataFile { extensions } => {
                            assert_eq!(extensions, &["csv", "txt", "dat", "tsv"]);
                            Some("/tmp/example.csv".into())
                        }
                        _ => None,
                    });
                }
            },
            w,
        );
    h.run();
    h.get_by_label("Data file").click();
    h.run();
    h.event(egui::Event::Text("/tmp/typed.csv".into()));
    h.run();
    h.get_by_label("Choose...").click();
    h.run();
    assert_eq!(
        h.state()
            .session
            .param_value("synthetic00000", "full_filename")
            .unwrap(),
        ParamValue::Text("/tmp/example.csv".into())
    );
    h.state_mut().session.execute("edit.undo", &[]).unwrap();
    h.run();
    assert_eq!(
        h.state()
            .session
            .param_value("synthetic00000", "full_filename")
            .unwrap(),
        ParamValue::Text("/tmp/typed.csv".into())
    );
    h.state_mut().session.execute("edit.undo", &[]).unwrap();
    h.run();
    assert_eq!(
        h.state()
            .session
            .param_value("synthetic00000", "full_filename")
            .unwrap(),
        ParamValue::Unset
    );
    assert!(h.state().session.undo_label().is_none());
}

#[test]
fn missing_attachment_disables_builder_parameters_with_a_reason() {
    let mut stream = synthetic::stream(1);
    let node = &mut stream.diagram.nodes[0];
    node.uid = "node".into();
    node.class = "test.CFBuildModelNode".into();
    node.extension = Some(dms_stream::Extension::Object {
        object_id: "Chaid".into(),
        extension_id: "spss.tree-extension".into(),
    });
    let mut h = Harness::builder()
        .with_size(vec2(800.0, 1100.0))
        .build_ui_state(
            |ui, s: &mut State| {
                s.form.show(ui, &mut s.session, "node");
            },
            State {
                form: Properties::default(),
                session: Session::with_document(Document::from_stream("test.str".into(), stream)),
            },
        );
    h.run();
    assert!(
        h.query_all_by_label_contains("The node has no parameters attachment")
            .next()
            .is_some()
    );
    assert!(
        h.get_by_label("Depth limit mode")
            .accesskit_node()
            .is_disabled()
    );
    h.get_by_label("Depth limit mode").click();
    h.run();
    assert!(h.query_by_label("Specified depth").is_none());
    assert!(!h.state().session.is_dirty());
    assert!(
        h.state()
            .session
            .document_state()
            .unwrap()
            .doc
            .attachments
            .is_empty()
    );
}

#[test]
fn real_numbers_reject_nan_and_commit_on_focus_loss() {
    let mut h = harness(
        "AnalysisNode",
        "<properties><analysisThreshold>90</analysisThreshold></properties>",
    );
    let before = xml(&h);
    replace(&mut h, "Confidence cutoff", "NaN");
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(xml(&h), before);
    assert!(!h.state().session.is_dirty());
    replace(&mut h, "Confidence cutoff", "85.5");
    h.get_by_label("Options").click();
    h.run();
    assert_eq!(
        h.state().session.param_value("node", "threshold").unwrap(),
        ParamValue::Real(85.5)
    );
}

#[test]
fn fields_without_run_use_text_and_keep_field_names_with_spaces() {
    let mut h = harness("AnalysisNode", "<properties/>");
    h.get_by_label("Run the stream to list the fields reaching this node.");
    replace(&mut h, "Breakdown fields", "First name\nTotal cost");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    assert_eq!(
        h.state().session.param_value("node", "by_fields").unwrap(),
        ParamValue::Fields(vec!["First name".into(), "Total cost".into()])
    );
}

#[test]
fn table_can_add_a_field_and_cancel_by_undo() {
    let mut h = harness("FilterNode", "<properties/>");
    let specs = dms_exec::schema::schema("filter").unwrap();
    let table = specs
        .iter()
        .find(|p| matches!(p.kind, dms_exec::schema::ParamKind::Table { .. }))
        .unwrap();
    let label = format!("New field for {}", table.label);
    replace(&mut h, &label, "customer");
    h.get_all_by_label("Add field").next().unwrap().click();
    h.run();
    assert!(
        matches!(h.state().session.param_value("node", table.key).unwrap(), ParamValue::Rows(rows) if rows.len() == 1 && rows[0].0 == "customer")
    );
    h.get_by_label("Undo").click();
    h.run();
    assert!(!h.state().session.is_dirty());
}

#[test]
fn attached_depth_edit_is_reversible_byte_for_byte() {
    let mut stream = synthetic::stream(1);
    let node = &mut stream.diagram.nodes[0];
    node.uid = "node".into();
    node.class = "test.CFBuildModelNode".into();
    node.extension = Some(dms_stream::Extension::Object {
        object_id: "Chaid".into(),
        extension_id: "spss.tree-extension".into(),
    });
    node.properties_attachment = Some("params.xml".into());
    let original: std::sync::Arc<[u8]> = std::sync::Arc::from(b"<Parameters>\r\n<Parameter name='MaxTreeDepth' value='Custom'/><Parameter name='tree.MaxTreeDepth' value='5'/><!--retain--></Parameters>".as_slice());
    let mut doc = Document::from_stream("test.str".into(), stream);
    doc.attachments = std::sync::Arc::new(
        [("params.xml".into(), original.clone())]
            .into_iter()
            .collect(),
    );
    let mut h = harness_session(Session::with_document(doc));
    replace(&mut h, "Depth limit", "8");
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(
        h.state().session.param_value("node", "max_depth").unwrap(),
        ParamValue::Int(8)
    );
    h.get_by_label("Undo").click();
    h.run();
    assert_eq!(
        h.state()
            .session
            .document_state()
            .unwrap()
            .doc
            .attachments
            .get("params.xml")
            .unwrap(),
        &original
    );
    assert!(h.state().session.undo_label().is_none());
}

#[test]
fn field_pickers_use_inputs_of_the_last_run() {
    use dms_engine::session::Arg;
    use std::time::{Duration, Instant};
    let path = std::env::temp_dir().join(format!("dms-wp5-fields-{}.csv", std::process::id()));
    std::fs::write(&path, "x,y\n1,2\n2,3\n").unwrap();
    for (class, settings, key, label, multi) in [
        (
            "HistogramNode",
            "<properties><graphXField>x</graphXField></properties>",
            "field",
            "Binned field",
            false,
        ),
        (
            "AnalysisNode",
            "<properties/>",
            "by_fields",
            "Breakdown fields",
            true,
        ),
    ] {
        let mut stream = synthetic::stream(2);
        let node = &mut stream.diagram.nodes[1];
        node.uid = "node".into();
        node.class = format!("test.{class}");
        node.settings = Some(dms_stream::settings_xml::element_from_xml(settings).unwrap());
        stream.diagram.links[0].to = dms_stream::NodeRef::Uid("node".into());
        let mut session = Session::with_document(Document::from_stream("test.str".into(), stream));
        session
            .execute(
                "node.set_param",
                &[
                    Arg::Node("synthetic00000".into()),
                    Arg::Param(
                        "full_filename".into(),
                        ParamValue::Text(path.to_string_lossy().into_owned()),
                    ),
                ],
            )
            .unwrap();
        session.execute("run.stream", &[]).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while session.poll_jobs().is_empty() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            session
                .input_fields("node")
                .unwrap()
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>(),
            vec!["x", "y"]
        );
        let mut h = harness_session(session);
        assert!(
            h.query_by_label("Run the stream to list the fields reaching this node.")
                .is_none()
        );
        if multi {
            h.get_by_label("y").click();
            h.run();
        } else {
            choose(&mut h, label, "y");
        }
        let expected = if multi {
            ParamValue::Fields(vec!["y".into()])
        } else {
            ParamValue::Text("y".into())
        };
        assert_eq!(
            h.state().session.param_value("node", key).unwrap(),
            expected
        );
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn xml_preview_has_a_line_limit_and_preserves_all_saved_text() {
    let payload: String = (0..2010).map(|n| format!("record {n}\n")).collect();
    let mut h = harness(
        "UnknownNode",
        &format!("<properties><custom>{payload}</custom></properties>"),
    );
    let before = xml(&h);
    h.get_by_label("Settings XML").click();
    h.run();
    let preview = h
        .get_by_role(egui::Role::MultilineTextInput)
        .value()
        .unwrap();
    assert_eq!(preview.lines().count(), 2000);
    assert!(!preview.contains("record 2009"));
    h.get_by_label("XML preview truncated; the complete settings are preserved.");
    assert_eq!(xml(&h), before);
    assert!(!h.state().session.is_dirty());
}
