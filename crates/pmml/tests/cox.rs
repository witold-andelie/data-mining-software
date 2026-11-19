//! Modeler's Cox regression nugget: parsing and the survival arithmetic.

#![allow(clippy::unwrap_used, clippy::panic, reason = "test assertions")]

use dms_pmml::{Model, Pmml, Value};

fn model_xml(tables: &str, ppmatrix: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" ?>
<PMML version="3.1" xmlns="http://www.dmg.org/PMML-3_1">
 <Header copyright="test"/>
 <DataDictionary numberOfFields="4">
  <DataField name="churn" optype="continuous" dataType="double"/>
  <DataField name="tenure" optype="continuous" dataType="double"/>
  <DataField name="age" optype="continuous" dataType="double"/>
  <DataField name="sex" optype="categorical" dataType="double">
   <Value value="0"/><Value value="1"/>
  </DataField>
 </DataDictionary>
 <GeneralRegressionModel modelType="generalLinear" modelName="COXREG" functionName="regression">
  <Extension extender="spss.com" name="endTimeVariable" value="tenure"/>
  <Extension extender="spss.com" name="referencePoint">
   <X-BasePoint parameterName="P1" value="0"/>
   <X-BasePoint parameterName="P2" value="0"/>
  </Extension>
  <Extension extender="spss.com" name="X-BaseCumHazardTables">{tables}
   <X-BaseMaxTime value="10"/>
  </Extension>
  <MiningSchema>
   <MiningField name="churn" usageType="predicted">
    <Extension extender="spss.com" name="status"><Value value="1" property="valid"/></Extension>
   </MiningField>
   <MiningField name="tenure" usageType="active"/>
   <MiningField name="age" usageType="active"/>
   <MiningField name="sex" usageType="active"/>
  </MiningSchema>
  <ParameterList><Parameter name="P1" label="age"/><Parameter name="P2" label="[sex=1]"/></ParameterList>
  <FactorList><Predictor name="sex"/></FactorList>
  <CovariateList><Predictor name="age"/></CovariateList>
  <PPMatrix>{ppmatrix}</PPMatrix>
  <ParamMatrix>
   <PCell parameterName="P1" beta="0.5" df="1"/>
   <PCell parameterName="P2" beta="-1" df="1"/>
  </ParamMatrix>
 </GeneralRegressionModel>
</PMML>"#
    )
}

const TABLE: &str = r#"<X-BaseCumHazardTable>
    <X-Baseline time="1" cumHazard="0.1"/>
    <X-Baseline time="3" cumHazard="0.4"/>
    <X-Baseline time="6" cumHazard="1"/>
   </X-BaseCumHazardTable>"#;
const PP: &str = r#"<PPCell value="1" predictorName="age" parameterName="P1"/>
   <PPCell value="1" predictorName="sex" parameterName="P2"/>"#;

fn cox(xml: &str) -> dms_pmml::CoxModel {
    match Pmml::parse(xml).unwrap().model {
        Model::Cox(model) => model,
        other => panic!("not a Cox model: {other:?}"),
    }
}

#[test]
fn cox_model_is_read() {
    let model = cox(&model_xml(TABLE, PP));
    assert_eq!(model.time_field, "tenure");
    assert_eq!(model.event_field, "churn");
    assert_eq!(model.event_value, "1");
    assert_eq!(model.terms.len(), 2);
    assert_eq!(model.terms[0].field, "age");
    assert_eq!(model.terms[0].level, None);
    assert_eq!(model.terms[1].level.as_deref(), Some("1"));
    assert_eq!(model.baseline.len(), 3);
}

#[test]
fn cox_linear_predictor_and_baseline_step_function() {
    let model = cox(&model_xml(TABLE, PP));
    let age = Value::Number(2.0);
    let sex = Value::Number(1.0);
    let lp = model
        .linear_predictor(|name| match name {
            "age" => Some(&age),
            "sex" => Some(&sex),
            _ => None,
        })
        .unwrap();
    assert!((lp - 0.0).abs() < 1e-12); // 0.5 * 2 - 1 * 1
    assert_eq!(model.baseline_hazard(0.5), 0.0);
    assert_eq!(model.baseline_hazard(1.0), 0.1);
    assert_eq!(model.baseline_hazard(2.9), 0.1);
    assert_eq!(model.baseline_hazard(100.0), 1.0);
    let missing = Value::Missing;
    assert!(
        model
            .linear_predictor(|name| match name {
                "age" => Some(&missing),
                _ => Some(&sex),
            })
            .is_none()
    );
}

#[test]
fn cox_conditional_survival_follows_the_baseline() {
    let model = cox(&model_xml(TABLE, PP));
    // lp = ln 2: H(t) = 2 H0(t). From t0 = 1 over 2 time units: 2 * (0.4 - 0.1) = 0.6.
    let score = model.conditional(2.0_f64.ln(), 1.0, 2.0).unwrap();
    assert!((score.cumulative_hazard - 0.6).abs() < 1e-12);
    assert!((score.survival - (-0.6_f64).exp()).abs() < 1e-12);
    // nothing past the longest event time (6)
    assert!(model.conditional(0.0, 5.0, 2.0).is_none());
    assert!(model.conditional(0.0, 5.0, 1.0).is_some());
    // no past time: the plain survival function
    let plain = model.conditional(0.0, 0.0, 3.0).unwrap();
    assert!((plain.survival - (-0.4_f64).exp()).abs() < 1e-12);
}

#[test]
fn cox_rejects_models_it_cannot_score() {
    let two_tables = format!("{TABLE}{TABLE}");
    assert!(Pmml::parse(&model_xml(&two_tables, PP)).is_err());
    let interaction = r#"<PPCell value="1" predictorName="age" parameterName="P1"/>
   <PPCell value="1" predictorName="age" parameterName="P2"/>
   <PPCell value="1" predictorName="sex" parameterName="P2"/>"#;
    assert!(Pmml::parse(&model_xml(TABLE, interaction)).is_err());
    let descending = TABLE.replace(r#"time="3""#, r#"time="0""#);
    assert!(Pmml::parse(&model_xml(&descending, PP)).is_err());
}
