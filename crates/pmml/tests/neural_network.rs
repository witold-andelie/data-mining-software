//! Neural Net scoring: activations, hidden normalization, output softmax, rectified
//! simplemax (`nn_fixed_a`), RBF (`nn_rbf`), adjusted propensity (`nn_partition`), and
//! missing inputs (`nn_missing_listwise`, `nn_missing_impute`).
#![allow(clippy::panic)]

use dms_pmml::{Model, Pmml, PmmlError, Value};

fn score(xml: &str, fields: &[&str], row: &[Value]) -> Result<dms_pmml::Prediction, PmmlError> {
    let pmml = Pmml::parse(xml)?;
    pmml.bind(fields)?.score(row)
}

fn close(got: f64, expected: f64) {
    let scale = expected.abs().max(1.0);
    assert!(
        (got - expected).abs() <= 1e-12 * scale,
        "{got} != {expected}"
    );
}

fn number(prediction: &dms_pmml::Prediction) -> f64 {
    match prediction.value {
        Value::Number(value) => value,
        ref other => panic!("expected a number, got {other:?}"),
    }
}

fn dictionary_head() -> &'static str {
    r#"<PMML version="4.3" xmlns="http://www.dmg.org/PMML-4_3"><DataDictionary>"#
}

#[test]
fn activations_match_the_pmml_definitions() {
    let cases = [
        ("threshold", 0.5, 1.0),
        ("threshold", 0.0, 0.0),
        ("logistic", 0.5, 1.0 / (1.0 + (-0.5_f64).exp())),
        ("tanh", 0.5, 0.5_f64.tanh()),
        ("identity", 0.5, 0.5),
        ("exponential", 0.5, 0.5_f64.exp()),
        ("reciprocal", 0.5, 2.0),
        ("square", 0.5, 0.25),
        ("Gauss", 0.5, (-0.25_f64).exp()),
        ("sine", 0.5, 0.5_f64.sin()),
        ("cosine", 0.5, 0.5_f64.cos()),
        ("Elliott", 0.5, 0.5 / 1.5),
        ("arctan", 0.5, 0.5_f64.atan()),
    ];
    for (activation, input, expected) in cases {
        let xml = format!(
            r#"{head}
    <DataField name="x" optype="continuous" dataType="double"/>
    <DataField name="r" optype="continuous" dataType="double"/>
  </DataDictionary>
  <TransformationDictionary>
    <DerivedField name="rNorm" optype="continuous" dataType="double">
      <NormContinuous field="r">
        <LinearNorm orig="0" norm="0"/><LinearNorm orig="1" norm="1"/>
      </NormContinuous>
    </DerivedField>
  </TransformationDictionary>
  <NeuralNetwork functionName="regression" activationFunction="{activation}">
    <MiningSchema>
      <MiningField name="x"/>
      <MiningField name="r" usageType="predicted"/>
    </MiningSchema>
    <NeuralInputs>
      <NeuralInput id="0"><DerivedField optype="continuous" dataType="double">
        <FieldRef field="x"/>
      </DerivedField></NeuralInput>
    </NeuralInputs>
    <NeuralLayer>
      <Neuron id="1" bias="0"><Con from="0" weight="1"/></Neuron>
    </NeuralLayer>
    <NeuralOutputs>
      <NeuralOutput outputNeuron="1"><DerivedField optype="continuous" dataType="double">
        <FieldRef field="rNorm"/>
      </DerivedField></NeuralOutput>
    </NeuralOutputs>
  </NeuralNetwork></PMML>"#,
            head = dictionary_head()
        );
        let got = score(&xml, &["x"], &[Value::Number(input)]).unwrap();
        close(number(&got), expected);
    }
}

