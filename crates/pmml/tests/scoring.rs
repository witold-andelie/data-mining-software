//! Unit tests for `dms-pmml`: parsing, transformations, `MiningSchema` treatments, `TreeModel`,
//! `ClusteringModel`, `GeneralRegressionModel`, `RegressionModel`, and malformed input rejection.

use dms_pmml::{Model, Pmml, PmmlError, Value};

fn assert_close(actual: f64, expected: f64) {
    let diff = (actual - expected).abs();
    let tol = 1e-9 * actual.abs().max(expected.abs()).max(1.0);
    assert!(
        diff <= tol,
        "expected {expected:.15}, got {actual:.15} (diff {diff:e})"
    );
}

#[test]
fn tree_predicates_and_surrogates() {
    let xml = r#"
<PMML version="4.3" xmlns="http://www.dmg.org/PMML-4_3">
  <DataDictionary>
    <DataField name="x" optype="continuous" dataType="double"/>
    <DataField name="cat" optype="categorical" dataType="string">
      <Value value="A"/>
      <Value value="B"/>
      <Value value="C"/>
    </DataField>
    <DataField name="flag" optype="categorical" dataType="string"/>
    <DataField name="y" optype="categorical" dataType="string">
      <Value value="yes"/>
      <Value value="no"/>
    </DataField>
  </DataDictionary>
  <TreeModel functionName="classification" missingValueStrategy="none" noTrueChildStrategy="returnLastPrediction">
    <MiningSchema>
      <MiningField name="x" usageType="active"/>
      <MiningField name="cat" usageType="active"/>
      <MiningField name="flag" usageType="active"/>
      <MiningField name="y" usageType="target"/>
    </MiningSchema>
    <Node id="root" score="no" recordCount="100">
      <True/>
      <ScoreDistribution value="yes" recordCount="40" probability="0.4" confidence="0.4"/>
      <ScoreDistribution value="no" recordCount="60" probability="0.6" confidence="0.6"/>
      <Node id="n1" score="yes" recordCount="30">
        <CompoundPredicate booleanOperator="surrogate">
          <SimplePredicate field="x" operator="lessOrEqual" value="10"/>
          <SimpleSetPredicate field="cat" booleanOperator="isIn">
            <Array n="2" type="string">"A" "B"</Array>
          </SimpleSetPredicate>
        </CompoundPredicate>
        <ScoreDistribution value="yes" recordCount="27" probability="0.9" confidence="0.88"/>
        <ScoreDistribution value="no" recordCount="3" probability="0.1" confidence="0.12"/>
      </Node>
      <Node id="n2" score="yes" recordCount="20">
        <CompoundPredicate booleanOperator="xor">
          <SimplePredicate field="flag" operator="isMissing"/>
          <SimplePredicate field="x" operator="greaterThan" value="50"/>
        </CompoundPredicate>
        <ScoreDistribution value="yes" recordCount="15" probability="0.75"/>
        <ScoreDistribution value="no" recordCount="5" probability="0.25"/>
      </Node>
      <Node id="n3" score="no" recordCount="50">
        <CompoundPredicate booleanOperator="and">
          <SimplePredicate field="flag" operator="isNotMissing"/>
          <SimpleSetPredicate field="cat" booleanOperator="isNotIn">
            <Array n="1" type="string">A</Array>
          </SimpleSetPredicate>
        </CompoundPredicate>
        <ScoreDistribution value="yes" recordCount="5" probability="0.1"/>
        <ScoreDistribution value="no" recordCount="45" probability="0.9"/>
      </Node>
    </Node>
  </TreeModel>
</PMML>
"#;
    let pmml = Pmml::parse(xml).unwrap();
    let scorer = pmml.bind(&["x", "cat", "flag"]).unwrap();

    // Primary split x <= 10 is true -> node n1.
    let p1 = scorer
        .score(&[
            Value::Number(5.0),
            Value::Text("C".into()),
            Value::Text("ok".into()),
        ])
        .unwrap();
    assert_eq!(p1.value, Value::Text("yes".into()));
    assert_eq!(p1.entity_id.as_deref(), Some("n1"));
    assert_close(p1.confidence.unwrap(), 0.88);
    assert_eq!(p1.probabilities.len(), 2);
    assert_close(p1.probabilities[0].1, 0.9);
    assert_close(p1.probabilities[1].1, 0.1);

    // Primary split x is Missing -> surrogate evaluates cat isIn {"A", "B"} -> true -> n1.
    let p2 = scorer
        .score(&[
            Value::Missing,
            Value::Text("B".into()),
            Value::Text("ok".into()),
        ])
        .unwrap();
    assert_eq!(p2.entity_id.as_deref(), Some("n1"));

    // x = 60 (> 10), flag not missing, xor(false, true) = true -> n2.
    let p3 = scorer
        .score(&[
            Value::Number(60.0),
            Value::Text("A".into()),
            Value::Text("ok".into()),
        ])
        .unwrap();
    assert_eq!(p3.entity_id.as_deref(), Some("n2"));
    assert_close(p3.confidence.unwrap(), 0.75);

    // x = 20, flag not missing, cat = "C" (not in {"A"}) -> n3.
    let p4 = scorer
        .score(&[
            Value::Number(20.0),
            Value::Text("C".into()),
            Value::Text("ok".into()),
        ])
        .unwrap();
    assert_eq!(p4.entity_id.as_deref(), Some("n3"));
    assert_eq!(p4.value, Value::Text("no".into()));

    // No child matches (x = 20, cat = "A", flag = "ok") -> returnLastPrediction returns root.
    let p5 = scorer
        .score(&[
            Value::Number(20.0),
            Value::Text("A".into()),
            Value::Text("ok".into()),
        ])
        .unwrap();
    assert_eq!(p5.entity_id.as_deref(), Some("root"));
    assert_eq!(p5.value, Value::Text("no".into()));
}

