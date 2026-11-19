//! `SupportVectorMachineModel`: the support-vector representation Modeler writes for its SVM
//! node (binary classification, `svmRepresentation="SupportVectors"`).
//!
//! The decision value is `f(x) = sum_i c_i K(sv_i, x) + b`, where the `c_i` are the
//! `Coefficients`, `b` is their `absoluteValue`, and `x` is the vector of `VectorFields` (derived
//! fields such as the scaled covariates and the indicator columns of a nominal field). Only binary
//! models are scored; several machines are rejected as unsupported.
//!
//! Which sign of `f` means the machine's `targetCategory`, and how the Platt parameters turn `f`
//! into a probability, are set by [`SvmModel::target_when_negative`] and
//! [`SvmModel::target_probability`]: see the notes there.

use crate::parse::{parse_f64, parse_model_header, parse_value_str, req_attr};
use crate::{ModelHeader, PmmlError, Prediction, Usage, Value};

const MAX_VECTORS: usize = 1_000_000;
const MAX_FIELDS: usize = 100_000;
const MAX_CELLS: usize = 200_000_000;

/// The kernel of the model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kernel {
    Linear,
    Polynomial { gamma: f64, coef0: f64, degree: f64 },
    RadialBasis { gamma: f64 },
    Sigmoid { gamma: f64, coef0: f64 },
}

impl Kernel {
    /// `K(a, b)`.
    pub fn value(self, a: &[f64], b: &[f64]) -> f64 {
        let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        match self {
            Kernel::Linear => dot,
            Kernel::Polynomial {
                gamma,
                coef0,
                degree,
            } => (gamma * dot + coef0).powf(degree),
            Kernel::RadialBasis { gamma } => {
                let distance: f64 = a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum();
                (-gamma * distance).exp()
            }
            Kernel::Sigmoid { gamma, coef0 } => (gamma * dot + coef0).tanh(),
        }
    }
}

/// A binary SVM classifier.
#[derive(Debug, Clone, PartialEq)]
pub struct SvmModel {
    pub header: ModelHeader,
    /// The predicted field.
    pub target: String,
    pub kernel: Kernel,
    /// The fields of a support vector (names of fields or derived fields).
    pub fields: Vec<String>,
    /// The support vectors, one row of `fields.len()` values each.
    pub vectors: Vec<Vec<f64>>,
    /// The coefficient of each support vector.
    pub coefficients: Vec<f64>,
    /// The constant `b` added to the sum.
    pub constant: f64,
    /// The category the machine is for (`targetCategory`).
    pub target_category: String,
    /// The other category (`alternateBinaryTargetCategory`).
    pub alternate_category: String,
    /// The Platt scaling parameters `(A, B)` of the machine, when the model has them.
    pub platt: Option<(f64, f64)>,
}

impl SvmModel {
    /// The fields the model needs.
    pub(crate) fn input_fields(&self) -> impl Iterator<Item = &str> {
        self.fields.iter().map(String::as_str)
    }

    /// `f(x)` for a vector of `fields.len()` values.
    pub fn decision(&self, x: &[f64]) -> f64 {
        self.vectors
            .iter()
            .zip(&self.coefficients)
            .map(|(sv, c)| c * self.kernel.value(sv, x))
            .sum::<f64>()
            + self.constant
    }

    /// Whether a negative decision value means the machine's target category.
    ///
    /// PMML: a value below the threshold (0) means `targetCategory`. Modeler's coefficients are
    /// the signed duals with the SVM's positive class being the model's `Response` category
    /// (`ResponseCategory`): a positive value means the response, so with `targetCategory` equal
    /// to the response category the target is the positive side.
    pub fn target_when_negative(&self) -> bool {
        false
    }

    /// The probability of the target category for a decision value: Platt scaling
    /// `1 / (1 + exp(A f + B))` with the machine's parameters; `None` without them.
    pub fn target_probability(&self, f: f64) -> Option<f64> {
        self.platt
            .map(|(a, b)| 1.0 / (1.0 + (a * f + b).exp()))
            .filter(|p| p.is_finite())
    }
}

