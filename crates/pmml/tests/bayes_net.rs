//! Modeler's Bayes Net nugget: parsing the `BayesNetModel` extension and scoring by exact inference.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use dms_pmml::{Model, Pmml, PmmlError, Value};

/// `a` and `b` depend on the target `t`, and `b` also on `a` (a tree augmented naive Bayes
/// network). `rows_b` is the table of `b`, so a test can drop rows.
fn network(rows_b: &str) -> String {
    format!(
        r#"<PMML version="4.1" xmlns="http://www.dmg.org/PMML-4_1">
  <Header copyright="test"/>
  <DataDictionary numberOfFields="3">
    <DataField name="a" optype="categorical" dataType="string"><Value value="x"/><Value value="y"/></DataField>
    <DataField name="b" optype="categorical" dataType="double"><Value value="0"/><Value value="1"/></DataField>
    <DataField name="t" optype="categorical" dataType="string"><Value value="yes"/><Value value="no"/></DataField>
  </DataDictionary>
  <Extension extender="spss.com" name="SPSS Models">
    <BayesNetModel functionName="classification" modelName="TAN">
      <ConditionalTables>
        <ConditionalTable fieldName="a" parentsNumber="1">
          <names>t; x; y</names>
          <row totalN="40">yes; 0.8; 0.2</row>
          <row totalN="60">no; 0.3; 0.7</row>
        </ConditionalTable>
        <ConditionalTable fieldName="b" parentsNumber="2">
          <names>a; t; 0; 1</names>
          {rows_b}
        </ConditionalTable>
        <ConditionalTable fieldName="t" parentsNumber="0">
          <names>yes; no</names>
          <row totalN="100">0.4; 0.6</row>
        </ConditionalTable>
      </ConditionalTables>
      <MiningSchema>
        <MiningField name="a" usageType="active"/>
        <MiningField name="b" usageType="active"/>
        <MiningField name="t" usageType="predicted"/>
      </MiningSchema>
    </BayesNetModel>
  </Extension>
</PMML>"#
    )
}

const FULL_B: &str = r#"
          <row>x; yes; 0.9; 0.1</row>
          <row>x; no; 0.5; 0.5</row>
          <row>y; yes; 0.2; 0.8</row>
          <row>y; no; 0.6; 0.4</row>"#;

fn score(xml: &str, a: Value, b: Value) -> dms_pmml::Prediction {
    let pmml = Pmml::parse(xml).unwrap();
    pmml.bind(&["a", "b"]).unwrap().score(&[a, b]).unwrap()
}

fn probability(p: &dms_pmml::Prediction, category: &str) -> f64 {
    p.probabilities
        .iter()
        .find(|(c, _)| c == category)
        .map(|(_, v)| *v)
        .unwrap()
}

fn text(s: &str) -> Value {
    Value::Text(s.into())
}

#[test]
fn bayes_net_parses_the_modeler_extension() {
    let pmml = Pmml::parse(&network(FULL_B)).unwrap();
    let Model::BayesNet(model) = &pmml.model else {
        panic!("not a Bayes Net model: {:?}", pmml.model)
    };
    assert_eq!(model.target, "t");
    assert_eq!(model.nodes.len(), 3);
    let b = model.nodes.iter().find(|n| n.name == "b").unwrap();
    assert_eq!(b.parents, ["a", "t"]);
    assert_eq!(b.categories, ["0", "1"]);
    assert_eq!(b.rows.len(), 4);
    assert_eq!(model.target_categories(), ["yes", "no"]);
}

#[test]
fn bayes_net_scores_complete_evidence() {
    // P(t) P(a=x|t) P(b=1|a=x,t): yes 0.4*0.8*0.1 = 0.032, no 0.6*0.3*0.5 = 0.09.
    let p = score(&network(FULL_B), text("x"), Value::Number(1.0));
    let total = 0.032 + 0.09;
    assert!((probability(&p, "yes") - 0.032 / total).abs() < 1e-12);
    assert!((probability(&p, "no") - 0.09 / total).abs() < 1e-12);
    assert_eq!(p.value, Value::Text("no".into()));
    assert!((p.confidence.unwrap() - 0.09 / total).abs() < 1e-12);
}

#[test]
fn bayes_net_sums_over_missing_evidence() {
    // a missing, b = 1: yes 0.4 (0.8*0.1 + 0.2*0.8) = 0.096, no 0.6 (0.3*0.5 + 0.7*0.4) = 0.258.
    let p = score(&network(FULL_B), Value::Missing, Value::Number(1.0));
    let total = 0.096 + 0.258;
    assert!((probability(&p, "yes") - 0.096 / total).abs() < 1e-12);
    // nothing known: the prior of the target
    let p = score(&network(FULL_B), Value::Missing, Value::Missing);
    assert!((probability(&p, "yes") - 0.4).abs() < 1e-12);
    // a value the tables do not list gives no evidence when the field treats it as missing
    let xml = network(FULL_B).replace(
        r#"<MiningField name="a" usageType="active"/>"#,
        r#"<MiningField name="a" usageType="active" invalidValueTreatment="asMissing"/>"#,
    );
    let p = score(&xml, text("zzz"), Value::Number(1.0));
    assert!((probability(&p, "yes") - 0.096 / total).abs() < 1e-12);
}

#[test]
fn bayes_net_gives_a_configuration_without_a_row_probability_zero() {
    // Without the row (y, no), the target `no` has probability 0 for a = y, b = 0.
    let rows =
        r#"<row>x; yes; 0.9; 0.1</row><row>x; no; 0.5; 0.5</row><row>y; yes; 0.2; 0.8</row>"#;
    let p = score(&network(rows), text("y"), Value::Number(0.0));
    assert!((probability(&p, "yes") - 1.0).abs() < 1e-12);
    assert_eq!(probability(&p, "no"), 0.0);
    // Without any row for a = y, both categories have probability 0: no prediction.
    let rows = r#"<row>x; yes; 0.9; 0.1</row><row>x; no; 0.5; 0.5</row>"#;
    let p = score(&network(rows), text("y"), Value::Number(0.0));
    assert_eq!(p.value, Value::Missing);
    assert!(p.probabilities.is_empty());
}

#[test]
fn bayes_net_rejects_malformed_models() {
    let cases = [
        // a parent that has no table
        network(FULL_B).replace("<names>a; t; 0; 1</names>", "<names>q; t; 0; 1</names>"),
        // a cycle: t depends on b
        network(FULL_B).replace(
            r#"<ConditionalTable fieldName="t" parentsNumber="0">
          <names>yes; no</names>
          <row totalN="100">0.4; 0.6</row>"#,
            r#"<ConditionalTable fieldName="t" parentsNumber="1">
          <names>b; yes; no</names>
          <row>0; 0.4; 0.6</row>"#,
        ),
        // a probability that is not a number
        network(FULL_B).replace("0.9; 0.1", "0.9; nan"),
        // a negative probability
        network(FULL_B).replace("0.9; 0.1", "0.9; -0.1"),
        // a row of the wrong width
        network(FULL_B).replace("<row>x; yes; 0.9; 0.1</row>", "<row>x; yes; 0.9</row>"),
        // more parents than names
        network(FULL_B).replace(
            r#"fieldName="b" parentsNumber="2""#,
            r#"fieldName="b" parentsNumber="9""#,
        ),
        // no predicted field
        network(FULL_B).replace(r#" usageType="predicted""#, ""),
    ];
    for xml in cases {
        assert!(
            matches!(Pmml::parse(&xml), Err(PmmlError::Parse(_))),
            "accepted: {xml}"
        );
    }
}