#[test]
fn reciprocal_of_zero_is_an_error() {
    let xml = r#"<PMML version="4.3"><DataDictionary>
    <DataField name="x" optype="continuous" dataType="double"/>
    <DataField name="r" optype="continuous" dataType="double"/>
  </DataDictionary>
  <TransformationDictionary>
    <DerivedField name="rNorm" optype="continuous" dataType="double">
      <NormContinuous field="r"><LinearNorm orig="0" norm="0"/><LinearNorm orig="1" norm="1"/></NormContinuous>
    </DerivedField>
  </TransformationDictionary>
  <NeuralNetwork functionName="regression" activationFunction="reciprocal">
    <MiningSchema><MiningField name="x"/><MiningField name="r" usageType="predicted"/></MiningSchema>
    <NeuralInputs><NeuralInput id="0"><DerivedField optype="continuous" dataType="double"><FieldRef field="x"/></DerivedField></NeuralInput></NeuralInputs>
    <NeuralLayer><Neuron id="1" bias="0"><Con from="0" weight="1"/></Neuron></NeuralLayer>
    <NeuralOutputs><NeuralOutput outputNeuron="1"><DerivedField optype="continuous" dataType="double"><FieldRef field="rNorm"/></DerivedField></NeuralOutput></NeuralOutputs>
  </NeuralNetwork></PMML>"#;
    let err = score(xml, &["x"], &[Value::Number(0.0)]).unwrap_err();
    assert!(matches!(err, PmmlError::Score(_)), "{err}");
}

#[test]
fn hidden_softmax_and_simplemax_and_output_softmax() {
    // Hidden softmax of logits 1 and 0, copied out through an identity output.
    let hidden_softmax = network_two_hidden("identity", "softmax", "identity", "none");
    let got = score(&hidden_softmax, &["x"], &[Value::Number(1.0)]).unwrap();
    let expected = 1.0_f64.exp() / (1.0_f64.exp() + 1.0);
    close(number(&got), expected);

    // Hidden simplemax of logits 2 and 0 is 1 on the first neuron, which the output copies.
    let hidden_simple = network_two_hidden("identity", "simplemax", "identity", "none");
    let got = score(&hidden_simple, &["x"], &[Value::Number(2.0)]).unwrap();
    close(number(&got), 1.0);

    // Output softmax (`nn_default_a`): logits 1 and 0.
    let output_softmax = categorical_output("softmax");
    let got = score(&output_softmax, &["x"], &[Value::Number(1.0)]).unwrap();
    close(got.confidence.unwrap(), expected);
    assert_eq!(got.value, Value::Text("A".into()));
    close(got.probabilities[0].1, expected);
    close(got.probabilities[1].1, 1.0 - expected);
}

#[test]
fn absent_output_normalization_is_rectified_simplemax() {
    // Logits 0.5, -0.2, -1. Shift by -1 → 1.5, 0.8, 0. Sum 2.3 (`nn_fixed_a` rule).
    let xml = categorical_three();
    let got = score(&xml, &["x"], &[Value::Number(0.0)]).unwrap();
    assert_eq!(got.value, Value::Text("A".into()));
    close(got.probabilities[0].1, 1.5 / 2.3);
    close(got.probabilities[1].1, 0.8 / 2.3);
    close(got.probabilities[2].1, 0.0);
    close(got.confidence.unwrap(), 1.5 / 2.3);
}

#[test]
fn simplemax_of_a_zero_sum_is_rejected() {
    let xml = r#"<PMML version="4.3"><DataDictionary>
    <DataField name="x" optype="continuous" dataType="double"/>
    <DataField name="y" optype="categorical" dataType="string"><Value value="A"/><Value value="B"/></DataField>
  </DataDictionary>
  <TransformationDictionary>
    <DerivedField name="yA" optype="categorical" dataType="double"><NormDiscrete field="y" value="A"/></DerivedField>
    <DerivedField name="yB" optype="categorical" dataType="double"><NormDiscrete field="y" value="B"/></DerivedField>
  </TransformationDictionary>
  <NeuralNetwork functionName="classification" activationFunction="identity">
    <MiningSchema><MiningField name="x"/><MiningField name="y" usageType="predicted"/></MiningSchema>
    <NeuralInputs><NeuralInput id="0"><DerivedField optype="continuous" dataType="double"><FieldRef field="x"/></DerivedField></NeuralInput></NeuralInputs>
    <NeuralLayer normalizationMethod="simplemax">
      <Neuron id="1" bias="0"><Con from="0" weight="0"/></Neuron>
      <Neuron id="2" bias="0"><Con from="0" weight="0"/></Neuron>
    </NeuralLayer>
    <NeuralOutputs>
      <NeuralOutput outputNeuron="1"><DerivedField optype="continuous" dataType="double"><FieldRef field="yA"/></DerivedField></NeuralOutput>
      <NeuralOutput outputNeuron="2"><DerivedField optype="continuous" dataType="double"><FieldRef field="yB"/></DerivedField></NeuralOutput>
    </NeuralOutputs>
  </NeuralNetwork></PMML>"#;
    let err = score(xml, &["x"], &[Value::Number(1.0)]).unwrap_err();
    assert!(matches!(err, PmmlError::Score(_)), "{err}");
}