pub(crate) fn score(
    model: &SvmModel,
    env: &crate::score::Env<'_>,
) -> Result<Prediction, PmmlError> {
    let mut x = Vec::with_capacity(model.fields.len());
    for field in &model.fields {
        match env.get(field) {
            Some(Value::Number(n)) if n.is_finite() => x.push(*n),
            Some(Value::Text(text)) => match text.trim().parse::<f64>() {
                Ok(n) if n.is_finite() => x.push(n),
                _ => return Ok(empty()),
            },
            _ => return Ok(empty()),
        }
    }
    let f = model.decision(&x);
    let positive = f >= 0.0;
    let is_target = positive != model.target_when_negative();
    let value = if is_target {
        &model.target_category
    } else {
        &model.alternate_category
    };
    let mut probabilities = Vec::new();
    let mut confidence = None;
    if let Some(p) = model.target_probability(f) {
        // the response last: a raw propensity is the second probability
        probabilities = vec![
            (model.alternate_category.clone(), 1.0 - p),
            (model.target_category.clone(), p),
        ];
        confidence = Some(if is_target { p } else { 1.0 - p });
    }
    Ok(Prediction {
        value: parse_value_str(value),
        confidence,
        probabilities,
        entity_id: None,
        affinity: None,
    })
}

fn empty() -> Prediction {
    Prediction {
        value: Value::Missing,
        confidence: None,
        probabilities: Vec::new(),
        entity_id: None,
        affinity: None,
    }
}