#[test]
fn tree_missing_value_strategies() {
    let make_tree = |strategy: &str, no_true: &str| -> String {
        format!(
            r#"
<PMML version="4.3" xmlns="http://www.dmg.org/PMML-4_3">
  <DataDictionary>
    <DataField name="x" optype="continuous" dataType="double"/>
    <DataField name="y" optype="categorical" dataType="string">
      <Value value="0"/>
      <Value value="1"/>
    </DataField>
  </DataDictionary>
  <TreeModel functionName="classification" missingValueStrategy="{strategy}" noTrueChildStrategy="{no_true}">
    <MiningSchema>
      <MiningField name="x"/>
      <MiningField name="y" usageType="target"/>
    </MiningSchema>
    <Node id="root" score="0" recordCount="100" defaultChild="c2">
      <True/>
      <ScoreDistribution value="0" recordCount="55" probability="0.55" confidence="0.55"/>
      <ScoreDistribution value="1" recordCount="45" probability="0.45" confidence="0.45"/>
      <Node id="c1" score="0" recordCount="40">
        <SimplePredicate field="x" operator="lessThan" value="0"/>
        <ScoreDistribution value="0" recordCount="30" probability="0.75" confidence="0.75"/>
        <ScoreDistribution value="1" recordCount="10" probability="0.25" confidence="0.25"/>
      </Node>
      <Node id="c2" score="1" recordCount="60">
        <SimplePredicate field="x" operator="greaterOrEqual" value="0"/>
        <ScoreDistribution value="0" recordCount="12" probability="0.20" confidence="0.20"/>
        <ScoreDistribution value="1" recordCount="48" probability="0.80" confidence="0.80"/>
      </Node>
    </Node>
  </TreeModel>
</PMML>
"#
        )
    };

    // lastPrediction
    let pmml = Pmml::parse(&make_tree("lastPrediction", "returnNullPrediction")).unwrap();
    let pred = pmml.bind(&["x"]).unwrap().score(&[Value::Missing]).unwrap();
    assert_eq!(pred.entity_id.as_deref(), Some("root"));
    assert_eq!(pred.value, Value::Text("0".into()));
    assert_close(pred.confidence.unwrap(), 0.55);

    // nullPrediction
    let pmml = Pmml::parse(&make_tree("nullPrediction", "returnLastPrediction")).unwrap();
    let pred = pmml.bind(&["x"]).unwrap().score(&[Value::Missing]).unwrap();
    assert_eq!(pred.value, Value::Missing);
    assert!(pred.confidence.is_none());

    // defaultChild -> c2
    let pmml = Pmml::parse(&make_tree("defaultChild", "returnNullPrediction")).unwrap();
    let pred = pmml.bind(&["x"]).unwrap().score(&[Value::Missing]).unwrap();
    assert_eq!(pred.entity_id.as_deref(), Some("c2"));
    assert_eq!(pred.value, Value::Text("1".into()));
    assert_close(pred.confidence.unwrap(), 0.80);

    // weightedConfidence: 0.4 * (0.75, 0.25) + 0.6 * (0.20, 0.80) = (0.42, 0.58)
    let pmml = Pmml::parse(&make_tree("weightedConfidence", "returnNullPrediction")).unwrap();
    let pred = pmml.bind(&["x"]).unwrap().score(&[Value::Missing]).unwrap();
    assert_eq!(pred.value, Value::Text("1".into()));
    assert_close(pred.confidence.unwrap(), 0.58);
    assert_close(pred.probabilities[0].1, 0.42);
    assert_close(pred.probabilities[1].1, 0.58);

    // aggregateNodes: counts (30+12, 10+48) = (42, 58) / 100 = (0.42, 0.58)
    let pmml = Pmml::parse(&make_tree("aggregateNodes", "returnNullPrediction")).unwrap();
    let pred = pmml.bind(&["x"]).unwrap().score(&[Value::Missing]).unwrap();
    assert_eq!(pred.value, Value::Text("1".into()));
    assert_close(pred.confidence.unwrap(), 0.58);
}