#[test]
fn rbf_uses_centers_width_and_simplemax() {
    // x=0. Centers 0 and 1, width 1. h = [1, exp(-0.5)], simplemax, output weight +1 and -1.
    let xml = r#"<PMML version="4.3"><DataDictionary>
    <DataField name="x" optype="continuous" dataType="double"/>
    <DataField name="r" optype="continuous" dataType="double"/>
  </DataDictionary>
  <TransformationDictionary>
    <DerivedField name="rNorm" optype="continuous" dataType="double">
      <NormContinuous field="r"><LinearNorm orig="0" norm="0"/><LinearNorm orig="1" norm="1"/></NormContinuous>
    </DerivedField>
  </TransformationDictionary>
  <NeuralNetwork functionName="regression" activationFunction="radialBasis" algorithmName="RBF">
    <MiningSchema><MiningField name="x"/><MiningField name="r" usageType="predicted"/></MiningSchema>
    <NeuralInputs><NeuralInput id="0"><DerivedField optype="continuous" dataType="double"><FieldRef field="x"/></DerivedField></NeuralInput></NeuralInputs>
    <NeuralLayer activationFunction="radialBasis" normalizationMethod="simplemax">
      <Neuron id="1" width="1"><Con from="0" weight="0"/></Neuron>
      <Neuron id="2" width="1"><Con from="0" weight="1"/></Neuron>
    </NeuralLayer>
    <NeuralLayer activationFunction="identity">
      <Neuron id="3" bias="0"><Con from="1" weight="1"/><Con from="2" weight="-1"/></Neuron>
    </NeuralLayer>
    <NeuralOutputs>
      <NeuralOutput outputNeuron="3"><DerivedField optype="continuous" dataType="double"><FieldRef field="rNorm"/></DerivedField></NeuralOutput>
    </NeuralOutputs>
  </NeuralNetwork></PMML>"#;
    let got = score(xml, &["x"], &[Value::Number(0.0)]).unwrap();
    let second = (-0.5_f64).exp();
    let sum = 1.0 + second;
    close(number(&got), 1.0 / sum - second / sum);
}