pub(crate) fn parse(node: roxmltree::Node<'_, '_>) -> Result<SvmModel, PmmlError> {
    let header = parse_model_header(node)?;
    let target = header
        .mining_schema
        .iter()
        .find(|f| f.usage == Usage::Target)
        .map(|f| f.name.clone())
        .ok_or_else(|| {
            PmmlError::Parse("SupportVectorMachineModel has no predicted field".into())
        })?;
    let child = |name: &str| node.children().find(|c| c.has_tag_name(name));
    let kernel = parse_kernel(node)?;
    let dictionary = child("VectorDictionary").ok_or_else(|| {
        PmmlError::Parse("SupportVectorMachineModel has no VectorDictionary".into())
    })?;
    let fields: Vec<String> = dictionary
        .children()
        .find(|c| c.has_tag_name("VectorFields"))
        .map(|list| {
            list.children()
                .filter(|c| c.has_tag_name("FieldRef"))
                .filter_map(|c| c.attribute("field").map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if fields.is_empty() || fields.len() > MAX_FIELDS {
        return Err(PmmlError::Parse(
            "SVM vectors have no or too many fields".into(),
        ));
    }
    let mut ids: Vec<(String, Vec<f64>)> = Vec::new();
    for instance in dictionary
        .children()
        .filter(|c| c.has_tag_name("VectorInstance"))
    {
        if ids.len() >= MAX_VECTORS || (ids.len() + 1).saturating_mul(fields.len()) > MAX_CELLS {
            return Err(PmmlError::Parse("SVM has too many support vectors".into()));
        }
        let id = req_attr(instance, "id")?.to_string();
        let mut row = vec![0.0; fields.len()];
        if let Some(sparse) = instance
            .children()
            .find(|c| c.has_tag_name("REAL-SparseArray"))
        {
            let indices = list_of(sparse, "Indices", |t| {
                t.parse::<usize>()
                    .map_err(|_| PmmlError::Parse(format!("bad SVM index {t:?}")))
            })?;
            let entries = list_of(sparse, "REAL-Entries", |t| parse_f64(t, "SVM entry"))?;
            if indices.len() != entries.len() {
                return Err(PmmlError::Parse("SVM sparse array lengths differ".into()));
            }
            for (index, value) in indices.into_iter().zip(entries) {
                // PMML sparse indices are 1-based
                let slot = index
                    .checked_sub(1)
                    .and_then(|i| row.get_mut(i))
                    .ok_or_else(|| PmmlError::Parse("SVM sparse index out of range".into()))?;
                *slot = value;
            }
        } else if let Some(dense) = instance.children().find(|c| c.has_tag_name("REAL-Array")) {
            let values = dense
                .text()
                .unwrap_or_default()
                .split_whitespace()
                .map(|t| parse_f64(t, "SVM entry"))
                .collect::<Result<Vec<_>, _>>()?;
            if values.len() != fields.len() {
                return Err(PmmlError::Parse("SVM dense array length differs".into()));
            }
            row = values;
        }
        ids.push((id, row));
    }

    let mut machines = node
        .children()
        .filter(|c| c.has_tag_name("SupportVectorMachine"));
    let (Some(machine), None) = (machines.next(), machines.next()) else {
        return Err(PmmlError::Unsupported(
            "SupportVectorMachineModel with other than one machine".into(),
        ));
    };
    let target_category = req_attr(machine, "targetCategory")?.to_string();
    let alternate_category = node
        .attribute("alternateBinaryTargetCategory")
        .ok_or_else(|| PmmlError::Unsupported("SVM without alternateBinaryTargetCategory".into()))?
        .to_string();
    let coefficients_node = machine
        .children()
        .find(|c| c.has_tag_name("Coefficients"))
        .ok_or_else(|| PmmlError::Parse("SVM machine has no Coefficients".into()))?;
    let constant = coefficients_node
        .attribute("absoluteValue")
        .map(|v| parse_f64(v, "SVM absoluteValue"))
        .transpose()?
        .unwrap_or(0.0);
    let coefficients = coefficients_node
        .children()
        .filter(|c| c.has_tag_name("Coefficient"))
        .map(|c| parse_f64(req_attr(c, "value")?, "SVM coefficient"))
        .collect::<Result<Vec<_>, _>>()?;
    let support = machine
        .children()
        .find(|c| c.has_tag_name("SupportVectors"))
        .ok_or_else(|| PmmlError::Parse("SVM machine has no SupportVectors".into()))?;
    let mut vectors = Vec::new();
    for sv in support
        .children()
        .filter(|c| c.has_tag_name("SupportVector"))
    {
        let id = req_attr(sv, "vectorId")?;
        let (_, row) = ids.iter().find(|(i, _)| i == id).ok_or_else(|| {
            PmmlError::Parse(format!("SVM vector {id:?} is not in the dictionary"))
        })?;
        vectors.push(row.clone());
    }
    if vectors.len() != coefficients.len() {
        return Err(PmmlError::Parse(
            "SVM has a different number of vectors and coefficients".into(),
        ));
    }
    let platt = machine
        .descendants()
        .find(|c| c.has_tag_name("ProbabilityParameter"))
        .map(|p| -> Result<(f64, f64), PmmlError> {
            Ok((
                parse_f64(req_attr(p, "paramA")?, "SVM paramA")?,
                parse_f64(req_attr(p, "paramB")?, "SVM paramB")?,
            ))
        })
        .transpose()?;
    Ok(SvmModel {
        header,
        target,
        kernel,
        fields,
        vectors,
        coefficients,
        constant,
        target_category,
        alternate_category,
        platt,
    })
}

fn list_of<T>(
    parent: roxmltree::Node<'_, '_>,
    name: &str,
    parse: impl Fn(&str) -> Result<T, PmmlError>,
) -> Result<Vec<T>, PmmlError> {
    parent
        .children()
        .find(|c| c.has_tag_name(name))
        .and_then(|c| c.text())
        .unwrap_or_default()
        .split_whitespace()
        .map(parse)
        .collect()
}

fn parse_kernel(node: roxmltree::Node<'_, '_>) -> Result<Kernel, PmmlError> {
    let number = |k: roxmltree::Node<'_, '_>, name: &str, default: f64| -> Result<f64, PmmlError> {
        k.attribute(name)
            .map(|v| parse_f64(v, name))
            .transpose()
            .map(|v| v.unwrap_or(default))
    };
    for k in node.children().filter(roxmltree::Node::is_element) {
        match k.tag_name().name() {
            "LinearKernelType" => return Ok(Kernel::Linear),
            "PolynomialKernelType" => {
                return Ok(Kernel::Polynomial {
                    gamma: number(k, "gamma", 1.0)?,
                    coef0: number(k, "coef0", 1.0)?,
                    degree: number(k, "degree", 1.0)?,
                });
            }
            "RadialBasisKernelType" => {
                return Ok(Kernel::RadialBasis {
                    gamma: number(k, "gamma", 1.0)?,
                });
            }
            "SigmoidKernelType" => {
                return Ok(Kernel::Sigmoid {
                    gamma: number(k, "gamma", 1.0)?,
                    coef0: number(k, "coef0", 1.0)?,
                });
            }
            _ => {}
        }
    }
    Err(PmmlError::Parse(
        "SupportVectorMachineModel has no kernel".into(),
    ))
}
