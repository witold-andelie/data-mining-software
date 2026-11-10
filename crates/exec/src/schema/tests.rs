use super::*;
use crate::{ExecContext, NodeOutput, nodes, settings};
use dms_stream::{Extension, StreamNode, settings_xml::element_from_xml};
use dms_strfmt::fastinfoset::Element;
use dms_types::{DataModel, Field, Measurement, Role, Storage, Table, Value};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

fn spec(id: &str, key: &str) -> &'static ParamSpec {
    schema(id).unwrap().iter().find(|p| p.key == key).unwrap()
}
fn xml(text: &str) -> Element {
    element_from_xml(text).unwrap()
}
fn node(settings: Option<Element>) -> StreamNode {
    StreamNode {
        uid: "n".into(),
        class: "SelectNode".into(),
        extension: None,
        model_builder: None,
        name: None,
        label: None,
        position: None,
        properties_attachment: None,
        containers: Vec::new(),
        settings,
        binning_output: None,
        result_items: Vec::new(),
        subdiagram: None,
    }
}
fn set(n: &mut StreamNode, id: &str, key: &str, value: ParamValue) {
    let p = spec(id, key);
    n.settings = Some(write(n.settings.as_ref(), p, &value).unwrap());
    assert_eq!(read(n.settings.as_ref(), p).unwrap(), value);
}
fn input() -> Arc<Table> {
    let mut x = Field::new("x", Storage::Integer);
    x.measurement = Measurement::Continuous;
    Arc::new(
        Table::new(
            DataModel::new(vec![x]).unwrap(),
            vec![
                vec![Value::Integer(1)],
                vec![Value::Integer(2)],
                vec![Value::Integer(3)],
            ],
        )
        .unwrap(),
    )
}
fn data(op: Box<dyn crate::Operator>) -> Table {
    op.run(&[input()], &ExecContext::default()).unwrap()
}