#[test]
fn missing_input_and_user_missing_values_score_as_missing() {
    let xml = r#"<PMML version="4.3"><DataDictionary>
    <DataField name="cat" optype="categorical" dataType="string">
      <Value value="blue"/>
      <Value property="missing" value="MISSING"/>
    </DataField>
    <DataField name="x" optype="continuous" dataType="double">
      <Value property="missing" value="-99"/>
    </DataField>
    <DataField name="r" optype="continuous" dataType="double"/>
  </DataDictionary>
  <TransformationDictionary>
    <DerivedField name="catBlue" optype="categorical" dataType="double"><NormDiscrete field="cat" value="blue"/></DerivedField>
    <DerivedField name="rNorm" optype="continuous" dataType="double">
      <NormContinuous field="r"><LinearNorm orig="0" norm="0"/><LinearNorm orig="1" norm="1"/></NormContinuous>
    </DerivedField>
  </TransformationDictionary>
  <NeuralNetwork functionName="regression" activationFunction="identity">
    <MiningSchema>
      <MiningField name="cat"/>
      <MiningField name="x"/>
      <MiningField name="r" usageType="predicted"/>
    </MiningSchema>
    <NeuralInputs>
      <NeuralInput id="0"><DerivedField optype="continuous" dataType="double"><FieldRef field="catBlue"/></DerivedField></NeuralInput>
      <NeuralInput id="1"><DerivedField optype="continuous" dataType="double"><FieldRef field="x"/></DerivedField></NeuralInput>
    </NeuralInputs>
    <NeuralLayer>
      <Neuron id="2" bias="0"><Con from="0" weight="1"/><Con from="1" weight="1"/></Neuron>
    </NeuralLayer>
    <NeuralOutputs>
      <NeuralOutput outputNeuron="2"><DerivedField optype="continuous" dataType="double"><FieldRef field="rNorm"/></DerivedField></NeuralOutput>
    </NeuralOutputs>
  </NeuralNetwork></PMML>"#;
    let missing = score(
        xml,
        &["cat", "x"],
        &[Value::Text("MISSING".into()), Value::Number(0.0)],
    )
    .unwrap();
    assert_eq!(missing.value, Value::Missing);

    let numeric = score(
        xml,
        &["cat", "x"],
        &[Value::Text("blue".into()), Value::Number(-99.0)],
    )
    .unwrap();
    assert_eq!(numeric.value, Value::Missing);

    let present = score(
        xml,
        &["cat", "x"],
        &[Value::Text("blue".into()), Value::Number(0.4)],
    )
    .unwrap();
    close(number(&present), 1.4);
}

#[test]
fn user_missing_is_imputed_before_scoring() {
    let xml = r#"<PMML version="4.3"><DataDictionary>
    <DataField name="cat" optype="categorical" dataType="string">
      <Value value="blue"/><Value value="red"/>
      <Value property="missing" value="MISSING"/>
    </DataField>
    <DataField name="r" optype="continuous" dataType="double"/>
  </DataDictionary>
  <TransformationDictionary>
    <DerivedField name="catBlue" optype="categorical" dataType="double"><NormDiscrete field="cat" value="blue"/></DerivedField>
    <DerivedField name="rNorm" optype="continuous" dataType="double">
      <NormContinuous field="r"><LinearNorm orig="0" norm="0"/><LinearNorm orig="1" norm="1"/></NormContinuous>
    </DerivedField>
  </TransformationDictionary>
  <NeuralNetwork functionName="regression" activationFunction="identity">
    <MiningSchema>
      <MiningField name="cat" missingValueReplacement="blue"/>
      <MiningField name="r" usageType="predicted"/>
    </MiningSchema>
    <NeuralInputs>
      <NeuralInput id="0"><DerivedField optype="continuous" dataType="double"><FieldRef field="catBlue"/></DerivedField></NeuralInput>
    </NeuralInputs>
    <NeuralLayer><Neuron id="1" bias="0"><Con from="0" weight="1"/></Neuron></NeuralLayer>
    <NeuralOutputs>
      <NeuralOutput outputNeuron="1"><DerivedField optype="continuous" dataType="double"><FieldRef field="rNorm"/></DerivedField></NeuralOutput>
    </NeuralOutputs>
  </NeuralNetwork></PMML>"#;
    let got = score(xml, &["cat"], &[Value::Text("MISSING".into())]).unwrap();
    close(number(&got), 1.0);
}

