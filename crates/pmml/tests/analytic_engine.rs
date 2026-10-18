//! Synthetic PMML fixtures for the scoring rules observed in `models_as`.
#![allow(clippy::unwrap_used, clippy::panic, reason = "test assertions")]

use dms_pmml::{Pmml, Value};

fn document(model: &str, transformations: &str) -> Pmml {
    Pmml::parse(&format!(r#"<PMML version="4.3"><DataDictionary>
      <DataField name="x" optype="continuous" dataType="double"/>
      <DataField name="y" optype="categorical" dataType="string"><Value value="Y"/><Value value="N"/></DataField>
      </DataDictionary>{transformations}{model}</PMML>"#)).unwrap()
}

fn leaf(score: &str, confidence: f64, function: &str) -> String {
    format!(
        r#"<Segment><True/><TreeModel functionName="{function}">
      <MiningSchema><MiningField name="y" usageType="target"/></MiningSchema>
      <Node score="{score}"><True/><ScoreDistribution value="{score}" recordCount="10" confidence="{confidence}"/></Node>
      </TreeModel></Segment>"#
    )
}

fn ensemble(method: &str, segments: &str, function: &str) -> Pmml {
    document(
        &format!(
            r#"<MiningModel functionName="{function}">
      <MiningSchema><MiningField name="y" usageType="target"/></MiningSchema>
      <Segmentation multipleModelMethod="{method}">{segments}</Segmentation></MiningModel>"#
        ),
        "",
    )
}

#[test]
fn random_trees_vote_counts_trees_not_leaf_confidences() {
    // `rt_default`, `rt_shallow`: the winner's vote share, not averaged leaf confidence.
    let segments = [
        leaf("Y", 0.1, "classification"),
        leaf("N", 0.99, "classification"),
        leaf("Y", 0.1, "classification"),
    ]
    .concat();
    let pmml = ensemble("majorityVote", &segments, "classification");
    let prediction = pmml.bind(&[]).unwrap().score(&[]).unwrap();
    assert_eq!(prediction.value, Value::Text("Y".into()));
    assert_eq!(prediction.confidence, Some(2.0 / 3.0));
    assert_eq!(
        prediction.probabilities,
        [("Y".into(), 2.0 / 3.0), ("N".into(), 1.0 / 3.0)]
    );
    let tie = ensemble(
        "majorityVote",
        &(leaf("N", 1.0, "classification") + &leaf("Y", 1.0, "classification")),
        "classification",
    );
    assert_eq!(
        tie.bind(&[]).unwrap().score(&[]).unwrap().value,
        Value::Text("Y".into())
    );
    let empty = ensemble(
        "majorityVote",
        &leaf("Y", 1.0, "classification").replace("<Segment><True/>", "<Segment><False/>"),
        "classification",
    );
    assert_eq!(
        empty.bind(&[]).unwrap().score(&[]).unwrap().value,
        Value::Missing
    );
}

#[test]
fn random_trees_regression_averages_and_rejects_overflow() {
    // `rt_numeric`, `rt_numeric_batch`.
    let pmml = ensemble(
        "average",
        &(leaf("2", 1.0, "regression") + &leaf("8", 1.0, "regression")),
        "regression",
    );
    let p = pmml.bind(&[]).unwrap().score(&[]).unwrap();
    assert_eq!(p.value, Value::Number(5.0));
    assert_eq!(p.confidence, None);
    let empty = ensemble(
        "average",
        &leaf("2", 1.0, "regression").replace("<Segment><True/>", "<Segment><False/>"),
        "regression",
    );
    assert_eq!(
        empty.bind(&[]).unwrap().score(&[]).unwrap().value,
        Value::Missing
    );
    let huge = ensemble(
        "average",
        &leaf("1e308", 1.0, "regression").repeat(2),
        "regression",
    );
    assert!(huge.bind(&[]).unwrap().score(&[]).is_err());
}

#[test]
fn tree_as_missing_values_retain_parent_prediction_and_id() {
    // `treeas_default`, `treeas_depth`: CHAID confidence uses Laplace smoothing.
    let pmml = document(
        r#"<TreeModel functionName="classification" algorithmName="CHAID" missingValueStrategy="lastPrediction">
      <MiningSchema><MiningField name="x"/><MiningField name="y" usageType="target"/></MiningSchema>
      <Node id="0" score="N"><True/><ScoreDistribution value="Y" recordCount="2"/><ScoreDistribution value="N" recordCount="8"/>
        <Node id="1" score="Y"><SimplePredicate field="x" operator="lessThan" value="0"/>
          <ScoreDistribution value="Y" recordCount="2"/><ScoreDistribution value="N" recordCount="0"/>
        </Node></Node></TreeModel>"#,
        "",
    );
    let p = pmml.bind(&["x"]).unwrap().score(&[Value::Missing]).unwrap();
    assert_eq!(p.value, Value::Text("N".into()));
    assert_eq!(p.confidence, Some(9.0 / 12.0));
    assert_eq!(p.entity_id.as_deref(), Some("0"));
}

#[test]
fn tree_as_discretize_boundaries_missing_and_default() {
    // `treeas_numeric`: (-infinity,13), [13,22), [22,+infinity).
    let transforms = r#"<TransformationDictionary><DerivedField name="bin" dataType="integer" optype="ordinal">
      <Discretize field="x" defaultValue="9" mapMissingTo="8">
        <DiscretizeBin binValue="1"><Interval closure="closedOpen" rightMargin="13"/></DiscretizeBin>
        <DiscretizeBin binValue="2"><Interval closure="closedOpen" leftMargin="13" rightMargin="22"/></DiscretizeBin>
        <DiscretizeBin binValue="3"><Interval closure="closedClosed" leftMargin="22"/></DiscretizeBin>
      </Discretize></DerivedField></TransformationDictionary>"#;
    let pmml = document(
        r#"<RegressionModel functionName="regression"><MiningSchema><MiningField name="x"/></MiningSchema>
      <RegressionTable intercept="0"><NumericPredictor name="bin" coefficient="1"/></RegressionTable></RegressionModel>"#,
        transforms,
    );
    let scorer = pmml.bind(&["x"]).unwrap();
    for (x, bin) in [
        (Value::Number(-1e6), 1.0),
        (Value::Number(12.999), 1.0),
        (Value::Number(13.0), 2.0),
        (Value::Number(22.0), 3.0),
        (Value::Number(1e6), 3.0),
        (Value::Missing, 8.0),
    ] {
        assert_eq!(scorer.score(&[x]).unwrap().value, Value::Number(bin));
    }
    let open = transforms
        .replace("closedOpen", "openOpen")
        .replace("closedClosed", "openClosed");
    let pmml = document(
        r#"<RegressionModel functionName="regression"><MiningSchema/><RegressionTable intercept="0"><NumericPredictor name="bin" coefficient="1"/></RegressionTable></RegressionModel>"#,
        &open,
    );
    assert_eq!(
        pmml.bind(&["x"])
            .unwrap()
            .score(&[Value::Number(13.0)])
            .unwrap()
            .value,
        Value::Number(9.0)
    );
    for bad in [
        transforms.replace("closedOpen", "bad"),
        transforms.replace("leftMargin=\"13\"", "leftMargin=\"23\""),
        transforms.replace("rightMargin=\"13\"", "rightMargin=\"NaN\""),
    ] {
        assert!(Pmml::parse(&format!(r#"<PMML version="4.3"><DataDictionary/>{bad}<TreeModel functionName="regression"><MiningSchema/><Node score="0"><True/></Node></TreeModel></PMML>"#)).is_err());
    }
}

#[test]
fn linear_svm_uses_logistic_decision_value_with_zero_reference() {
    // `lsvm_default`, `lsvm_l1`: eta for N; reference Y has eta=0.
    let pmml = document(
        r#"<GeneralRegressionModel modelType="multinomialLogistic" algorithmName="linearSVM" functionName="classification" targetReferenceCategory="Y">
      <MiningSchema><MiningField name="x" missingValueReplacement="0"/><MiningField name="y" usageType="target"/></MiningSchema>
      <ParameterList><Parameter name="p"/></ParameterList><CovariateList><Predictor name="x"/></CovariateList>
      <PPMatrix><PPCell parameterName="p" predictorName="x" value="1"/></PPMatrix>
      <ParamMatrix><PCell parameterName="p" targetCategory="N" beta="1"/></ParamMatrix></GeneralRegressionModel>"#,
        "",
    );
    let scorer = pmml.bind(&["x"]).unwrap();
    for (x, winner) in [(-1.0, "Y"), (1.0, "N")] {
        let p = scorer.score(&[Value::Number(x)]).unwrap();
        assert_eq!(p.value, Value::Text(winner.into()));
        assert!((p.confidence.unwrap() - 0.7310585786300049).abs() < 1e-15);
    }
    assert_eq!(
        scorer.score(&[Value::Missing]).unwrap().confidence,
        Some(0.5)
    );
}

#[test]
fn interval_and_ensemble_mutations_never_panic() {
    let xml = r#"<PMML version="4.3"><DataDictionary><DataField name="x" dataType="double" optype="continuous"/></DataDictionary>
      <TransformationDictionary><DerivedField name="bin" dataType="integer" optype="ordinal"><Discretize field="x">
        <DiscretizeBin binValue="1"><Interval closure="closedOpen" rightMargin="13"/></DiscretizeBin>
        <DiscretizeBin binValue="2"><Interval closure="closedClosed" leftMargin="13"/></DiscretizeBin>
      </Discretize></DerivedField></TransformationDictionary>
      <MiningModel functionName="regression"><MiningSchema/><Segmentation multipleModelMethod="average">
        <Segment><True/><TreeModel functionName="regression"><MiningSchema/>
          <Node score="0"><True/><Node score="2"><SimplePredicate field="bin" operator="equal" value="1"/></Node></Node>
        </TreeModel></Segment></Segmentation></MiningModel></PMML>"#;
    let exercise = |candidate: &str| {
        if let Ok(pmml) = Pmml::parse(candidate)
            && let Ok(scorer) = pmml.bind(&["x"])
        {
            for value in [Value::Missing, Value::Number(13.0), Value::Number(f64::MAX)] {
                let _ = scorer.score(&[value]);
            }
        }
    };
    exercise(xml);
    for (at, ch) in xml.char_indices() {
        exercise(&xml[..at]);
        for replacement in ["", "<", "é", "\"999999999999999999999999999999999999\""] {
            exercise(&format!(
                "{}{replacement}{}",
                &xml[..at],
                &xml[at + ch.len_utf8()..]
            ));
        }
    }
}
