//! Ensemble on synthetic flag tables: each method rule, ties, blanks, and rejected settings.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use dms_exec::{Arity, ExecContext, Operator, OperatorError, registry};
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::{Element, Node, QualifiedName};
use dms_types::{
    DataModel, Field, Measurement, MissingSpec, Role, Storage, Table, Value, ValueSpec,
};
use std::sync::Arc;

fn e(name: &str, children: Vec<Node>) -> Node {
    Node::Element(Element {
        name: QualifiedName {
            prefix: None,
            namespace: None,
            local: Arc::from(name),
        },
        namespace_declarations: Vec::new(),
        attributes: Vec::new(),
        children,
    })
}
fn p(name: &str, text: &str) -> Node {
    e(name, vec![Node::Text(Arc::from(text))])
}
fn node(uid: &str, class: &str, settings: Vec<Node>) -> StreamNode {
    let Node::Element(settings) = e("properties", settings) else {
        panic!("element")
    };
    StreamNode {
        uid: uid.into(),
        class: class.into(),
        extension: None,
        model_builder: None,
        name: None,
        label: None,
        position: None,
        properties_attachment: None,
        containers: Vec::new(),
        settings: Some(settings),
        binning_output: None,
        result_items: Vec::new(),
        subdiagram: None,
    }
}

fn s(text: &str) -> Value {
    Value::string(text)
}
fn r(value: f64) -> Value {
    Value::real(value)
}
fn respond() -> Field {
    let mut field = Field::new("respond", Storage::String);
    field.measurement = Measurement::Flag;
    field.role = Role::Target;
    field.values = ValueSpec::Flag {
        true_value: s("Y"),
        false_value: s("N"),
    };
    field
}
fn column(name: &str, storage: Storage) -> Field {
    Field::new(name, storage)
}
fn table(fields: Vec<Field>, rows: Vec<Vec<Value>>) -> Table {
    Table::new(DataModel::new(fields).unwrap(), rows).unwrap()
}

/// The shape saved on every `ens_*` ensemble node, with the method under test.
fn settings(method: &str, tie: &str, filter: bool) -> Vec<Node> {
    vec![
        p(
            "filterEnsembledFields",
            if filter { "true" } else { "false" },
        ),
        p("calculateStandardError", "true"),
        p("ensembleTargetField", "respond"),
        p("ensembleFlagTiesMethod", tie),
        p("ensembleFlagsMethod", method),
        p("ensembleSetsMethod", "confidenceWeightedVoting"),
        p("ensembleSetTiesMethod", "random"),
        p("enabled", "true"),
        p("cacheEnabled", "false"),
        p("autoLabel", "true"),
        p("name", "Train ensemble"),
        e("position", vec![p("x", "1200"), p("y", "80")]),
    ]
}

fn run(method: &str, tie: &str, filter: bool, input: Table) -> Result<Table, OperatorError> {
    run_with(settings(method, tie, filter), input)
}
fn run_with(properties: Vec<Node>, input: Table) -> Result<Table, OperatorError> {
    dms_exec::nodes::ensemble::build(&node("e", "EnsembleNode", properties))?
        .run(&[Arc::new(input)], &ExecContext::default())
}
fn names(table: &Table) -> Vec<&str> {
    table
        .model()
        .fields()
        .iter()
        .map(|field| field.name.as_str())
        .collect()
}
fn cell(table: &Table, row: usize, name: &str) -> Value {
    let position = table.model().position(name).expect(name);
    table
        .rows()
        .get(row)
        .and_then(|values| values.get(position))
        .cloned()
        .unwrap_or(Value::Null)
}
fn near(value: &Value, expected: f64) {
    let actual = value.as_f64().expect("real");
    assert!((actual - expected).abs() < 1e-12, "{actual} != {expected}");
}
fn message(error: OperatorError) -> String {
    match error {
        OperatorError::Cancelled => "cancelled".into(),
        OperatorError::Settings(message)
        | OperatorError::Inputs(message)
        | OperatorError::Io(message)
        | OperatorError::Data(message) => message,
    }
}