#[test]
fn adjusted_propensity_matches_the_partition_extension() {
    let xml = r#"<PMML version="4.3"><DataDictionary>
    <DataField name="x" optype="continuous" dataType="double"/>
    <DataField name="binary" optype="categorical" dataType="integer">
      <Extension extender="spss.com" name="flagTypeTrueValue" value="1"/>
      <Value value="0"/><Value value="1"/>
    </DataField>
  </DataDictionary>
  <TransformationDictionary>
    <DerivedField name="b0" optype="categorical" dataType="double"><NormDiscrete field="binary" value="0"/></DerivedField>
    <DerivedField name="b1" optype="categorical" dataType="double"><NormDiscrete field="binary" value="1"/></DerivedField>
  </TransformationDictionary>
  <NeuralNetwork functionName="classification" activationFunction="tanh">
    <Extension extender="spss.com" name="adjustedPropensity">
      <AdjustedPropensityByNN description="standalone">
        <HiddenLayer activationFunction="tanh">
          <Matrix><Array n="3" type="real">-1.85841715370082 -2.17624318668412 -1.34722037387433</Array></Matrix>
          <Array n="3" type="real">-1.02722843348216 -0.342824111383914 -0.550346488173681</Array>
        </HiddenLayer>
        <OutputLayer activationFunction="identity">
          <Matrix>
            <Array n="1" type="real">0.00388612714366912</Array>
            <Array n="1" type="real">0.117231589116019</Array>
            <Array n="1" type="real">-0.888388506784424</Array>
          </Matrix>
          <Array n="1" type="real">0.0685381874597424</Array>
        </OutputLayer>
      </AdjustedPropensityByNN>
    </Extension>
    <MiningSchema><MiningField name="x"/><MiningField name="binary" usageType="predicted"/></MiningSchema>
    <NeuralInputs><NeuralInput id="0"><DerivedField optype="continuous" dataType="double"><FieldRef field="x"/></DerivedField></NeuralInput></NeuralInputs>
    <NeuralLayer><Neuron id="1" bias="0"><Con from="0" weight="1"/></Neuron></NeuralLayer>
    <NeuralOutputs>
      <NeuralOutput outputNeuron="1"><DerivedField optype="continuous" dataType="double"><FieldRef field="b0"/></DerivedField></NeuralOutput>
    </NeuralOutputs>
  </NeuralNetwork></PMML>"#;
    let pmml = Pmml::parse(xml).unwrap();
    let Model::Neural(network) = &pmml.model else {
        panic!("expected a neural network");
    };
    close(
        network.adjusted_propensity(0.0).unwrap(),
        0.47173608356033014,
    );
    close(
        network.adjusted_propensity(0.183582125499843).unwrap(),
        0.5799681303327233,
    );
    assert_eq!(
        pmml.data_dictionary
            .iter()
            .find(|field| field.name == "binary")
            .and_then(|field| field.flag_true_value.as_deref()),
        Some("1")
    );
}

fn network_two_hidden(
    hidden_activation: &str,
    hidden_norm: &str,
    output_activation: &str,
    output_norm: &str,
) -> String {
    format!(
        r#"{head}
    <DataField name="x" optype="continuous" dataType="double"/>
    <DataField name="r" optype="continuous" dataType="double"/>
  </DataDictionary>
  <TransformationDictionary>
    <DerivedField name="rNorm" optype="continuous" dataType="double">
      <NormContinuous field="r"><LinearNorm orig="0" norm="0"/><LinearNorm orig="1" norm="1"/></NormContinuous>
    </DerivedField>
  </TransformationDictionary>
  <NeuralNetwork functionName="regression" activationFunction="{hidden_activation}">
    <MiningSchema><MiningField name="x"/><MiningField name="r" usageType="predicted"/></MiningSchema>
    <NeuralInputs><NeuralInput id="0"><DerivedField optype="continuous" dataType="double"><FieldRef field="x"/></DerivedField></NeuralInput></NeuralInputs>
    <NeuralLayer activationFunction="{hidden_activation}" normalizationMethod="{hidden_norm}">
      <Neuron id="1" bias="0"><Con from="0" weight="1"/></Neuron>
      <Neuron id="2" bias="0"><Con from="0" weight="0"/></Neuron>
    </NeuralLayer>
    <NeuralLayer activationFunction="{output_activation}" normalizationMethod="{output_norm}">
      <Neuron id="3" bias="0"><Con from="1" weight="1"/><Con from="2" weight="0"/></Neuron>
    </NeuralLayer>
    <NeuralOutputs>
      <NeuralOutput outputNeuron="3"><DerivedField optype="continuous" dataType="double"><FieldRef field="rNorm"/></DerivedField></NeuralOutput>
    </NeuralOutputs>
  </NeuralNetwork></PMML>"#,
        head = dictionary_head()
    )
}

