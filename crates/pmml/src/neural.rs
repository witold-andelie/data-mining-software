//! PMML 4.3 `NeuralNetwork` scoring for Modeler Neural Net nuggets.
//!
//! Rules locked to oracle exports (DMG NeuralNetwork, checked against the scored CSVs):
//!
//! - A non-RBF neuron is `bias` (default 0) plus `Σ weight * from`, then its activation.
//!   A layer that omits `activationFunction` inherits the network's (`tanh` on `nn_fixed_a`
//!   and `nn_default_a`). Hidden layers are then normalized: `none`, `simplemax`
//!   (`y_i / Σ y_j`, `nn_rbf`), or `softmax`. A zero or non-finite sum is an error.
//! - The output layer's activation is kept raw. Each categorical target is normalized on
//!   its own, not together with a continuous neuron (`nn_mixed_targets`): declared
//!   `softmax` (`nn_default_a`, `models/neuralnet_default`), declared `simplemax`, or, when
//!   `normalizationMethod` is absent, rectified simplemax — `shift = min(0, min z)`, then
//!   divide by the shifted sum (`nn_fixed_a` grid 43/43 and train 180/180). Output-layer
//!   normalization together with a continuous output is rejected.
//! - RBF (`nn_rbf`, `activationFunction="radialBasis"`) treats weights as centers and does
//!   not add bias: `altitude * exp(-Σ(x - w)² / (2 width²))` with `altitude` default 1.
//!   A missing or non-positive `width` is an error. Hidden `simplemax` follows.
//! - Continuous outputs are the inverse of `NormContinuous` (`asIs` extrapolates;
//!   `nn_regression`, `nn_rbf`). The predicted class is the first maximum (`>`), in
//!   `NeuralOutput` order.
//! - A missing neural input, after user-missing values and `missingValueReplacement`,
//!   scores every target as missing (`nn_missing_listwise`, `nn_missing_impute`). It does
//!   not fail the row.
//! - Adjusted propensity (`nn_partition`) is the `adjustedPropensity` extension, not
//!   `DefineFunction` `UnknownScoreAlgorithm`: `h_j = tanh(bias_j + weight_j * raw)`,
//!   `out = bias_o + Σ weight_o_j * h_j`. The value is not clamped.
//!
//! GAP: a model with two categorical targets has no oracle. GAP: raw propensity with no
//! `flagTypeTrueValue` uses the second category; no non-flag neural net oracle shows that.
//!
//! Counts of layers, neurons, and connections are capped. An unknown activation,
//! normalization, connection, or non-finite weight is an error.

use crate::expr::{self, denormalize};
use crate::parse::{self, opt_f64_attr, req_attr};
use crate::score::Env;
use crate::{DerivedField, Expr, Outliers, Pmml, PmmlError, Value};
use std::collections::{HashMap, HashSet};

const MAX_LAYERS: usize = 64;
const MAX_UNITS: usize = 100_000;
const MAX_CONNECTIONS: usize = 1_000_000;
const MAX_OUTPUTS: usize = 100_000;
const MAX_PROPENSITY: usize = 10_000;
const MAX_ARRAY: usize = 100_000;

/// One scored target. `probabilities` is empty for a continuous target or a missing score.
#[derive(Debug, Clone, PartialEq)]
pub struct NeuralTarget {
    pub name: String,
    pub value: Value,
    /// `(category, probability)` in `DataField` value order when the field lists them.
    pub probabilities: Vec<(String, f64)>,
}

/// PMML `activationFunction` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activation {
    Threshold,
    Logistic,
    Tanh,
    Identity,
    Exponential,
    Reciprocal,
    Square,
    Gauss,
    Sine,
    Cosine,
    Elliott,
    Arctan,
    RadialBasis,
}