fn pair_input(predictions: &[(&str, &str)], confidences: &[f64]) -> Table {
    table(
        vec![
            respond(),
            column("$C-respond", Storage::String),
            column("$CC-respond", Storage::Real),
            column("$R-respond", Storage::String),
            column("$RC-respond", Storage::Real),
        ],
        predictions
            .iter()
            .zip(confidences.chunks(2))
            .map(|((left, right), scores)| {
                vec![
                    s("N"),
                    s(left),
                    r(*scores.first().unwrap_or(&0.0)),
                    s(right),
                    r(*scores.get(1).unwrap_or(&0.0)),
                ]
            })
            .collect(),
    )
}

#[test]
fn registry_lists_ensemble() {
    assert!(registry::is_implemented("ensemble"));
    let operator: Box<dyn Operator> = dms_exec::nodes::ensemble::build(&node(
        "e",
        "EnsembleNode",
        settings("voting", "highestConfidence", false),
    ))
    .unwrap();
    assert_eq!(operator.arity(), Arity::Exactly(1));
}

#[test]
fn voting_unanimous_confidence_is_the_vote_share() {
    // ens_voting train id 1: both models predict Y at 0.987804878048780, and $XFC is 1.
    let input = pair_input(
        &[("Y", "Y")],
        &[0.987_804_878_048_780, 0.987_804_878_048_780],
    );
    let out = run("voting", "highestConfidence", false, input).unwrap();
    assert_eq!(
        names(&out),
        [
            "respond",
            "$C-respond",
            "$CC-respond",
            "$R-respond",
            "$RC-respond",
            "$XF-respond",
            "$XFC-respond",
        ]
    );
    assert_eq!(cell(&out, 0, "$XF-respond"), s("Y"));
    near(&cell(&out, 0, "$XFC-respond"), 1.0);
    let prediction = out.model().field("$XF-respond").unwrap();
    assert_eq!(prediction.measurement, Measurement::Flag);
    assert_eq!(prediction.role, Role::None);
    assert_eq!(prediction.storage, Storage::String);
    match &prediction.values {
        ValueSpec::Flag {
            true_value,
            false_value,
        } => {
            assert_eq!(true_value.as_str(), Some("Y"));
            assert_eq!(false_value.as_str(), Some("N"));
        }
        other => panic!("{other:?}"),
    }
    let confidence = out.model().field("$XFC-respond").unwrap();
    assert_eq!(confidence.measurement, Measurement::Continuous);
    assert_eq!(confidence.storage, Storage::Real);
    assert_eq!(confidence.role, Role::None);
}

#[test]
fn voting_two_of_three_is_two_thirds() {
    let input = table(
        vec![
            respond(),
            column("$C-respond", Storage::String),
            column("$R-respond", Storage::String),
            column("$L-respond", Storage::String),
        ],
        vec![vec![s("N"), s("Y"), s("Y"), s("N")]],
    );
    let out = run("voting", "highestConfidence", false, input).unwrap();
    assert_eq!(cell(&out, 0, "$XF-respond"), s("Y"));
    near(&cell(&out, 0, "$XFC-respond"), 2.0 / 3.0);
}

#[test]
fn voting_tie_highest_confidence_picks_that_side_with_half_the_votes() {
    let input = pair_input(&[("Y", "N")], &[0.9, 0.2]);
    let out = run("voting", "highestConfidence", false, input).unwrap();
    assert_eq!(cell(&out, 0, "$XF-respond"), s("Y"));
    near(&cell(&out, 0, "$XFC-respond"), 0.5);
}

#[test]
fn voting_tie_without_confidence_stays_a_gap() {
    let input = table(
        vec![
            respond(),
            column("$C-respond", Storage::String),
            column("$R-respond", Storage::String),
        ],
        vec![vec![s("N"), s("Y"), s("N")]],
    );
    let error = message(run("voting", "highestConfidence", false, input).unwrap_err());
    assert!(error.contains("GAP:"), "{error}");
}