#[test]
fn clustering_metrics_and_missing_weights() {
    let make_clustering = |kind: &str, metric_xml: &str, compare: &str, extra_attr: &str| {
        format!(
            r#"
<PMML version="4.3" xmlns="http://www.dmg.org/PMML-4_3">
  <DataDictionary>
    <DataField name="f1" optype="continuous" dataType="double"/>
    <DataField name="f2" optype="continuous" dataType="double"/>
  </DataDictionary>
  <ClusteringModel modelClass="centerBased" functionName="clustering" numberOfClusters="2">
    <MiningSchema>
      <MiningField name="f1"/>
      <MiningField name="f2"/>
    </MiningSchema>
    <ComparisonMeasure kind="{kind}" compareFunction="{compare}" {extra_attr}>
      {metric_xml}
    </ComparisonMeasure>
    <ClusteringField field="f1" fieldWeight="2.0" isCenterField="true"/>
    <ClusteringField field="f2" fieldWeight="1.0" isCenterField="true"/>
    <MissingValueWeights>
      <Array n="2" type="real">0.25 0.75</Array>
    </MissingValueWeights>
    <Cluster id="c1" name="first">
      <Array n="2" type="real">1.0 2.0</Array>
    </Cluster>
    <Cluster id="c2" name="second">
      <Array n="2" type="real">5.0 6.0</Array>
    </Cluster>
  </ClusteringModel>
</PMML>
"#
        )
    };

    // Euclidean distance: for (2.0, 4.0) vs (1.0, 2.0), w1*d1^2 + w2*d2^2 = 2*1 + 1*4 = 6 -> sqrt(6)
    let pmml = Pmml::parse(&make_clustering("distance", "<euclidean/>", "absDiff", "")).unwrap();
    let scorer = pmml.bind(&["f1", "f2"]).unwrap();
    let pred = scorer
        .score(&[Value::Number(2.0), Value::Number(4.0)])
        .unwrap();
    assert_eq!(pred.value, Value::Text("first".into()));
    assert_eq!(pred.entity_id.as_deref(), Some("c1"));
    assert_close(pred.affinity.unwrap(), 6.0_f64.sqrt());

    // Euclidean with missing f2: adjust_M = (0.25 + 0.75) / 0.25 = 4.0.
    // For c1: sqrt(4.0 * 2.0 * (2 - 1)^2) = sqrt(8.0).
    let pred_missing = scorer.score(&[Value::Number(2.0), Value::Missing]).unwrap();
    assert_eq!(pred_missing.entity_id.as_deref(), Some("c1"));
    assert_close(pred_missing.affinity.unwrap(), 8.0_f64.sqrt());

    // SquaredEuclidean
    let pmml = Pmml::parse(&make_clustering(
        "distance",
        "<squaredEuclidean/>",
        "absDiff",
        "",
    ))
    .unwrap();
    let pred = pmml
        .bind(&["f1", "f2"])
        .unwrap()
        .score(&[Value::Number(2.0), Value::Number(4.0)])
        .unwrap();
    assert_close(pred.affinity.unwrap(), 6.0);

    // CityBlock: 2*|2-1| + 1*|4-2| = 4.0
    let pmml = Pmml::parse(&make_clustering("distance", "<cityBlock/>", "absDiff", "")).unwrap();
    let pred = pmml
        .bind(&["f1", "f2"])
        .unwrap()
        .score(&[Value::Number(2.0), Value::Number(4.0)])
        .unwrap();
    assert_close(pred.affinity.unwrap(), 4.0);

    // Chebychev: max(2*|2-1|, 1*|4-2|) = 2.0
    let pmml = Pmml::parse(&make_clustering("distance", "<chebychev/>", "absDiff", "")).unwrap();
    let pred = pmml
        .bind(&["f1", "f2"])
        .unwrap()
        .score(&[Value::Number(2.0), Value::Number(4.0)])
        .unwrap();
    assert_close(pred.affinity.unwrap(), 2.0);

    // Minkowski(p=3): (2*1^3 + 1*2^3)^(1/3) = 10^(1/3)
    let pmml = Pmml::parse(&make_clustering(
        "distance",
        r#"<minkowski p="3"/>"#,
        "absDiff",
        "",
    ))
    .unwrap();
    let pred = pmml
        .bind(&["f1", "f2"])
        .unwrap()
        .score(&[Value::Number(2.0), Value::Number(4.0)])
        .unwrap();
    assert_close(pred.affinity.unwrap(), 10.0_f64.powf(1.0 / 3.0));

    // Similarity with gaussSim(scale=2.0): at (5.0, 6.0), c2 has diff=0 -> gaussSim=1.0 -> cityBlock=3.0
    let pmml = Pmml::parse(&make_clustering(
        "similarity",
        "<cityBlock/>",
        "gaussSim",
        r#"similarityScale="2.0""#,
    ))
    .unwrap();
    let pred = pmml
        .bind(&["f1", "f2"])
        .unwrap()
        .score(&[Value::Number(5.0), Value::Number(6.0)])
        .unwrap();
    assert_eq!(pred.entity_id.as_deref(), Some("c2"));
    assert_close(pred.affinity.unwrap(), 3.0);
}