fn categorical_output(normalization: &str) -> String {
    format!(
        r#"{head}
    <DataField name="x" optype="continuous" dataType="double"/>
    <DataField name="y" optype="categorical" dataType="string"><Value value="A"/><Value value="B"/></DataField>
  </DataDictionary>
  <TransformationDictionary>
    <DerivedField name="yA" optype="categorical" dataType="double"><NormDiscrete field="y" value="A"/></DerivedField>
    <DerivedField name="yB" optype="categorical" dataType="double"><NormDiscrete field="y" value="B"/></DerivedField>
  </TransformationDictionary>
  <NeuralNetwork functionName="classification" activationFunction="identity">
    <MiningSchema><MiningField name="x"/><MiningField name="y" usageType="predicted"/></MiningSchema>
    <NeuralInputs><NeuralInput id="0"><DerivedField optype="continuous" dataType="double"><FieldRef field="x"/></DerivedField></NeuralInput></NeuralInputs>
    <NeuralLayer normalizationMethod="{normalization}">
      <Neuron id="1" bias="0"><Con from="0" weight="1"/></Neuron>
      <Neuron id="2" bias="0"><Con from="0" weight="0"/></Neuron>
    </NeuralLayer>
    <NeuralOutputs>
      <NeuralOutput outputNeuron="1"><DerivedField optype="continuous" dataType="double"><FieldRef field="yA"/></DerivedField></NeuralOutput>
      <NeuralOutput outputNeuron="2"><DerivedField optype="continuous" dataType="double"><FieldRef field="yB"/></DerivedField></NeuralOutput>
    </NeuralOutputs>
  </NeuralNetwork></PMML>"#,
        head = dictionary_head()
    )
}

fn categorical_three() -> String {
    format!(
        r#"{head}
    <DataField name="x" optype="continuous" dataType="double"/>
    <DataField name="y" optype="categorical" dataType="string">
      <Value value="A"/><Value value="B"/><Value value="C"/>
    </DataField>
  </DataDictionary>
  <TransformationDictionary>
    <DerivedField name="yA" optype="categorical" dataType="double"><NormDiscrete field="y" value="A"/></DerivedField>
    <DerivedField name="yB" optype="categorical" dataType="double"><NormDiscrete field="y" value="B"/></DerivedField>
    <DerivedField name="yC" optype="categorical" dataType="double"><NormDiscrete field="y" value="C"/></DerivedField>
  </TransformationDictionary>
  <NeuralNetwork functionName="classification" activationFunction="identity">
    <MiningSchema><MiningField name="x"/><MiningField name="y" usageType="predicted"/></MiningSchema>
    <NeuralInputs><NeuralInput id="0"><DerivedField optype="continuous" dataType="double"><FieldRef field="x"/></DerivedField></NeuralInput></NeuralInputs>
    <NeuralLayer activationFunction="identity">
      <Neuron id="1" bias="0.5"><Con from="0" weight="0"/></Neuron>
      <Neuron id="2" bias="-0.2"><Con from="0" weight="0"/></Neuron>
      <Neuron id="3" bias="-1"><Con from="0" weight="0"/></Neuron>
    </NeuralLayer>
    <NeuralOutputs>
      <NeuralOutput outputNeuron="1"><DerivedField optype="continuous" dataType="double"><FieldRef field="yA"/></DerivedField></NeuralOutput>
      <NeuralOutput outputNeuron="2"><DerivedField optype="continuous" dataType="double"><FieldRef field="yB"/></DerivedField></NeuralOutput>
      <NeuralOutput outputNeuron="3"><DerivedField optype="continuous" dataType="double"><FieldRef field="yC"/></DerivedField></NeuralOutput>
    </NeuralOutputs>
  </NeuralNetwork></PMML>"#,
        head = dictionary_head()
    )
}