#[test]
fn voting_tie_random_raw_and_adjusted_are_gaps_only_when_a_tie_occurs() {
    let tied = table(
        vec![
            respond(),
            column("$C-respond", Storage::String),
            column("$R-respond", Storage::String),
        ],
        vec![vec![s("N"), s("Y"), s("N")]],
    );
    let agreed = table(
        vec![
            respond(),
            column("$C-respond", Storage::String),
            column("$R-respond", Storage::String),
        ],
        vec![vec![s("N"), s("Y"), s("Y")]],
    );
    for tie in ["random", "rawPropensity", "adjustedPropensity"] {
        let error = message(run("voting", tie, false, tied.clone()).unwrap_err());
        assert!(error.contains("GAP:"), "{tie}: {error}");
        let out = run("voting", tie, false, agreed.clone()).unwrap();
        assert_eq!(cell(&out, 0, "$XF-respond"), s("Y"));
        near(&cell(&out, 0, "$XFC-respond"), 1.0);
    }
}

#[test]
fn confidence_weighted_agreeing_labels_match_ens_confidence() {
    // ens_confidence train id 196: both predict N. $XFC is the mean of the confidences.
    let input = pair_input(
        &[("N", "N")],
        &[0.982_142_857_142_857, 0.996_932_515_337_423],
    );
    let out = run(
        "confidenceWeightedVoting",
        "highestConfidence",
        false,
        input,
    )
    .unwrap();
    assert_eq!(cell(&out, 0, "$XF-respond"), s("N"));
    near(&cell(&out, 0, "$XFC-respond"), 0.989_537_686_240_140);
}

#[test]
fn confidence_weighted_disagreement_uses_the_full_posterior() {
    // Y at 0.8 and N at 0.6: P(Y) = 1.2, P(N) = 0.8, so Y with confidence 0.6.
    // A hard-vote weight would report 0.4.
    let input = pair_input(&[("Y", "N")], &[0.8, 0.6]);
    let out = run(
        "confidenceWeightedVoting",
        "highestConfidence",
        false,
        input,
    )
    .unwrap();
    assert_eq!(cell(&out, 0, "$XF-respond"), s("Y"));
    near(&cell(&out, 0, "$XFC-respond"), 0.6);
}

#[test]
fn confidence_weighted_three_models_can_overturn_the_hard_vote() {
    // Y 0.51, N 0.50, N 0.50. Hard votes pick N. Posteriors pick Y: 1.51 against 1.49.
    let input = table(
        vec![
            respond(),
            column("$C-respond", Storage::String),
            column("$CC-respond", Storage::Real),
            column("$R-respond", Storage::String),
            column("$RC-respond", Storage::Real),
            column("$L-respond", Storage::String),
            column("$LP-respond", Storage::Real),
        ],
        vec![vec![
            s("N"),
            s("Y"),
            r(0.51),
            s("N"),
            r(0.50),
            s("N"),
            r(0.50),
        ]],
    );
    let out = run(
        "confidenceWeightedVoting",
        "highestConfidence",
        false,
        input,
    )
    .unwrap();
    assert_eq!(cell(&out, 0, "$XF-respond"), s("Y"));
    near(&cell(&out, 0, "$XFC-respond"), 1.51 / 3.0);
}

#[test]
fn raw_weighted_agreeing_row_matches_ens_raw_vote_and_can_differ_from_confidence() {
    let agreed = table(
        vec![
            respond(),
            column("$CRP-respond", Storage::Real),
            column("$RRP-respond", Storage::Real),
        ],
        vec![vec![
            s("N"),
            r(0.017_857_142_857_143),
            r(0.003_067_484_662_577),
        ]],
    );
    let out = run(
        "rawPropensityWeightedVoting",
        "highestConfidence",
        false,
        agreed,
    )
    .unwrap();
    assert_eq!(
        names(&out),
        [
            "respond",
            "$CRP-respond",
            "$RRP-respond",
            "$XF-respond",
            "$XFC-respond",
        ]
    );
    assert_eq!(cell(&out, 0, "$XF-respond"), s("N"));
    near(&cell(&out, 0, "$XFC-respond"), 0.989_537_686_240_140);

    // Both hard predictions are Y at 0.9, but each raw propensity is 0.2.
    let diverged = table(
        vec![
            respond(),
            column("$C-respond", Storage::String),
            column("$CC-respond", Storage::Real),
            column("$CRP-respond", Storage::Real),
            column("$R-respond", Storage::String),
            column("$RC-respond", Storage::Real),
            column("$RRP-respond", Storage::Real),
        ],
        vec![vec![s("N"), s("Y"), r(0.9), r(0.2), s("Y"), r(0.9), r(0.2)]],
    );
    let weighted = run(
        "confidenceWeightedVoting",
        "highestConfidence",
        false,
        diverged.clone(),
    )
    .unwrap();
    assert_eq!(cell(&weighted, 0, "$XF-respond"), s("Y"));
    near(&cell(&weighted, 0, "$XFC-respond"), 0.9);
    let raw = run(
        "rawPropensityWeightedVoting",
        "highestConfidence",
        false,
        diverged,
    )
    .unwrap();
    assert_eq!(cell(&raw, 0, "$XF-respond"), s("N"));
    near(&cell(&raw, 0, "$XFC-respond"), 0.8);
    assert!(raw.model().field("$XFRP-respond").is_none());
}