#[test]
fn select_condition_changes_the_selected_records() {
    let mut n = node(Some(xml(
        "<properties><selectCondition>x &gt; 0</selectCondition></properties>",
    )));
    set(
        &mut n,
        "select",
        "condition",
        ParamValue::Text("x > 1".into()),
    );
    assert_eq!(data(nodes::select::build(&n).unwrap()).row_count(), 2);
}
#[test]
fn derive_formula_changes_the_computed_values() {
    let mut n = node(Some(xml(
        "<properties><deriveSingleFieldName>result</deriveSingleFieldName></properties>",
    )));
    set(
        &mut n,
        "derive",
        "formula_expr",
        ParamValue::Text("x * 2".into()),
    );
    assert_eq!(
        data(nodes::derive::build(&n).unwrap()).rows()[1][1],
        Value::Integer(4)
    );
}
#[test]
fn type_role_changes_the_output_metadata() {
    let mut n = node(None);
    set(
        &mut n,
        "type",
        "direction",
        ParamValue::Rows(vec![("x".into(), vec![Some("out".into())])]),
    );
    assert_eq!(
        data(nodes::r#type::build(&n).unwrap()).model().fields()[0].role,
        Role::Target
    );
    set(
        &mut n,
        "type",
        "measure_type",
        ParamValue::Rows(vec![("x".into(), vec![Some("set".into())])]),
    );
    assert_eq!(
        data(nodes::r#type::build(&n).unwrap()).model().fields()[0].measurement,
        Measurement::Nominal
    );
}
#[test]
fn filter_alias_changes_the_output_name() {
    let mut n = node(None);
    set(
        &mut n,
        "filter",
        "new_name",
        ParamValue::Rows(vec![("x".into(), vec![Some("renamed".into())])]),
    );
    assert_eq!(
        data(nodes::filter::build(&n).unwrap()).model().fields()[0].name,
        "renamed"
    );
}
#[test]
fn partition_name_changes_the_generated_column() {
    let mut n = node(None);
    set(
        &mut n,
        "partition",
        "new_name",
        ParamValue::Text("split".into()),
    );
    set(
        &mut n,
        "partition",
        "set_random_seed",
        ParamValue::Bool(true),
    );
    assert_eq!(
        data(nodes::partition::build(&n).unwrap()).model().fields()[1].name,
        "split"
    );
}
#[test]
fn var_file_header_setting_changes_the_reader() {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "dms-schema-{}-{}.csv",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, "x\n1\n2\n").unwrap();
    let mut n = node(None);
    set(
        &mut n,
        "var_file",
        "full_filename",
        ParamValue::Text(path.to_string_lossy().into_owned()),
    );
    assert_eq!(
        nodes::var_file::build(&n)
            .unwrap()
            .run(&[], &ExecContext::default())
            .unwrap()
            .row_count(),
        2
    );
    set(
        &mut n,
        "var_file",
        "read_field_names",
        ParamValue::Bool(false),
    );
    let output = nodes::var_file::build(&n)
        .unwrap()
        .run(&[], &ExecContext::default())
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(output.row_count(), 3);
}
#[test]
fn statistics_file_path_reaches_the_reader() {
    let mut n = node(None);
    set(
        &mut n,
        "statistics_file",
        "full_filename",
        ParamValue::Text("schema-missing-unique.sav".into()),
    );
    let error = format!(
        "{:?}",
        nodes::statistics_file::build(&n)
            .unwrap()
            .run(&[], &ExecContext::default())
            .unwrap_err()
    );
    assert!(error.contains("schema-missing-unique.sav"), "{error}");
    set(
        &mut n,
        "statistics_file",
        "import_names",
        ParamValue::Text("labelsAsNames".into()),
    );
    assert!(nodes::statistics_file::build(&n).is_ok());
}
#[test]
fn table_has_no_settings_consumed_by_the_executor() {
    assert!(schema("table").unwrap().is_empty());
    assert_eq!(data(nodes::table::build(&node(None)).unwrap()), *input());
}
fn document(output: NodeOutput) -> Arc<crate::Document> {
    output.document().unwrap().clone()
}
#[test]
fn histogram_bin_count_changes_the_document() {
    let mut n = node(Some(xml(
        "<properties><graphXField>x</graphXField></properties>",
    )));
    set(&mut n, "histogram", "num_bins", ParamValue::Int(2));
    let output = nodes::histogram::build(&n)
        .unwrap()
        .produce(&[input()], None, &ExecContext::default())
        .unwrap();
    assert_eq!(document(output).tables[0].rows.len(), 2);
}
#[test]
fn data_audit_extra_statistics_change_the_columns() {
    let mut n = node(None);
    let before = document(
        nodes::data_audit::build(&n)
            .unwrap()
            .produce(&[input()], None, &ExecContext::default())
            .unwrap(),
    );
    set(
        &mut n,
        "data_audit",
        "advanced_stats",
        ParamValue::Bool(true),
    );
    let after = document(
        nodes::data_audit::build(&n)
            .unwrap()
            .produce(&[input()], None, &ExecContext::default())
            .unwrap(),
    );
    assert!(after.tables[0].header.len() > before.tables[0].header.len());
}
#[test]
fn analysis_detection_method_reaches_the_executor() {
    let mut n = node(None);
    set(
        &mut n,
        "analysis",
        "field_detection_method",
        ParamValue::Text("name".into()),
    );
    assert!(nodes::analysis::build(&n).is_ok());
    set(&mut n, "analysis", "performance", ParamValue::Bool(true));
    let error = format!("{:?}", nodes::analysis::build(&n).err().unwrap());
    assert!(error.contains("appendTagField"));
}
#[test]
fn logistic_intercept_changes_the_trained_model() {
    let mut x = Field::new("x", Storage::Real);
    x.measurement = Measurement::Continuous;
    let mut y = Field::new("y", Storage::String);
    y.measurement = Measurement::Nominal;
    y.role = Role::Target;
    let rows = (0..60)
        .map(|i| {
            vec![
                Value::Real(f64::from(i % 10)),
                Value::string(if i % 5 == 0 || i % 10 > 6 {
                    "yes"
                } else {
                    "no"
                }),
            ]
        })
        .collect();
    let input = Arc::new(Table::new(DataModel::new(vec![x, y]).unwrap(), rows).unwrap());
    let mut n = node(None);
    n.class = "BuildLogisticNode".into();
    let before = nodes::build::build(&n)
        .unwrap()
        .produce(std::slice::from_ref(&input), None, &ExecContext::default())
        .unwrap();
    set(
        &mut n,
        "logistic",
        "include_constant",
        ParamValue::Bool(false),
    );
    let after = nodes::build::build(&n)
        .unwrap()
        .produce(&[input], None, &ExecContext::default())
        .unwrap();
    assert_ne!(before.model(), after.model());
}
fn extension_model(
    id: &str,
    key: &str,
    before: ParamValue,
    after: ParamValue,
) -> (Arc<dms_pmml::Pmml>, Arc<dms_pmml::Pmml>) {
    let linear = id == "linear";
    let mut x = Field::new("x", Storage::Real);
    x.measurement = Measurement::Continuous;
    let mut y = Field::new(
        "y",
        if linear {
            Storage::Real
        } else {
            Storage::String
        },
    );
    y.measurement = if linear {
        Measurement::Continuous
    } else {
        Measurement::Nominal
    };
    y.role = Role::Target;
    let rows = (0..100)
        .map(|i| {
            vec![
                Value::Real(f64::from(i % 10)),
                if linear {
                    Value::Real(3.0 + f64::from(i % 10) * 2.0)
                } else {
                    Value::string(if i % 10 < 5 { "a" } else { "b" })
                },
            ]
        })
        .collect();
    let input = Arc::new(Table::new(DataModel::new(vec![x, y]).unwrap(), rows).unwrap());
    let mut n = node(None);
    n.class = "CFBuildModelNode".into();
    n.extension = Some(Extension::Object {
        object_id: if linear {
            "LinearRegression"
        } else if id == "cart" {
            "Cart"
        } else {
            "Chaid"
        }
        .into(),
        extension_id: if linear {
            "spss.alm-extension"
        } else {
            "spss.tree-extension"
        }
        .into(),
    });
    n.properties_attachment = Some("params.xml".into());
    let initial=b"<?xml version='1.0'?><Parameters xmlns='http://xml.spss.com/pasw/extension'><Parameter name='MaxTreeDepth' value='Custom'/><Parameter name='tree.UsePercentageForNodeSizeRequirement' value='absolute'/><Parameter name='tree.MinParentRecordsABS' value='2'/><Parameter name='tree.MinChildRecordsABS' value='1'/><Parameter name='tree.PruneTree' value='false'/><Parameter name='linreg.AutoDataPreparation' value='false'/><Parameter name='linreg.VarSelectionMethod' value='none'/></Parameters>";
    let p = spec(id, key);
    let one = write_attachment(initial, p, &before).unwrap();
    assert_eq!(read_attachment(Some(&one), p).unwrap(), before);
    let two = write_attachment(&one, p, &after).unwrap();
    assert_eq!(read_attachment(Some(&two), p).unwrap(), after);
    let run = |bytes: Vec<u8>| {
        let context = ExecContext {
            attachments: Arc::new(
                [("params.xml".into(), Arc::from(bytes))]
                    .into_iter()
                    .collect(),
            ),
            ..Default::default()
        };
        nodes::build::build(&n)
            .unwrap()
            .produce(std::slice::from_ref(&input), None, &context)
            .unwrap()
            .model()
            .unwrap()
            .clone()
    };
    (run(one), run(two))
}
#[test]
fn chaid_depth_changes_the_trained_tree() {
    let (before, after) =
        extension_model("chaid", "max_depth", ParamValue::Int(0), ParamValue::Int(2));
    let dms_pmml::Model::Tree(before) = &before.model else {
        panic!("tree")
    };
    let dms_pmml::Model::Tree(after) = &after.model else {
        panic!("tree")
    };
    assert!(before.root.children.is_empty());
    assert!(!after.root.children.is_empty());
}
#[test]
fn cart_depth_changes_the_trained_tree() {
    let (before, after) =
        extension_model("cart", "max_depth", ParamValue::Int(0), ParamValue::Int(2));
    let dms_pmml::Model::Tree(before) = &before.model else {
        panic!("tree")
    };
    let dms_pmml::Model::Tree(after) = &after.model else {
        panic!("tree")
    };
    assert!(before.root.children.is_empty());
    assert!(!after.root.children.is_empty());
}
#[test]
fn linear_constant_changes_the_trained_model() {
    let (before, after) = extension_model(
        "linear",
        "include_constant",
        ParamValue::Bool(true),
        ParamValue::Bool(false),
    );
    assert_ne!(before, after);
}

#[test]
fn preserves_unknown_nodes_and_real_roundtrip_spelling() {
    let original = xml(
        "<properties z='keep'><future q='1'>x<!--keep--></future><analysisImproveAccuracy type='D'>2.0</analysisImproveAccuracy><tail/></properties>",
    );
    let p = spec("analysis", "improve_accuracy");
    assert_eq!(
        write(Some(&original), p, &read(Some(&original), p).unwrap()).unwrap(),
        original
    );
    let changed = write(Some(&original), p, &ParamValue::Real(3.0)).unwrap();
    assert_eq!(settings::real(Some(&changed), p.path).unwrap(), Some(3.0));
    assert_eq!(
        settings::text(Some(&changed), p.path).as_deref(),
        Some("3.0")
    );
    assert_eq!(original.child("future"), changed.child("future"));
    assert_eq!(original.child("tail"), changed.child("tail"));
    let appended = write(
        Some(&original),
        spec("select", "condition"),
        &ParamValue::Text("x = 2".into()),
    )
    .unwrap();
    assert_eq!(
        appended
            .child_elements()
            .last()
            .unwrap()
            .name
            .local
            .as_ref(),
        "selectCondition"
    );
    for value in [f64::MIN_POSITIVE, f64::MAX, 1e-100, 0.125] {
        let changed = write(
            None,
            spec("derive", "count_initial_val"),
            &ParamValue::Real(value),
        )
        .unwrap();
        assert_eq!(
            settings::real(Some(&changed), "deriveCountInitialValue").unwrap(),
            Some(value)
        );
    }
}
#[test]
fn attachment_splices_only_the_parameter_and_preserves_list_metadata() {
    let p = spec("chaid", "max_depth");
    let original=b"<?xml version='1.0'?><p:Parameters xmlns:p='urn:p'>\r\n<!--keep--><p:Parameter name = 'tree.MaxTreeDepth' value = '5' extra='same'/><p:Parameter name='future' value='&amp;'/></p:Parameters>";
    let changed = write_attachment(original, p, &ParamValue::Int(8)).unwrap();
    assert_eq!(
        String::from_utf8(changed).unwrap(),
        String::from_utf8(original.to_vec())
            .unwrap()
            .replace("value = '5'", "value = '8'")
    );
    let list = spec("chaid", "inputs");
    let bytes=b"<Parameters><Parameter name='tree.InputFieldList'><ListValue><!--keep--><Value tag='keep' value='a'/><Unknown/></ListValue></Parameter><Parameter name='future'/></Parameters>";
    let changed = write_attachment(
        bytes,
        list,
        &ParamValue::Fields(vec!["b&\"\n".into(), "c".into()]),
    )
    .unwrap();
    assert_eq!(
        read_attachment(Some(&changed), list).unwrap(),
        ParamValue::Fields(vec!["b&\"\n".into(), "c".into()])
    );
    let text = std::str::from_utf8(&changed).unwrap();
    assert!(text.contains("tag='keep'"));
    assert!(text.contains("<!--keep-->"));
    assert!(text.contains("<Unknown/>"));
    assert!(text.contains("<Parameter name='future'/>"));
}
#[test]
fn limits_and_malformed_values_are_handled_without_mutation() {
    assert!(validate(spec("partition", "training_size"), &ParamValue::Int(101)).is_err());
    assert!(
        validate(
            spec("select", "condition"),
            &ParamValue::Text("x".repeat(MAX_TEXT + 1))
        )
        .is_err()
    );
    assert!(validate(spec("analysis", "threshold"), &ParamValue::Real(f64::NAN)).is_err());
    assert!(
        read(
            Some(&xml(
                "<properties><selectMode>unknown</selectMode></properties>"
            )),
            spec("select", "mode")
        )
        .is_err()
    );
    for bytes in [b"not xml".as_slice(),b"<!DOCTYPE a [<!ENTITY x 'v'>]><a>&x;</a>",b"<a><Parameter name='tree.MaxTreeDepth' value='NaN'/></a>",b"<a><Parameter name='tree.MaxTreeDepth' value='3'/><Parameter name='tree.MaxTreeDepth' value='4'/></a>"] {
        assert!(read_attachment(Some(bytes),spec("chaid","max_depth")).is_err());
    }
    assert!(
        write_attachment(
            format!("{}{}", "<p>".repeat(513), "</p>".repeat(513)).as_bytes(),
            spec("chaid", "max_depth"),
            &ParamValue::Int(3)
        )
        .is_err()
    );
    assert!(
        validate(
            spec("type", "direction"),
            &ParamValue::Rows(vec![("x".into(), vec![Some("bad".into())])])
        )
        .is_err()
    );
    assert!(
        validate(
            spec("type", "direction"),
            &ParamValue::Rows((0..10001).map(|i| (i.to_string(), vec![None])).collect())
        )
        .is_err()
    );
}

#[test]
fn clearing_one_table_parameter_keeps_other_columns_and_unknown_rows() {
    let original = xml(
        "<properties><typeTable><future/><rows><row><name>x</name><direction>in</direction><measure>range</measure><unknown k='v'/></row></rows></typeTable><tail/></properties>",
    );
    let changed = write(
        Some(&original),
        spec("type", "direction"),
        &ParamValue::Unset,
    )
    .unwrap();
    let row = settings::element(Some(&changed), "typeTable/rows/row").unwrap();
    assert!(row.child("direction").is_none());
    assert_eq!(row.child("measure").unwrap().text(), "range");
    assert!(row.child("unknown").is_some());
    assert!(settings::element(Some(&changed), "typeTable/future").is_some());
    assert_eq!(original.child("tail"), changed.child("tail"));
}

#[test]
fn attachment_append_unset_and_empty_optional_scalars_keep_other_bytes() {
    let p = spec("linear", "max_steps");
    let bytes=b"<p:Parameters xmlns:p='urn:test'><p:Parameter name='linreg.MaxSteps'><p:ListValue/></p:Parameter><p:Parameter name='future' value='v'/></p:Parameters>";
    assert_eq!(read_attachment(Some(bytes), p).unwrap(), ParamValue::Unset);
    let changed = write_attachment(bytes, p, &ParamValue::Int(4)).unwrap();
    assert_eq!(
        read_attachment(Some(&changed), p).unwrap(),
        ParamValue::Int(4)
    );
    assert!(
        std::str::from_utf8(&changed)
            .unwrap()
            .contains("<p:ListValue/>")
    );
    let added = write_attachment(bytes, spec("chaid", "max_depth"), &ParamValue::Int(7)).unwrap();
    assert!(
        std::str::from_utf8(&added)
            .unwrap()
            .contains("<p:Parameter name=\"tree.MaxTreeDepth\" value=\"7\"/>")
    );
    assert_eq!(
        write_attachment(&added, spec("chaid", "max_depth"), &ParamValue::Unset).unwrap(),
        bytes
    );
    let long_space = format!(
        "<Parameters><Parameter name='tree.MaxTreeDepth' value{}='5'/></Parameters>",
        " ".repeat(300)
    );
    let changed = write_attachment(
        long_space.as_bytes(),
        spec("chaid", "max_depth"),
        &ParamValue::Int(8),
    )
    .unwrap();
    assert_eq!(
        std::str::from_utf8(&changed).unwrap(),
        long_space.replace("='5'", "='8'")
    );
}

#[test]
fn malformed_attachment_mutations_and_item_limits_return_errors() {
    let seed = b"<Parameters><Parameter name='tree.MaxTreeDepth' value='5'/></Parameters>";
    for i in 0..seed.len() {
        for byte in [0, b'\'', b'&', b'<', 0xff] {
            let mut mutated = seed.to_vec();
            mutated[i] = byte;
            let _ = read_attachment(Some(&mutated), spec("chaid", "max_depth"));
            let _ = write_attachment(&mutated, spec("chaid", "max_depth"), &ParamValue::Int(6));
        }
    }
    let huge = xml(&format!(
        "<properties>{}</properties>",
        "<v/>".repeat(MAX_ITEMS + 1)
    ));
    assert!(read(Some(&huge), spec("select", "condition")).is_err());
    let attributes = format!(
        "<p {} />",
        (0..=MAX_ITEMS)
            .map(|i| format!("a{i}='x'"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    assert!(read_attachment(Some(attributes.as_bytes()), spec("chaid", "max_depth")).is_err());
}

#[test]
fn attachment_namespace_bindings_stay_valid_when_appending() {
    let bytes =
        b"<Parameters><p:Parameter xmlns:p='urn:local' name='future' value='v'/></Parameters>";
    let p = spec("chaid", "inputs");
    let changed = write_attachment(bytes, p, &ParamValue::Fields(vec!["x".into()])).unwrap();
    assert_eq!(
        read_attachment(Some(&changed), p).unwrap(),
        ParamValue::Fields(vec!["x".into()])
    );
    let text = std::str::from_utf8(&changed).unwrap();
    assert!(
        text.starts_with("<Parameters><p:Parameter xmlns:p='urn:local' name='future' value='v'/>")
    );
    let bytes=b"<p:Parameters xmlns:p='urn:p' xmlns:q='urn:p'><p:Parameter name='tree.InputFieldList'><q:ListValue/></p:Parameter></p:Parameters>";
    let changed = write_attachment(bytes, p, &ParamValue::Fields(vec!["x".into()])).unwrap();
    assert_eq!(
        read_attachment(Some(&changed), p).unwrap(),
        ParamValue::Fields(vec!["x".into()])
    );
}

#[test]
fn empty_inherited_choice_cells_do_not_block_another_field_edit() {
    let original = xml(
        "<properties><typeTable><rows><row><name>x</name><direction/></row><row><name>y</name><direction>in</direction></row></rows></typeTable></properties>",
    );
    let p = spec("type", "direction");
    let ParamValue::Rows(mut rows) = read(Some(&original), p).unwrap() else {
        panic!("rows")
    };
    rows[1].1[0] = Some("out".into());
    let changed = write(Some(&original), p, &ParamValue::Rows(rows)).unwrap();
    assert_eq!(
        settings::children(Some(&changed), "typeTable/rows", "row")[0],
        settings::children(Some(&original), "typeTable/rows", "row")[0]
    );
    assert_eq!(
        settings::children(Some(&changed), "typeTable/rows", "row")[1]
            .child("direction")
            .unwrap()
            .text(),
        "out"
    );
}

#[test]
fn explicit_real_changes_preserve_the_sign_of_zero() {
    let p = spec("derive", "count_initial_val");
    let changed = write(None, p, &ParamValue::Real(-0.0)).unwrap();
    assert_eq!(
        settings::real(Some(&changed), p.path)
            .unwrap()
            .unwrap()
            .to_bits(),
        (-0.0f64).to_bits()
    );
    let p = spec("chaid", "split_alpha");
    let changed = write_attachment(
        b"<Parameters><Parameter name='tree.SplitAlpha' value='0.0'/></Parameters>",
        p,
        &ParamValue::Real(-0.0),
    )
    .unwrap();
    assert!(
        std::str::from_utf8(&changed)
            .unwrap()
            .contains("value='-0.0'")
    );
}