/// PMML `normalizationMethod`. `None` on a hidden layer leaves values unchanged.
/// On a categorical output group it means rectified simplemax (`nn_fixed_a`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Normalization {
    None,
    Simplemax,
    Softmax,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NeuralNetworkModel {
    pub header: crate::ModelHeader,
    pub activation: Activation,
    pub inputs: Vec<NeuralInput>,
    pub layers: Vec<NeuralLayer>,
    pub outputs: Vec<NeuralOutput>,
    /// SPSS `Extension name="adjustedPropensity"` (`nn_partition`).
    pub adjusted_propensity: Option<AdjustedPropensity>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NeuralInput {
    pub id: String,
    pub expr: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NeuralLayer {
    pub activation: Activation,
    pub normalization: Normalization,
    pub neurons: Vec<Neuron>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Neuron {
    pub id: String,
    pub bias: f64,
    pub activation: Activation,
    /// `(from id, weight)`. For `radialBasis`, weights are centers.
    pub connections: Vec<(String, f64)>,
    pub width: Option<f64>,
    pub altitude: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NeuralOutput {
    pub neuron_id: String,
    /// Index into the last layer.
    pub neuron_index: usize,
    pub target: String,
    pub kind: NeuralOutputKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NeuralOutputKind {
    Category(String),
    Continuous(ContinuousNorm),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContinuousNorm {
    pub points: Vec<(f64, f64)>,
    pub outliers: Outliers,
    pub map_missing_to: Option<f64>,
}

/// The small network inside `adjustedPropensity` (`nn_partition`).
#[derive(Debug, Clone, PartialEq)]
pub struct AdjustedPropensity {
    pub hidden_activation: Activation,
    pub hidden_weights: Vec<f64>,
    pub hidden_bias: Vec<f64>,
    pub output_activation: Activation,
    pub output_weights: Vec<f64>,
    pub output_bias: f64,
}

impl NeuralNetworkModel {
    /// `h_j = activation(bias_j + weight_j * raw)`, then the output layer.
    /// `nn_partition`: raw 0 → about 0.471736083560330, raw 0.183582125499843 → about 0.579968130332723.
    pub fn adjusted_propensity(&self, raw: f64) -> Result<f64, PmmlError> {
        let net = self.adjusted_propensity.as_ref().ok_or_else(|| {
            PmmlError::Score("neural network has no adjusted propensity extension".into())
        })?;
        if !raw.is_finite() {
            return Err(PmmlError::Score("raw propensity is not finite".into()));
        }
        if net.hidden_weights.len() != net.hidden_bias.len()
            || net.output_weights.len() != net.hidden_weights.len()
            || net.hidden_weights.is_empty()
        {
            return Err(PmmlError::Score(
                "adjusted propensity weight and bias lengths differ".into(),
            ));
        }
        let mut hidden = Vec::with_capacity(net.hidden_weights.len());
        for (bias, weight) in net.hidden_bias.iter().zip(&net.hidden_weights) {
            hidden.push(activate(net.hidden_activation, bias + weight * raw)?);
        }
        let mut combined = net.output_bias;
        for (activation, weight) in hidden.iter().zip(&net.output_weights) {
            combined += weight * activation;
        }
        if !combined.is_finite() {
            return Err(PmmlError::Score(
                "adjusted propensity output is not finite".into(),
            ));
        }
        activate(net.output_activation, combined)
    }
}

pub(crate) fn parse(
    node: roxmltree::Node<'_, '_>,
    dictionary: &[DerivedField],
) -> Result<NeuralNetworkModel, PmmlError> {
    let mut header = parse::parse_model_header(node)?;
    // Modeler's neural nets treat an invalid input (empty, blank, unseen category) as
    // missing when the PMML leaves `invalidValueTreatment` unset (`nn_missing_impute`:
    // "", "   ", "unseen" score like "MISSING"; `nn_missing_listwise`: all null).
    let explicit: HashSet<String> = node
        .children()
        .filter(|c| c.tag_name().name() == "MiningSchema")
        .flat_map(|schema| schema.children().filter(roxmltree::Node::is_element))
        .filter(|f| f.attribute("invalidValueTreatment").is_some())
        .filter_map(|f| f.attribute("name").map(str::to_string))
        .collect();
    for field in &mut header.mining_schema {
        if !explicit.contains(&field.name) {
            field.invalid_treatment = crate::InvalidTreatment::AsMissing;
        }
    }
    let activation = parse_activation(req_attr(node, "activationFunction")?)?;
    let mut inputs = None;
    let mut layers = Vec::new();
    let mut outputs = None;
    let mut adjusted = None;
    let mut ids = HashSet::new();
    let mut connections = 0usize;

    for child in node.children().filter(roxmltree::Node::is_element) {
        let tag = child.tag_name().name();
        if is_skipped(tag) {
            continue;
        }
        match tag {
            "Extension" => {
                if child.attribute("name") == Some("adjustedPropensity") {
                    if adjusted.is_some() {
                        return Err(PmmlError::Parse(
                            "duplicate adjustedPropensity extension".into(),
                        ));
                    }
                    adjusted = Some(parse_adjusted(child)?);
                }
            }
            "NeuralInputs" => {
                if inputs.is_some() {
                    return Err(PmmlError::Parse("duplicate NeuralInputs".into()));
                }
                inputs = Some(parse_inputs(child, &mut ids)?);
            }
            "NeuralLayer" => {
                if layers.len() >= MAX_LAYERS {
                    return Err(PmmlError::Parse(
                        "neural network has too many layers".into(),
                    ));
                }
                layers.push(parse_layer(child, activation, &mut ids, &mut connections)?);
            }
            "NeuralOutputs" => {
                if outputs.is_some() {
                    return Err(PmmlError::Parse("duplicate NeuralOutputs".into()));
                }
                outputs = Some(child);
            }
            other => {
                return Err(PmmlError::Parse(format!(
                    "unexpected element {other:?} in NeuralNetwork"
                )));
            }
        }
    }

    let inputs =
        inputs.ok_or_else(|| PmmlError::Parse("NeuralNetwork has no NeuralInputs".into()))?;
    if layers.is_empty() {
        return Err(PmmlError::Parse("NeuralNetwork has no NeuralLayer".into()));
    }
    let output_node =
        outputs.ok_or_else(|| PmmlError::Parse("NeuralNetwork has no NeuralOutputs".into()))?;
    let last = layers
        .last()
        .ok_or_else(|| PmmlError::Parse("NeuralNetwork has no NeuralLayer".into()))?;
    let outputs = parse_outputs(output_node, last, dictionary, &header.local_transformations)?;
    let continuous = outputs
        .iter()
        .any(|output| matches!(output.kind, NeuralOutputKind::Continuous(_)));
    if continuous && last.normalization != Normalization::None {
        return Err(PmmlError::Unsupported(
            "output normalization with a continuous neural output is not supported".into(),
        ));
    }
    check_targets(&outputs)?;

    Ok(NeuralNetworkModel {
        header,
        activation,
        inputs,
        layers,
        outputs,
        adjusted_propensity: adjusted,
    })
}

fn is_skipped(tag: &str) -> bool {
    matches!(
        tag,
        "MiningSchema"
            | "Output"
            | "LocalTransformations"
            | "ModelExplanation"
            | "ModelStats"
            | "ModelVerification"
            | "Targets"
            | "Taxonomy"
    ) || tag.starts_with("X-")
}

fn parse_inputs(
    node: roxmltree::Node<'_, '_>,
    ids: &mut HashSet<String>,
) -> Result<Vec<NeuralInput>, PmmlError> {
    let mut inputs = Vec::new();
    for child in node.children().filter(roxmltree::Node::is_element) {
        let tag = child.tag_name().name();
        if tag == "Extension" {
            continue;
        }
        if tag != "NeuralInput" {
            return Err(PmmlError::Parse(format!(
                "unexpected element {tag:?} in NeuralInputs"
            )));
        }
        if ids.len() >= MAX_UNITS {
            return Err(PmmlError::Parse("neural network has too many units".into()));
        }
        let id = req_attr(child, "id")?.to_string();
        insert_id(ids, &id)?;
        let expr = sole_derived_expr(child, "NeuralInput")?;
        inputs.push(NeuralInput { id, expr });
    }
    if inputs.is_empty() {
        return Err(PmmlError::Parse("NeuralInputs is empty".into()));
    }
    if let Some(declared) = node.attribute("numberOfInputs") {
        let count = parse_count(declared)?;
        if count != inputs.len() {
            return Err(PmmlError::Parse(format!(
                "NeuralInputs numberOfInputs is {count} but {} NeuralInput elements were found",
                inputs.len()
            )));
        }
    }
    Ok(inputs)
}

fn parse_layer(
    node: roxmltree::Node<'_, '_>,
    network_activation: Activation,
    ids: &mut HashSet<String>,
    connections: &mut usize,
) -> Result<NeuralLayer, PmmlError> {
    let activation = match node.attribute("activationFunction") {
        Some(text) => parse_activation(text)?,
        None => network_activation,
    };
    let normalization = match node.attribute("normalizationMethod") {
        Some(text) => parse_normalization(text)?,
        None => Normalization::None,
    };
    let mut neurons = Vec::new();
    for child in node.children().filter(roxmltree::Node::is_element) {
        let tag = child.tag_name().name();
        if tag == "Extension" {
            continue;
        }
        if tag != "Neuron" {
            return Err(PmmlError::Parse(format!(
                "unexpected element {tag:?} in NeuralLayer"
            )));
        }
        if ids.len() >= MAX_UNITS {
            return Err(PmmlError::Parse("neural network has too many units".into()));
        }
        neurons.push(parse_neuron(child, activation, ids, connections)?);
    }
    if neurons.is_empty() {
        return Err(PmmlError::Parse("NeuralLayer has no Neuron".into()));
    }
    if let Some(declared) = node.attribute("numberOfNeurons") {
        let count = parse_count(declared)?;
        if count != neurons.len() {
            return Err(PmmlError::Parse(format!(
                "NeuralLayer numberOfNeurons is {count} but {} Neuron elements were found",
                neurons.len()
            )));
        }
    }
    Ok(NeuralLayer {
        activation,
        normalization,
        neurons,
    })
}

fn parse_neuron(
    node: roxmltree::Node<'_, '_>,
    layer_activation: Activation,
    ids: &mut HashSet<String>,
    connections: &mut usize,
) -> Result<Neuron, PmmlError> {
    let id = req_attr(node, "id")?.to_string();
    let bias = opt_f64_attr(node, "bias")?.unwrap_or(0.0);
    let width = opt_f64_attr(node, "width")?;
    let altitude = opt_f64_attr(node, "altitude")?.unwrap_or(1.0);
    let activation = match node.attribute("activationFunction") {
        Some(text) => parse_activation(text)?,
        None => layer_activation,
    };
    if activation == Activation::RadialBasis {
        match width {
            Some(value) if value > 0.0 => {}
            _ => {
                return Err(PmmlError::Parse(format!(
                    "radialBasis neuron {id:?} needs a positive width"
                )));
            }
        }
    }
    let mut cons = Vec::new();
    let mut seen_from = HashSet::new();
    for child in node.children().filter(roxmltree::Node::is_element) {
        let tag = child.tag_name().name();
        if tag == "Extension" {
            continue;
        }
        if tag != "Con" {
            return Err(PmmlError::Parse(format!(
                "unexpected element {tag:?} in Neuron"
            )));
        }
        *connections = connections.saturating_add(1);
        if *connections > MAX_CONNECTIONS {
            return Err(PmmlError::Parse(
                "neural network has too many connections".into(),
            ));
        }
        let from = req_attr(child, "from")?.to_string();
        if !ids.contains(&from) {
            return Err(PmmlError::Parse(format!(
                "neuron {id:?} connects from unknown id {from:?}"
            )));
        }
        if !seen_from.insert(from.clone()) {
            return Err(PmmlError::Parse(format!(
                "neuron {id:?} connects from {from:?} twice"
            )));
        }
        let weight = parse::parse_f64(req_attr(child, "weight")?, "weight")?;
        cons.push((from, weight));
    }
    insert_id(ids, &id)?;
    Ok(Neuron {
        id,
        bias,
        activation,
        connections: cons,
        width,
        altitude,
    })
}

fn parse_outputs(
    node: roxmltree::Node<'_, '_>,
    last: &NeuralLayer,
    dictionary: &[DerivedField],
    local: &[DerivedField],
) -> Result<Vec<NeuralOutput>, PmmlError> {
    let mut outputs = Vec::new();
    let mut seen_neurons = HashSet::new();
    for child in node.children().filter(roxmltree::Node::is_element) {
        let tag = child.tag_name().name();
        if tag == "Extension" {
            continue;
        }
        if tag != "NeuralOutput" {
            return Err(PmmlError::Parse(format!(
                "unexpected element {tag:?} in NeuralOutputs"
            )));
        }
        if outputs.len() >= MAX_OUTPUTS {
            return Err(PmmlError::Parse(
                "neural network has too many outputs".into(),
            ));
        }
        let neuron_id = req_attr(child, "outputNeuron")?.to_string();
        if !seen_neurons.insert(neuron_id.clone()) {
            return Err(PmmlError::Parse(format!(
                "neural output neuron {neuron_id:?} is used twice"
            )));
        }
        let neuron_index = last
            .neurons
            .iter()
            .position(|neuron| neuron.id == neuron_id)
            .ok_or_else(|| {
                PmmlError::Parse(format!(
                    "neural output neuron {neuron_id:?} is not in the output layer"
                ))
            })?;
        let expr = sole_derived_expr(child, "NeuralOutput")?;
        let (target, kind) = resolve_output(&expr, dictionary, local)?;
        outputs.push(NeuralOutput {
            neuron_id,
            neuron_index,
            target,
            kind,
        });
    }
    if outputs.is_empty() {
        return Err(PmmlError::Parse("NeuralOutputs is empty".into()));
    }
    if let Some(declared) = node.attribute("numberOfOutputs") {
        let count = parse_count(declared)?;
        if count != outputs.len() {
            return Err(PmmlError::Parse(format!(
                "NeuralOutputs numberOfOutputs is {count} but {} NeuralOutput elements were found",
                outputs.len()
            )));
        }
    }
    Ok(outputs)
}

fn resolve_output(
    expr: &Expr,
    dictionary: &[DerivedField],
    local: &[DerivedField],
) -> Result<(String, NeuralOutputKind), PmmlError> {
    let resolved = match expr {
        Expr::FieldRef { field, .. } => {
            let derived = local
                .iter()
                .chain(dictionary.iter())
                .find(|derived| &derived.name == field)
                .ok_or_else(|| {
                    PmmlError::Parse(format!("neural output field {field:?} is not defined"))
                })?;
            match &derived.expr {
                Expr::FieldRef { field: inner, .. } => {
                    return Err(PmmlError::Parse(format!(
                        "neural output {inner:?} does not resolve to NormDiscrete or NormContinuous"
                    )));
                }
                other => other,
            }
        }
        other => other,
    };
    match resolved {
        Expr::NormDiscrete { field, value, .. } => {
            Ok((field.clone(), NeuralOutputKind::Category(value.clone())))
        }
        Expr::NormContinuous {
            field,
            points,
            outliers,
            map_missing_to,
        } => Ok((
            field.clone(),
            NeuralOutputKind::Continuous(ContinuousNorm {
                points: points.clone(),
                outliers: *outliers,
                map_missing_to: *map_missing_to,
            }),
        )),
        _ => Err(PmmlError::Unsupported("neural output expression".into())),
    }
}

fn check_targets(outputs: &[NeuralOutput]) -> Result<(), PmmlError> {
    let mut continuous: HashSet<&str> = HashSet::new();
    let mut categories: HashSet<(&str, &str)> = HashSet::new();
    for output in outputs {
        match &output.kind {
            NeuralOutputKind::Continuous(_) => {
                if !continuous.insert(output.target.as_str()) {
                    return Err(PmmlError::Parse(format!(
                        "neural target {:?} has two continuous outputs",
                        output.target
                    )));
                }
                if categories
                    .iter()
                    .any(|(target, _)| *target == output.target)
                {
                    return Err(PmmlError::Parse(format!(
                        "neural target {:?} is both continuous and categorical",
                        output.target
                    )));
                }
            }
            NeuralOutputKind::Category(category) => {
                if continuous.contains(output.target.as_str()) {
                    return Err(PmmlError::Parse(format!(
                        "neural target {:?} is both continuous and categorical",
                        output.target
                    )));
                }
                if !categories.insert((output.target.as_str(), category.as_str())) {
                    return Err(PmmlError::Parse(format!(
                        "neural target {:?} repeats category {category:?}",
                        output.target
                    )));
                }
            }
        }
    }
    Ok(())
}

fn sole_derived_expr(node: roxmltree::Node<'_, '_>, owner: &str) -> Result<Expr, PmmlError> {
    let mut found = None;
    for child in node.children().filter(roxmltree::Node::is_element) {
        if child.tag_name().name() == "Extension" {
            continue;
        }
        if child.tag_name().name() != "DerivedField" {
            return Err(PmmlError::Parse(format!(
                "unexpected element {:?} in {owner}",
                child.tag_name().name()
            )));
        }
        if found.is_some() {
            return Err(PmmlError::Parse(format!("{owner} has two DerivedFields")));
        }
        found = Some(expression_of(child)?);
    }
    found.ok_or_else(|| PmmlError::Parse(format!("{owner} is missing a DerivedField")))
}

fn expression_of(derived: roxmltree::Node<'_, '_>) -> Result<Expr, PmmlError> {
    let mut expr = None;
    for child in derived.children().filter(roxmltree::Node::is_element) {
        let tag = child.tag_name().name();
        if matches!(tag, "Extension" | "Value" | "Interval") {
            continue;
        }
        if expr.is_some() {
            return Err(PmmlError::Parse(
                "neural DerivedField has multiple expressions".into(),
            ));
        }
        expr = Some(parse::parse_expr(child, 0)?);
    }
    expr.ok_or_else(|| PmmlError::Parse("neural DerivedField is missing an expression".into()))
}

fn parse_adjusted(extension: roxmltree::Node<'_, '_>) -> Result<AdjustedPropensity, PmmlError> {
    let mut body = None;
    for child in extension.children().filter(roxmltree::Node::is_element) {
        match child.tag_name().name() {
            "Extension" => {}
            "AdjustedPropensityByNN" => {
                if body.is_some() {
                    return Err(PmmlError::Parse("duplicate AdjustedPropensityByNN".into()));
                }
                body = Some(parse_adjusted_body(child)?);
            }
            other => {
                return Err(PmmlError::Parse(format!(
                    "unexpected element {other:?} in adjustedPropensity"
                )));
            }
        }
    }
    body.ok_or_else(|| PmmlError::Parse("adjustedPropensity has no network".into()))
}

fn parse_adjusted_body(node: roxmltree::Node<'_, '_>) -> Result<AdjustedPropensity, PmmlError> {
    let mut hidden = None;
    let mut output = None;
    for child in node.children().filter(roxmltree::Node::is_element) {
        match child.tag_name().name() {
            "Extension" => {}
            "HiddenLayer" => {
                if hidden.is_some() {
                    return Err(PmmlError::Parse(
                        "adjusted propensity has two hidden layers".into(),
                    ));
                }
                hidden = Some(parse_propensity_layer(child)?);
            }
            "OutputLayer" => {
                if output.is_some() {
                    return Err(PmmlError::Parse(
                        "adjusted propensity has two output layers".into(),
                    ));
                }
                output = Some(parse_propensity_layer(child)?);
            }
            other => {
                return Err(PmmlError::Parse(format!(
                    "unexpected element {other:?} in AdjustedPropensityByNN"
                )));
            }
        }
    }
    let (hidden_activation, hidden_weights, hidden_bias) =
        hidden.ok_or_else(|| PmmlError::Parse("adjusted propensity has no hidden layer".into()))?;
    let (output_activation, output_weights, output_bias) =
        output.ok_or_else(|| PmmlError::Parse("adjusted propensity has no output layer".into()))?;
    if hidden_weights.is_empty()
        || hidden_weights.len() > MAX_PROPENSITY
        || hidden_weights.len() != hidden_bias.len()
        || output_weights.len() != hidden_weights.len()
    {
        return Err(PmmlError::Parse(
            "adjusted propensity weight and bias lengths differ".into(),
        ));
    }
    let output_bias = match output_bias.as_slice() {
        [bias] => *bias,
        _ => {
            return Err(PmmlError::Parse(
                "adjusted propensity output bias must be one number".into(),
            ));
        }
    };
    if !matches!(hidden_activation, Activation::Tanh | Activation::Identity)
        || !matches!(output_activation, Activation::Tanh | Activation::Identity)
    {
        return Err(PmmlError::Unsupported(
            "adjusted propensity activation is not tanh or identity".into(),
        ));
    }
    Ok(AdjustedPropensity {
        hidden_activation,
        hidden_weights,
        hidden_bias,
        output_activation,
        output_weights,
        output_bias,
    })
}

fn parse_propensity_layer(
    node: roxmltree::Node<'_, '_>,
) -> Result<(Activation, Vec<f64>, Vec<f64>), PmmlError> {
    let activation = parse_activation(req_attr(node, "activationFunction")?)?;
    let mut matrix = None;
    let mut bias = None;
    for child in node.children().filter(roxmltree::Node::is_element) {
        match child.tag_name().name() {
            "Extension" => {}
            "Matrix" => {
                if matrix.is_some() {
                    return Err(PmmlError::Parse(
                        "adjusted propensity layer has two matrices".into(),
                    ));
                }
                matrix = Some(parse_matrix(child)?);
            }
            "Array" => {
                if bias.is_some() {
                    return Err(PmmlError::Parse(
                        "adjusted propensity layer has two bias arrays".into(),
                    ));
                }
                bias = Some(parse_array(child)?);
            }
            other => {
                return Err(PmmlError::Parse(format!(
                    "unexpected element {other:?} in adjusted propensity layer"
                )));
            }
        }
    }
    let weights =
        matrix.ok_or_else(|| PmmlError::Parse("adjusted propensity layer has no matrix".into()))?;
    let bias =
        bias.ok_or_else(|| PmmlError::Parse("adjusted propensity layer has no bias".into()))?;
    Ok((activation, weights, bias))
}

fn parse_matrix(node: roxmltree::Node<'_, '_>) -> Result<Vec<f64>, PmmlError> {
    let mut values = Vec::new();
    for child in node.children().filter(roxmltree::Node::is_element) {
        let tag = child.tag_name().name();
        if tag == "Extension" {
            continue;
        }
        if tag != "Array" {
            return Err(PmmlError::Parse(format!(
                "unexpected element {tag:?} in Matrix"
            )));
        }
        values.extend(parse_array(child)?);
        if values.len() > MAX_ARRAY {
            return Err(PmmlError::Parse("Array is too long".into()));
        }
    }
    if values.is_empty() {
        return Err(PmmlError::Parse("Matrix has no numbers".into()));
    }
    Ok(values)
}

fn parse_array(node: roxmltree::Node<'_, '_>) -> Result<Vec<f64>, PmmlError> {
    let mut values = Vec::new();
    let mut text = String::new();
    for child in node.children() {
        if let Some(piece) = child.text() {
            text.push_str(piece);
        }
    }
    for token in text.split_whitespace() {
        if values.len() >= MAX_ARRAY {
            return Err(PmmlError::Parse("Array is too long".into()));
        }
        values.push(parse::parse_f64(token, "Array")?);
    }
    if let Some(declared) = node.attribute("n") {
        let count = parse_count(declared)?;
        if count != values.len() {
            return Err(PmmlError::Parse(format!(
                "Array n={count} but {} numbers were found",
                values.len()
            )));
        }
    }
    Ok(values)
}

fn insert_id(ids: &mut HashSet<String>, id: &str) -> Result<(), PmmlError> {
    if id.is_empty() {
        return Err(PmmlError::Parse("neural id is empty".into()));
    }
    if !ids.insert(id.to_string()) {
        return Err(PmmlError::Parse(format!("duplicate neural id {id:?}")));
    }
    Ok(())
}

fn parse_count(text: &str) -> Result<usize, PmmlError> {
    text.trim()
        .parse::<usize>()
        .map_err(|_| PmmlError::Parse(format!("invalid count {text:?}")))
}

fn parse_activation(text: &str) -> Result<Activation, PmmlError> {
    match text {
        "threshold" => Ok(Activation::Threshold),
        "logistic" => Ok(Activation::Logistic),
        "tanh" => Ok(Activation::Tanh),
        "identity" => Ok(Activation::Identity),
        "exponential" => Ok(Activation::Exponential),
        "reciprocal" => Ok(Activation::Reciprocal),
        "square" => Ok(Activation::Square),
        "Gauss" => Ok(Activation::Gauss),
        "sine" => Ok(Activation::Sine),
        "cosine" => Ok(Activation::Cosine),
        "Elliott" => Ok(Activation::Elliott),
        "arctan" => Ok(Activation::Arctan),
        "radialBasis" => Ok(Activation::RadialBasis),
        other => Err(PmmlError::Parse(format!(
            "unknown activationFunction {other:?}"
        ))),
    }
}

fn parse_normalization(text: &str) -> Result<Normalization, PmmlError> {
    match text {
        "none" => Ok(Normalization::None),
        "simplemax" => Ok(Normalization::Simplemax),
        "softmax" => Ok(Normalization::Softmax),
        other => Err(PmmlError::Parse(format!(
            "unknown normalizationMethod {other:?}"
        ))),
    }
}

pub(crate) fn score(
    pmml: &Pmml,
    model: &NeuralNetworkModel,
    env: &Env<'_>,
) -> Result<Vec<NeuralTarget>, PmmlError> {
    let mut inputs = Vec::with_capacity(model.inputs.len());
    for input in &model.inputs {
        let value = expr::eval(&input.expr, env, &pmml.functions, &[], 0)?;
        if value == Value::Missing {
            return Ok(null_targets(model));
        }
        inputs.push(expr::as_f64(&value, &input.id)?);
    }
    let raw = forward(model, &inputs)?;
    let method = model
        .layers
        .last()
        .map(|layer| layer.normalization)
        .ok_or_else(|| PmmlError::Score("neural network has no layer".into()))?;
    let mut groups: Vec<Group> = Vec::new();
    for output in &model.outputs {
        let activation = raw.get(output.neuron_index).copied().ok_or_else(|| {
            PmmlError::Score(format!(
                "neural output neuron {} has no value",
                output.neuron_id
            ))
        })?;
        let index = groups.iter().position(|group| group.name == output.target);
        let index = if let Some(index) = index {
            index
        } else {
            groups.push(Group {
                name: output.target.clone(),
                continuous: None,
                categories: Vec::new(),
            });
            groups.len().saturating_sub(1)
        };
        let group = groups
            .get_mut(index)
            .ok_or_else(|| PmmlError::Score("neural output group was not stored".into()))?;
        match &output.kind {
            NeuralOutputKind::Continuous(norm) => {
                if group.continuous.is_some() || !group.categories.is_empty() {
                    return Err(PmmlError::Score(format!(
                        "neural target {:?} mixes output kinds",
                        output.target
                    )));
                }
                group.continuous = Some((norm.clone(), activation));
            }
            NeuralOutputKind::Category(category) => {
                if group.continuous.is_some() {
                    return Err(PmmlError::Score(format!(
                        "neural target {:?} mixes output kinds",
                        output.target
                    )));
                }
                group.categories.push((category.clone(), activation));
            }
        }
    }

    let mut targets = Vec::with_capacity(groups.len());
    for group in groups {
        if let Some((norm, activation)) = group.continuous {
            if method != Normalization::None {
                return Err(PmmlError::Unsupported(
                    "output normalization with a continuous neural output is not supported".into(),
                ));
            }
            let value = denormalize(activation, &norm.points, norm.outliers, norm.map_missing_to)?;
            targets.push(NeuralTarget {
                name: group.name,
                value,
                probabilities: Vec::new(),
            });
            continue;
        }
        let logits: Vec<f64> = group.categories.iter().map(|(_, logit)| *logit).collect();
        let probabilities = normalize_output_group(method, &logits)?;
        let mut best = 0usize;
        for (index, probability) in probabilities.iter().enumerate().skip(1) {
            if let Some(current) = probabilities.get(best)
                && probability > current
            {
                best = index;
            }
        }
        let winner = group
            .categories
            .get(best)
            .map(|(category, _)| category.clone())
            .ok_or_else(|| PmmlError::Score("neural output group is empty".into()))?;
        let pairs: Vec<(String, f64)> = group
            .categories
            .into_iter()
            .zip(probabilities)
            .map(|((category, _), probability)| (category, probability))
            .collect();
        let pairs = order_categories(pmml, &group.name, pairs)?;
        targets.push(NeuralTarget {
            name: group.name,
            value: Value::Text(winner),
            probabilities: pairs,
        });
    }
    Ok(targets)
}

struct Group {
    name: String,
    continuous: Option<(ContinuousNorm, f64)>,
    categories: Vec<(String, f64)>,
}

fn null_targets(model: &NeuralNetworkModel) -> Vec<NeuralTarget> {
    let mut targets = Vec::new();
    for output in &model.outputs {
        if targets
            .iter()
            .any(|target: &NeuralTarget| target.name == output.target)
        {
            continue;
        }
        targets.push(NeuralTarget {
            name: output.target.clone(),
            value: Value::Missing,
            probabilities: Vec::new(),
        });
    }
    targets
}

fn order_categories(
    pmml: &Pmml,
    target: &str,
    probabilities: Vec<(String, f64)>,
) -> Result<Vec<(String, f64)>, PmmlError> {
    let Some(field) = pmml
        .data_dictionary
        .iter()
        .find(|field| field.name == target)
    else {
        return Ok(probabilities);
    };
    if field.values.is_empty() {
        return Ok(probabilities);
    }
    let mut ordered = Vec::with_capacity(field.values.len());
    for value in &field.values {
        let probability = probabilities
            .iter()
            .find(|(category, _)| category == value)
            .map(|(_, probability)| *probability)
            .ok_or_else(|| {
                PmmlError::Score(format!(
                    "neural network has no output for {target:?} category {value:?}"
                ))
            })?;
        ordered.push((value.clone(), probability));
    }
    for (category, probability) in probabilities {
        if !field.values.iter().any(|value| value == &category) {
            ordered.push((category, probability));
        }
    }
    Ok(ordered)
}

/// Hidden layers are normalized after activation. The output layer is not: its
/// categorical groups are normalized later (`nn_fixed_a` versus `nn_default_a`).
fn forward(model: &NeuralNetworkModel, inputs: &[f64]) -> Result<Vec<f64>, PmmlError> {
    if inputs.len() != model.inputs.len() {
        return Err(PmmlError::Score(
            "neural input count does not match the network".into(),
        ));
    }
    let mut values: HashMap<&str, f64> = HashMap::new();
    for (input, value) in model.inputs.iter().zip(inputs) {
        values.insert(input.id.as_str(), *value);
    }
    let last = model.layers.len().saturating_sub(1);
    let mut output = Vec::new();
    for (index, layer) in model.layers.iter().enumerate() {
        let mut activated = Vec::with_capacity(layer.neurons.len());
        for neuron in &layer.neurons {
            let raw = if neuron.activation == Activation::RadialBasis {
                radial_basis(neuron, &values)?
            } else {
                let mut sum = neuron.bias;
                for (from, weight) in &neuron.connections {
                    let incoming = values.get(from.as_str()).copied().ok_or_else(|| {
                        PmmlError::Score(format!(
                            "neuron {} connects from unknown id {from:?}",
                            neuron.id
                        ))
                    })?;
                    sum += weight * incoming;
                }
                if !sum.is_finite() {
                    return Err(PmmlError::Score(format!(
                        "neuron {} combination is not finite",
                        neuron.id
                    )));
                }
                activate(neuron.activation, sum)?
            };
            activated.push(raw);
        }
        let normalized = if index == last {
            activated.clone()
        } else {
            normalize_layer(layer.normalization, &activated)?
        };
        for (neuron, value) in layer.neurons.iter().zip(&normalized) {
            values.insert(neuron.id.as_str(), *value);
        }
        if index == last {
            output = activated;
        }
    }
    if output.is_empty() {
        return Err(PmmlError::Score(
            "neural network has no output layer".into(),
        ));
    }
    Ok(output)
}

fn radial_basis(neuron: &Neuron, values: &HashMap<&str, f64>) -> Result<f64, PmmlError> {
    let width = neuron.width.ok_or_else(|| {
        PmmlError::Score(format!("radialBasis neuron {} has no width", neuron.id))
    })?;
    if width == 0.0 {
        return Err(PmmlError::Score(format!(
            "radialBasis neuron {} has width 0",
            neuron.id
        )));
    }
    let mut distance = 0.0;
    for (from, center) in &neuron.connections {
        let incoming = values.get(from.as_str()).copied().ok_or_else(|| {
            PmmlError::Score(format!(
                "neuron {} connects from unknown id {from:?}",
                neuron.id
            ))
        })?;
        let delta = incoming - center;
        distance += delta * delta;
    }
    if !distance.is_finite() {
        return Err(PmmlError::Score(format!(
            "radialBasis neuron {} distance is not finite",
            neuron.id
        )));
    }
    let value = neuron.altitude * (-distance / (2.0 * width * width)).exp();
    finite(value, "radialBasis activation")
}

fn activate(kind: Activation, z: f64) -> Result<f64, PmmlError> {
    let value = match kind {
        Activation::Threshold => {
            if z > 0.0 {
                1.0
            } else {
                0.0
            }
        }
        Activation::Logistic => {
            if z >= 0.0 {
                let exp = (-z).exp();
                1.0 / (1.0 + exp)
            } else {
                let exp = z.exp();
                exp / (1.0 + exp)
            }
        }
        Activation::Tanh => z.tanh(),
        Activation::Identity => z,
        Activation::Exponential => z.exp(),
        Activation::Reciprocal => {
            if z == 0.0 {
                return Err(PmmlError::Score("reciprocal activation of zero".into()));
            }
            1.0 / z
        }
        Activation::Square => z * z,
        Activation::Gauss => (-z * z).exp(),
        Activation::Sine => z.sin(),
        Activation::Cosine => z.cos(),
        Activation::Elliott => z / (1.0 + z.abs()),
        Activation::Arctan => z.atan(),
        Activation::RadialBasis => {
            return Err(PmmlError::Score(
                "radialBasis is not a scalar activation".into(),
            ));
        }
    };
    finite(value, "neural activation")
}

fn normalize_layer(method: Normalization, values: &[f64]) -> Result<Vec<f64>, PmmlError> {
    match method {
        Normalization::None => Ok(values.to_vec()),
        Normalization::Simplemax => simplemax(values),
        Normalization::Softmax => softmax(values),
    }
}

/// Categorical output group. Absent normalization is rectified simplemax (`nn_fixed_a`),
/// not the hidden-layer meaning of `none`.
fn normalize_output_group(method: Normalization, values: &[f64]) -> Result<Vec<f64>, PmmlError> {
    match method {
        Normalization::None => rectified_simplemax(values),
        Normalization::Simplemax => simplemax(values),
        Normalization::Softmax => softmax(values),
    }
}

fn rectified_simplemax(values: &[f64]) -> Result<Vec<f64>, PmmlError> {
    let Some(mut min) = values.first().copied() else {
        return Err(PmmlError::Score("neural output group is empty".into()));
    };
    for value in values.iter().skip(1) {
        if *value < min {
            min = *value;
        }
    }
    if !min.is_finite() {
        return Err(PmmlError::Score("neural output is not finite".into()));
    }
    let shift = min.min(0.0);
    let shifted: Vec<f64> = values.iter().map(|value| value - shift).collect();
    simplemax(&shifted)
}

fn simplemax(values: &[f64]) -> Result<Vec<f64>, PmmlError> {
    if values.is_empty() {
        return Err(PmmlError::Score(
            "neural normalization group is empty".into(),
        ));
    }
    let sum: f64 = values.iter().sum();
    if sum == 0.0 || !sum.is_finite() {
        return Err(PmmlError::Score(
            "neural simplemax sum is zero or not finite".into(),
        ));
    }
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        out.push(finite(value / sum, "simplemax")?);
    }
    Ok(out)
}

fn softmax(values: &[f64]) -> Result<Vec<f64>, PmmlError> {
    let Some(mut max) = values.first().copied() else {
        return Err(PmmlError::Score("neural softmax group is empty".into()));
    };
    for value in values.iter().skip(1) {
        if *value > max {
            max = *value;
        }
    }
    if !max.is_finite() {
        return Err(PmmlError::Score(
            "neural softmax input is not finite".into(),
        ));
    }
    let mut exps = Vec::with_capacity(values.len());
    let mut sum = 0.0;
    for value in values {
        let exp = (value - max).exp();
        sum += exp;
        exps.push(exp);
    }
    if sum == 0.0 || !sum.is_finite() {
        return Err(PmmlError::Score("neural softmax sum is not finite".into()));
    }
    let mut out = Vec::with_capacity(exps.len());
    for exp in exps {
        out.push(finite(exp / sum, "softmax")?);
    }
    Ok(out)
}

fn finite(value: f64, what: &str) -> Result<f64, PmmlError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(PmmlError::Score(format!("{what} is not finite")))
    }
}

pub(crate) fn input_exprs(model: &NeuralNetworkModel) -> impl Iterator<Item = &Expr> {
    model.inputs.iter().map(|input| &input.expr)
}