#[test]
fn highest_confidence_matches_ens_highest_and_ignores_a_stored_random_tie() {
    let input = pair_input(
        &[("N", "N")],
        &[0.982_142_857_142_857, 0.996_932_515_337_423],
    );
    let out = run("highestConfidence", "random", false, input).unwrap();
    assert_eq!(cell(&out, 0, "$XF-respond"), s("N"));
    near(&cell(&out, 0, "$XFC-respond"), 0.996_932_515_337_423);
}

#[test]
fn highest_confidence_same_label_tie_is_kept_and_a_later_winner_clears_a_conflict() {
    let agreed_input = pair_input(&[("Y", "Y")], &[0.8, 0.8]);
    let same = table(
        vec![
            respond(),
            column("$C-respond", Storage::String),
            column("$CC-respond", Storage::Real),
            column("$R-respond", Storage::String),
            column("$RC-respond", Storage::Real),
            column("$L-respond", Storage::String),
            column("$LP-respond", Storage::Real),
        ],
        vec![vec![s("N"), s("Y"), r(0.5), s("N"), r(0.5), s("Y"), r(0.9)]],
    );
    let agreed = run("highestConfidence", "random", false, agreed_input).unwrap();
    assert_eq!(cell(&agreed, 0, "$XF-respond"), s("Y"));
    near(&cell(&agreed, 0, "$XFC-respond"), 0.8);
    let cleared = run("highestConfidence", "random", false, same).unwrap();
    assert_eq!(cell(&cleared, 0, "$XF-respond"), s("Y"));
    near(&cell(&cleared, 0, "$XFC-respond"), 0.9);
}

#[test]
fn highest_confidence_collision_is_a_gap() {
    let input = pair_input(&[("Y", "N")], &[0.8, 0.8]);
    let error = message(run("highestConfidence", "random", false, input).unwrap_err());
    assert!(error.contains("GAP:"), "{error}");
    assert!(error.contains("ens_highest"), "{error}");
}

#[test]
fn average_raw_matches_ens_avg_raw_and_treats_one_half_as_false() {
    let input = table(
        vec![
            respond(),
            column("$CRP-respond", Storage::Real),
            column("$RRP-respond", Storage::Real),
        ],
        vec![
            vec![s("N"), r(0.017_857_142_857_143), r(0.003_067_484_662_577)],
            vec![s("Y"), r(0.25), r(0.75)],
            vec![s("Y"), r(0.4), r(0.7)],
        ],
    );
    let out = run("averageRawPropensity", "random", false, input).unwrap();
    assert_eq!(
        names(&out),
        [
            "respond",
            "$CRP-respond",
            "$RRP-respond",
            "$XF-respond",
            "$XFRP-respond",
        ]
    );
    assert!(out.model().field("$XFC-respond").is_none());
    assert_eq!(cell(&out, 0, "$XF-respond"), s("N"));
    near(&cell(&out, 0, "$XFRP-respond"), 0.010_462_313_759_860);
    assert_eq!(cell(&out, 1, "$XF-respond"), s("N"));
    near(&cell(&out, 1, "$XFRP-respond"), 0.5);
    assert_eq!(cell(&out, 2, "$XF-respond"), s("Y"));
    near(&cell(&out, 2, "$XFRP-respond"), 0.55);
}