#[test]
fn general_regression_and_regression_models() {
    // 1. GeneralRegressionModel (generalLinear with factor + covariate)
    let glm_xml = r#"
<PMML version="4.3" xmlns="http://www.dmg.org/PMML-4_3">
  <DataDictionary>
    <DataField name="y" optype="continuous" dataType="double"/>
    <DataField name="group" optype="categorical" dataType="string">
      <Value value="G1"/>
      <Value value="G2"/>
    </DataField>
    <DataField name="x" optype="continuous" dataType="double"/>
  </DataDictionary>
  <GeneralRegressionModel modelType="generalLinear" functionName="regression" targetVariableName="y">
    <MiningSchema>
      <MiningField name="group"/>
      <MiningField name="x"/>
      <MiningField name="y" usageType="predicted"/>
    </MiningSchema>
    <ParameterList>
      <Parameter name="p0"/>
      <Parameter name="p1"/>
      <Parameter name="p2"/>
    </ParameterList>
    <FactorList>
      <Predictor name="group"/>
    </FactorList>
    <CovariateList>
      <Predictor name="x"/>
    </CovariateList>
    <PPMatrix>
      <PPCell value="G1" predictorName="group" parameterName="p1"/>
      <PPCell value="2" predictorName="x" parameterName="p2"/>
    </PPMatrix>
    <ParamMatrix>
      <PCell parameterName="p0" beta="1.5"/>
      <PCell parameterName="p1" beta="2.5"/>
      <PCell parameterName="p2" beta="0.5"/>
    </ParamMatrix>
  </GeneralRegressionModel>
</PMML>
"#;
    let pmml = Pmml::parse(glm_xml).unwrap();
    let scorer = pmml.bind(&["group", "x"]).unwrap();
    // y = 1.5 + 2.5 * 1 + 0.5 * (3^2) = 8.5
    let pred = scorer
        .score(&[Value::Text("G1".into()), Value::Number(3.0)])
        .unwrap();
    assert_eq!(pred.value, Value::Number(8.5));

    // 2. GeneralRegressionModel (multinomialLogistic with targetReferenceCategory)
    let mn_xml = r#"
<PMML version="4.3" xmlns="http://www.dmg.org/PMML-4_3">
  <DataDictionary>
    <DataField name="target" optype="categorical" dataType="string">
      <Value value="0"/>
      <Value value="1"/>
    </DataField>
    <DataField name="x" optype="continuous" dataType="double"/>
  </DataDictionary>
  <GeneralRegressionModel modelType="multinomialLogistic" functionName="classification" targetReferenceCategory="0">
    <MiningSchema>
      <MiningField name="x"/>
      <MiningField name="target" usageType="target"/>
    </MiningSchema>
    <ParameterList>
      <Parameter name="p0"/>
      <Parameter name="p1"/>
    </ParameterList>
    <CovariateList>
      <Predictor name="x"/>
    </CovariateList>
    <PPMatrix>
      <PPCell value="1" predictorName="x" parameterName="p1"/>
    </PPMatrix>
    <ParamMatrix>
      <PCell targetCategory="1" parameterName="p0" beta="-1.0"/>
      <PCell targetCategory="1" parameterName="p1" beta="2.0"/>
    </ParamMatrix>
  </GeneralRegressionModel>
</PMML>
"#;
    let pmml = Pmml::parse(mn_xml).unwrap();
    let pred = pmml
        .bind(&["x"])
        .unwrap()
        .score(&[Value::Number(1.0)])
        .unwrap();
    let p1 = 1.0 / (1.0 + (-1.0_f64).exp());
    assert_eq!(pred.value, Value::Text("1".into()));
    assert_close(pred.confidence.unwrap(), p1);
    assert_close(pred.probabilities[0].1, 1.0 - p1);
    assert_close(pred.probabilities[1].1, p1);

    // 3. RegressionModel (regression + classification with softmax and logit)
    let reg_xml = r#"
<PMML version="4.3" xmlns="http://www.dmg.org/PMML-4_3">
  <DataDictionary>
    <DataField name="y" optype="categorical" dataType="string">
      <Value value="catA"/>
      <Value value="catB"/>
    </DataField>
    <DataField name="x" optype="continuous" dataType="double"/>
    <DataField name="c" optype="categorical" dataType="string"/>
  </DataDictionary>
  <RegressionModel functionName="classification" normalizationMethod="softmax">
    <MiningSchema>
      <MiningField name="x"/>
      <MiningField name="c"/>
      <MiningField name="y" usageType="target"/>
    </MiningSchema>
    <RegressionTable intercept="0.5" targetCategory="catA">
      <NumericPredictor name="x" exponent="2" coefficient="0.25"/>
      <CategoricalPredictor name="c" value="hi" coefficient="1.0"/>
    </RegressionTable>
    <RegressionTable intercept="0.0" targetCategory="catB"/>
  </RegressionModel>
</PMML>
"#;
    let pmml = Pmml::parse(reg_xml).unwrap();
    let pred = pmml
        .bind(&["x", "c"])
        .unwrap()
        .score(&[Value::Number(2.0), Value::Text("hi".into())])
        .unwrap();
    // r_A = 0.5 + 0.25 * 4 + 1.0 = 2.5, r_B = 0.0
    let expected_pa = 2.5_f64.exp() / (2.5_f64.exp() + 1.0);
    assert_eq!(pred.value, Value::Text("catA".into()));
    assert_close(pred.confidence.unwrap(), expected_pa);

    let logit_xml = reg_xml.replace(
        r#"normalizationMethod="softmax""#,
        r#"normalizationMethod="logit""#,
    );
    let pmml = Pmml::parse(&logit_xml).unwrap();
    let pred = pmml
        .bind(&["x", "c"])
        .unwrap()
        .score(&[Value::Number(2.0), Value::Text("hi".into())])
        .unwrap();
    let expected_logit_pa = 1.0 / (1.0 + (-2.5_f64).exp());
    assert_close(pred.probabilities[0].1, expected_logit_pa);
    assert_close(pred.probabilities[1].1, 1.0 - expected_logit_pa);
}