#[test]
fn adjusted_methods_fail_without_columns_and_average_the_synthetic_ones() {
    let raw_only = table(
        vec![respond(), column("$CRP-respond", Storage::Real)],
        vec![vec![s("Y"), r(0.9)]],
    );
    for method in [
        "adjustedPropensityWeightedVoting",
        "averageAdjustedPropensity",
    ] {
        let error = message(run(method, "highestConfidence", false, raw_only.clone()).unwrap_err());
        assert_eq!(
            error, "No valid models predicting the target field 'respond' available for ensemble",
            "{method}: {error}"
        );
    }
    let adjusted = table(
        vec![
            respond(),
            column("$CAP-respond", Storage::Real),
            column("$RAP-respond", Storage::Real),
        ],
        vec![vec![s("N"), r(0.2), r(0.8)], vec![s("N"), r(0.8), r(0.9)]],
    );
    let mean = run("averageAdjustedPropensity", "random", false, adjusted).unwrap();
    assert_eq!(
        names(&mean),
        [
            "respond",
            "$CAP-respond",
            "$RAP-respond",
            "$XF-respond",
            "$XFAP-respond",
        ]
    );
    assert_eq!(cell(&mean, 0, "$XF-respond"), s("N"));
    near(&cell(&mean, 0, "$XFAP-respond"), 0.5);
    assert_eq!(cell(&mean, 1, "$XF-respond"), s("Y"));
    near(&cell(&mean, 1, "$XFAP-respond"), 0.85);
    let weighted_input = table(
        vec![
            respond(),
            column("$CAP-respond", Storage::Real),
            column("$RAP-respond", Storage::Real),
        ],
        vec![vec![s("N"), r(0.2), r(0.4)], vec![s("N"), r(0.8), r(0.9)]],
    );
    let weighted = run(
        "adjustedPropensityWeightedVoting",
        "highestConfidence",
        false,
        weighted_input,
    )
    .unwrap();
    assert_eq!(cell(&weighted, 0, "$XF-respond"), s("N"));
    near(&cell(&weighted, 0, "$XFC-respond"), 0.7);
    assert_eq!(cell(&weighted, 1, "$XF-respond"), s("Y"));
    near(&cell(&weighted, 1, "$XFC-respond"), 0.85);
    assert!(weighted.model().field("$XFAP-respond").is_none());
}

#[test]
fn filter_drops_recognized_component_fields_and_keeps_neural_propensity() {
    let input = table(
        vec![
            column("id", Storage::Integer),
            respond(),
            column("$C-respond", Storage::String),
            column("$CC-respond", Storage::Real),
            column("$CRP-respond", Storage::Real),
            column("$RP-respond", Storage::Real),
        ],
        vec![vec![
            Value::Integer(1),
            s("N"),
            s("Y"),
            r(0.8),
            r(0.8),
            r(0.2),
        ]],
    );
    let out = run("voting", "highestConfidence", true, input).unwrap();
    assert_eq!(
        names(&out),
        [
            "id",
            "respond",
            "$RP-respond",
            "$XF-respond",
            "$XFC-respond",
        ]
    );
    assert_eq!(cell(&out, 0, "$XF-respond"), s("Y"));
    near(&cell(&out, 0, "$XFC-respond"), 1.0);
}

#[test]
fn neural_propensity_and_confidence_alone_are_not_models() {
    let propensity = table(
        vec![respond(), column("$RP-respond", Storage::Real)],
        vec![vec![s("Y"), r(0.9)]],
    );
    let confidence = table(
        vec![respond(), column("$CC-respond", Storage::Real)],
        vec![vec![s("Y"), r(0.9)]],
    );
    let indexed_zero = table(
        vec![respond(), column("$C0-respond", Storage::String)],
        vec![vec![s("Y"), s("Y")]],
    );
    for input in [propensity, confidence, indexed_zero] {
        let error = message(run("voting", "highestConfidence", false, input).unwrap_err());
        assert_eq!(
            error,
            "No valid models predicting the target field 'respond' available for ensemble"
        );
    }
    let raw = table(
        vec![respond(), column("$RP-respond", Storage::Real)],
        vec![vec![s("Y"), r(0.9)]],
    );
    let error = message(run("averageRawPropensity", "random", false, raw).unwrap_err());
    assert!(error.contains("No valid models"), "{error}");
}