#[test]
fn transformations_and_mining_schema_treatments() {
    let xml = r#"
<PMML version="4.3" xmlns="http://www.dmg.org/PMML-4_3">
  <DataDictionary>
    <DataField name="x" optype="continuous" dataType="double"/>
    <DataField name="cat" optype="categorical" dataType="string">
      <Value value="A"/>
      <Value value="B"/>
    </DataField>
    <DataField name="y" optype="continuous" dataType="double"/>
  </DataDictionary>
  <TransformationDictionary>
    <DefineFunction name="scaleAndShift" optype="continuous" dataType="double">
      <ParameterField name="v"/>
      <ParameterField name="scale"/>
      <Apply function="+">
        <Apply function="*">
          <FieldRef field="v"/>
          <FieldRef field="scale"/>
        </Apply>
        <Constant dataType="double">1.0</Constant>
      </Apply>
    </DefineFunction>
    <DerivedField name="x_norm" optype="continuous" dataType="double">
      <NormContinuous field="x" outliers="asExtremeValues" mapMissingTo="0.5">
        <LinearNorm orig="10" norm="0"/>
        <LinearNorm orig="20" norm="1"/>
      </NormContinuous>
    </DerivedField>
    <DerivedField name="is_a" optype="continuous" dataType="double">
      <NormDiscrete field="cat" value="A" mapMissingTo="0"/>
    </DerivedField>
  </TransformationDictionary>
  <RegressionModel functionName="regression" normalizationMethod="none">
    <MiningSchema>
      <MiningField name="x" outliers="asExtremeValues" lowValue="0" highValue="30" missingValueReplacement="15"/>
      <MiningField name="cat" invalidValueTreatment="asMissing" missingValueReplacement="A"/>
      <MiningField name="y" usageType="target"/>
    </MiningSchema>
    <LocalTransformations>
      <DerivedField name="combined" optype="continuous" dataType="double">
        <Apply function="scaleAndShift">
          <FieldRef field="x_norm"/>
          <Apply function="if">
            <Apply function="equal">
              <FieldRef field="is_a"/>
              <Constant>1</Constant>
            </Apply>
            <Constant>10</Constant>
            <Constant>2</Constant>
          </Apply>
        </Apply>
      </DerivedField>
    </LocalTransformations>
    <RegressionTable intercept="0.0">
      <NumericPredictor name="combined" coefficient="1.0"/>
    </RegressionTable>
  </RegressionModel>
</PMML>
"#;
    let pmml = Pmml::parse(xml).unwrap();
    let scorer = pmml.bind(&["x", "cat"]).unwrap();

    // Missing x -> replaced by 15 -> x_norm = 0.5.
    // Invalid cat "Z" -> asMissing -> replaced by "A" -> is_a = 1 -> scale = 10 -> combined = 0.5*10 + 1 = 6.0.
    let pred = scorer
        .score(&[Value::Missing, Value::Text("Z".into())])
        .unwrap();
    assert_eq!(pred.value, Value::Number(6.0));

    // Extreme x = 50 -> clamped to 30 by MiningField, then clamped to 1.0 by NormContinuous(asExtremeValues).
    // cat = "B" -> is_a = 0 -> scale = 2 -> combined = 1.0*2 + 1 = 3.0.
    let pred2 = scorer
        .score(&[Value::Number(50.0), Value::Text("B".into())])
        .unwrap();
    assert_eq!(pred2.value, Value::Number(3.0));
}