#[test]
fn a_later_model_index_votes_and_an_existing_ensemble_field_takes_the_next_suffix() {
    let input = table(
        vec![
            respond(),
            column("$XF-respond", Storage::String),
            column("$C-respond", Storage::String),
            column("$C1-respond", Storage::String),
        ],
        vec![vec![s("N"), s("old"), s("Y"), s("Y")]],
    );
    let out = run("voting", "highestConfidence", false, input).unwrap();
    assert_eq!(
        names(&out),
        [
            "respond",
            "$XF-respond",
            "$C-respond",
            "$C1-respond",
            "$XF1-respond",
            "$XFC1-respond",
        ]
    );
    assert_eq!(cell(&out, 0, "$XF-respond"), s("old"));
    assert_eq!(cell(&out, 0, "$XF1-respond"), s("Y"));
    near(&cell(&out, 0, "$XFC1-respond"), 1.0);
}

#[test]
fn nulls_blanks_and_unknown_categories_abstain() {
    let mut blank = column("$C-respond", Storage::String);
    blank.missing = MissingSpec {
        values: vec![s("Y")],
        ..MissingSpec::default()
    };
    let input = table(
        vec![respond(), blank, column("$R-respond", Storage::String)],
        vec![
            vec![s("N"), Value::Null, Value::Null],
            vec![s("N"), Value::Null, s("Y")],
            vec![s("N"), s("Y"), s("N")],
            vec![s("N"), s("M"), s("N")],
        ],
    );
    let out = run("voting", "highestConfidence", false, input).unwrap();
    assert!(cell(&out, 0, "$XF-respond").is_null());
    assert!(cell(&out, 0, "$XFC-respond").is_null());
    assert_eq!(cell(&out, 1, "$XF-respond"), s("Y"));
    near(&cell(&out, 1, "$XFC-respond"), 1.0);
    assert_eq!(cell(&out, 2, "$XF-respond"), s("N"));
    near(&cell(&out, 2, "$XFC-respond"), 1.0);
    assert_eq!(cell(&out, 3, "$XF-respond"), s("N"));
    near(&cell(&out, 3, "$XFC-respond"), 1.0);
}

#[test]
fn voting_keeps_the_model_value_and_weighted_methods_keep_the_flag_value() {
    let mut target = Field::new("respond", Storage::Integer);
    target.measurement = Measurement::Flag;
    target.values = ValueSpec::Flag {
        true_value: Value::Integer(1),
        false_value: Value::Integer(0),
    };
    let input = table(
        vec![
            target,
            column("$C-respond", Storage::Real),
            column("$CC-respond", Storage::Real),
        ],
        vec![vec![Value::Integer(0), Value::Real(1.0), r(0.7)]],
    );
    let voted = run("voting", "highestConfidence", false, input.clone()).unwrap();
    assert_eq!(cell(&voted, 0, "$XF-respond"), Value::Real(1.0));
    let weighted = run(
        "confidenceWeightedVoting",
        "highestConfidence",
        false,
        input,
    )
    .unwrap();
    assert_eq!(cell(&weighted, 0, "$XF-respond"), Value::Integer(1));
}

#[test]
fn empty_input_keeps_the_schema_and_a_missing_target_is_a_data_error() {
    let empty = table(
        vec![respond(), column("$C-respond", Storage::String)],
        vec![],
    );
    let out = run("voting", "highestConfidence", false, empty).unwrap();
    assert_eq!(out.row_count(), 0);
    assert_eq!(
        names(&out),
        ["respond", "$C-respond", "$XF-respond", "$XFC-respond"]
    );
    let missing = table(
        vec![column("id", Storage::Integer)],
        vec![vec![Value::Integer(1)]],
    );
    let error = message(run("voting", "highestConfidence", false, missing).unwrap_err());
    assert!(error.contains("target field"), "{error}");
}

#[test]
fn non_flag_targets_and_a_flag_without_values_are_rejected() {
    let mut nominal = Field::new("respond", Storage::String);
    nominal.measurement = Measurement::Nominal;
    nominal.values = ValueSpec::Categories(vec![s("a"), s("b")]);
    let mut continuous = Field::new("respond", Storage::Real);
    continuous.measurement = Measurement::Continuous;
    let mut bare = Field::new("respond", Storage::String);
    bare.measurement = Measurement::Flag;
    let cases = [
        (nominal, "set target"),
        (continuous, "continuous"),
        (bare, "true/false"),
    ];
    for (field, needle) in cases {
        let input = table(
            vec![field, column("$C-respond", Storage::String)],
            vec![vec![s("a"), s("a")]],
        );
        let error = message(run("voting", "highestConfidence", false, input).unwrap_err());
        assert!(
            error.contains("GAP:") || error.contains(needle),
            "{needle}: {error}"
        );
        if needle != "true/false" {
            assert!(error.contains("GAP:"), "{needle}: {error}");
        }
    }
}

#[test]
fn unknown_settings_methods_and_arity_fail() {
    let input = table(
        vec![respond(), column("$C-respond", Storage::String)],
        vec![vec![s("Y"), s("Y")]],
    );
    let bad_method = message(run("Voting", "highestConfidence", false, input.clone()).unwrap_err());
    assert!(bad_method.contains("ensembleFlagsMethod"), "{bad_method}");
    let properties = vec![
        p("ensembleTargetField", "respond"),
        p("ensembleFlagsMethod", "voting"),
        p("ensembleSetsMethod", "plurality"),
    ];
    let bad_set = message(run_with(properties, input.clone()).unwrap_err());
    assert!(bad_set.contains("ensembleSetsMethod"), "{bad_set}");
    let bad_child = message(
        run_with(
            vec![
                p("ensembleTargetField", "respond"),
                p("ensembleFlagsMethod", "voting"),
                p("notASetting", "1"),
            ],
            input.clone(),
        )
        .unwrap_err(),
    );
    assert!(bad_child.contains("notASetting"), "{bad_child}");
    let missing_method =
        message(run_with(vec![p("ensembleTargetField", "respond")], input.clone()).unwrap_err());
    assert!(
        missing_method.contains("ensembleFlagsMethod"),
        "{missing_method}"
    );
    let bad_flag = message(
        run_with(
            vec![
                p("ensembleTargetField", "respond"),
                p("ensembleFlagsMethod", "voting"),
                p("calculateStandardError", "yes"),
            ],
            input.clone(),
        )
        .unwrap_err(),
    );
    assert!(bad_flag.contains("calculateStandardError"), "{bad_flag}");
    let operator = dms_exec::nodes::ensemble::build(&node(
        "e",
        "EnsembleNode",
        settings("voting", "highestConfidence", false),
    ))
    .unwrap();
    let error = message(operator.run(&[], &ExecContext::default()).unwrap_err());
    assert!(error.contains("one input"), "{error}");
    let mut bare = node("e", "EnsembleNode", vec![]);
    bare.settings = None;
    let Err(error) = dms_exec::nodes::ensemble::build(&bare) else {
        panic!("expected a settings error");
    };
    let error = message(error);
    assert!(error.contains("no settings"), "{error}");
}

#[test]
fn a_prediction_only_model_votes_and_is_skipped_by_average_raw() {
    let input = table(
        vec![
            respond(),
            column("$C-respond", Storage::String),
            column("$RRP-respond", Storage::Real),
        ],
        vec![vec![s("N"), s("Y"), r(0.1)]],
    );
    let voted = run("voting", "highestConfidence", false, input.clone()).unwrap();
    assert_eq!(cell(&voted, 0, "$XF-respond"), s("Y"));
    near(&cell(&voted, 0, "$XFC-respond"), 1.0);
    let averaged = run("averageRawPropensity", "random", false, input).unwrap();
    assert_eq!(cell(&averaged, 0, "$XF-respond"), s("N"));
    near(&cell(&averaged, 0, "$XFRP-respond"), 0.1);
}