#[test]
fn rejects_malformed_and_deeply_nested_input() {
    // Empty input
    assert!(matches!(Pmml::parse(""), Err(PmmlError::Parse(_))));

    // Wrong root element
    assert!(matches!(
        Pmml::parse(r#"<NotPMML version="4.3"/>"#),
        Err(PmmlError::Parse(_))
    ));

    // Wrong namespace
    assert!(matches!(
        Pmml::parse(r#"<PMML version="4.3" xmlns="http://example.com/bad"/>"#),
        Err(PmmlError::Parse(_))
    ));

    // Missing model element (header only)
    let header_only = r#"
<PMML version="4.3" xmlns="http://www.dmg.org/PMML-4_3">
  <Header/>
  <DataDictionary/>
</PMML>
"#;
    assert!(matches!(Pmml::parse(header_only), Err(PmmlError::Parse(_))));

    // Unknown field on bind
    let simple = r#"
<PMML version="4.3" xmlns="http://www.dmg.org/PMML-4_3">
  <DataDictionary>
    <DataField name="x" optype="continuous" dataType="double"/>
  </DataDictionary>
  <RegressionModel functionName="regression">
    <MiningSchema>
      <MiningField name="x"/>
    </MiningSchema>
    <RegressionTable intercept="1.0">
      <NumericPredictor name="x" coefficient="2.0"/>
    </RegressionTable>
  </RegressionModel>
</PMML>
"#;
    let pmml = Pmml::parse(simple).unwrap();
    assert_eq!(
        pmml.bind(&["other"]).unwrap_err(),
        PmmlError::UnknownField("x".into())
    );

    // Unsupported model element is preserved as Model::Unsupported and returns Unsupported when scored.
    let unsupported_xml = r#"
<PMML version="4.3" xmlns="http://www.dmg.org/PMML-4_3">
  <DataDictionary/>
  <NeuralNetwork functionName="regression"/>
</PMML>
"#;
    let pmml = Pmml::parse(unsupported_xml).unwrap();
    assert_eq!(pmml.model, Model::Unsupported("NeuralNetwork".into()));
    let scorer = pmml.bind(&[]).unwrap();
    assert_eq!(
        scorer.score(&[]),
        Err(PmmlError::Unsupported("NeuralNetwork".into()))
    );

    // Excessive recursion depth in Apply is rejected with PmmlError::Parse without panicking.
    let mut deep = String::from(
        r#"<PMML version="4.3"><DataDictionary/><TransformationDictionary><DerivedField name="d" optype="continuous" dataType="double">"#,
    );
    for _ in 0..80 {
        deep.push_str(r#"<Apply function="+"><Constant>1</Constant>"#);
    }
    deep.push_str("<Constant>1</Constant>");
    for _ in 0..80 {
        deep.push_str("</Apply>");
    }
    deep.push_str("</DerivedField></TransformationDictionary></PMML>");
    assert!(matches!(Pmml::parse(&deep), Err(PmmlError::Parse(_))));
}

#[test]
fn builtin_functions_and_modeler_tree_laplace() {
    let xml = r#"
<PMML version="4.3" xmlns="http://www.dmg.org/PMML-4_3">
  <DataDictionary>
    <DataField name="a" optype="continuous" dataType="double"/>
    <DataField name="b" optype="continuous" dataType="double"/>
    <DataField name="y" optype="categorical" dataType="string">
      <Value value="0"/>
      <Value value="1"/>
    </DataField>
  </DataDictionary>
  <TransformationDictionary>
    <DerivedField name="math_expr" optype="continuous" dataType="double">
      <Apply function="+">
        <Apply function="-">
          <Apply function="max">
            <FieldRef field="a"/>
            <FieldRef field="b"/>
          </Apply>
          <Apply function="min">
            <FieldRef field="a"/>
            <FieldRef field="b"/>
          </Apply>
        </Apply>
        <Apply function="/">
          <Apply function="sqrt">
            <Apply function="abs">
              <Constant>-16</Constant>
            </Apply>
          </Apply>
          <Apply function="exp">
            <Apply function="ln">
              <Constant>2</Constant>
            </Apply>
          </Apply>
        </Apply>
      </Apply>
    </DerivedField>
    <DerivedField name="logic_expr" optype="continuous" dataType="double">
      <Apply function="if">
        <Apply function="and">
          <Apply function="not">
            <Apply function="isMissing">
              <FieldRef field="a"/>
            </Apply>
          </Apply>
          <Apply function="or">
            <Apply function="lessThan">
              <FieldRef field="a"/>
              <FieldRef field="b"/>
            </Apply>
            <Apply function="greaterOrEqual">
              <FieldRef field="a"/>
              <FieldRef field="b"/>
            </Apply>
          </Apply>
          <Apply function="isNotMissing">
            <FieldRef field="b"/>
          </Apply>
          <Apply function="notEqual">
            <FieldRef field="a"/>
            <Constant>999</Constant>
          </Apply>
          <Apply function="lessOrEqual">
            <FieldRef field="a"/>
            <Constant>100</Constant>
          </Apply>
          <Apply function="greaterThan">
            <FieldRef field="b"/>
            <Constant>0</Constant>
          </Apply>
        </Apply>
        <FieldRef field="math_expr"/>
        <Constant>0</Constant>
      </Apply>
    </DerivedField>
  </TransformationDictionary>
  <TreeModel algorithmName="CHAID" functionName="classification">
    <MiningSchema>
      <MiningField name="a"/>
      <MiningField name="b"/>
      <MiningField name="y" usageType="predicted"/>
    </MiningSchema>
    <Node id="0" score="1" recordCount="10">
      <SimplePredicate field="logic_expr" operator="equal" value="5"/>
      <ScoreDistribution value="0" recordCount="2" probability="0.2" confidence="0.2"/>
      <ScoreDistribution value="1" recordCount="8" probability="0.8" confidence="0.8"/>
    </Node>
  </TreeModel>
</PMML>
"#;
    let pmml = Pmml::parse(xml).unwrap();
    let scorer = pmml.bind(&["a", "b"]).unwrap();
    // max(2, 5) - min(2, 5) + sqrt(16)/exp(ln(2)) = 3 + 4/2 = 5 -> matches root predicate.
    // CHAID Laplace confidence: (8 + 1) / (10 + 2) = 9 / 12 = 0.75.
    let pred = scorer
        .score(&[Value::Number(2.0), Value::Number(5.0)])
        .unwrap();
    assert_eq!(pred.value, Value::Text("1".into()));
    assert_close(pred.confidence.unwrap(), 0.75);
    assert_close(pred.probabilities[0].1, 0.25);
    assert_close(pred.probabilities[1].1, 0.75);
}
