//! Modeler's Self-Learning Response Model nugget: one `NaiveBayesModel` per product
//! (`algorithmName="SelfLearning"`), written by the "SPSS Self Learning Component"
//! (`corpus/ibm-demos/pm_selflearn.str`).
//!
//! Each model is a Naive Bayes classifier of the response (1 or 0) for one product: the training
//! counts of the response (`BayesOutput`), and for every predictor either the cut points of its
//! `Discretize` (a continuous predictor) or its categories, with the response counts of each cell
//! (`PairCounts`). A cell the training data never occupied has no `PairCounts` entry.
//!
//! The probability of a response for a record is the Naive Bayes posterior with the Dirichlet
//! smoothing `f = 1/N` of the Algorithms Guide (Self-Learning Response Model): the prior is
//! `(n_y + f) / (N + 2 f)` and each predictor with `M` occupied cells gives the likelihood
//! `(n_{y,x} + f) / (n_y,field + M f)`; a missing predictor, or a value in a cell without counts,
//! gives no factor. The offer score shrinks the probability towards 0.5 with the model's size
//! (`W_emp = 500` reliability cases) when the model reliability is on.

use crate::parse::parse_f64;
use crate::{MiningField, MiningFunction, ModelHeader, PmmlError, Usage, Value};

const MAX_PRODUCTS: usize = 1_000;
const MAX_INPUTS: usize = 10_000;
const MAX_CELLS: usize = 1_000_000;
/// Cases of the reliability weight (`W_emp`).
const RELIABILITY_CASES: f64 = 500.0;

/// An SLRM nugget: one model per product.
#[derive(Debug, Clone, PartialEq)]
pub struct SlrmModel {
    pub header: ModelHeader,
    /// The field holding the product (the self-learning target).
    pub target: String,
    /// The products, in the order Modeler lists them.
    pub products: Vec<SlrmProduct>,
}

/// The Naive Bayes model of one product.
#[derive(Debug, Clone, PartialEq)]
pub struct SlrmProduct {
    pub name: String,
    /// The probability below which a response is not offered (`1/N`).
    pub threshold: f64,
    /// `[count of response 0, count of response 1]`.
    pub target_counts: [u64; 2],
    pub inputs: Vec<SlrmInput>,
}

/// One predictor of a product.
#[derive(Debug, Clone, PartialEq)]
pub struct SlrmInput {
    pub field: String,
    /// The cut points of a continuous predictor, ascending: `k` cuts give `k + 1` bins, bin `b`
    /// holding the values above `b` cuts and not above the next. `None` for a categorical one.
    pub cuts: Option<Vec<f64>>,
    pub cells: Vec<SlrmCell>,
}

/// The response counts of one bin (its index) or category (its label).
#[derive(Debug, Clone, PartialEq)]
pub struct SlrmCell {
    pub value: String,
    pub n0: u64,
    pub n1: u64,
}

/// The score of one product for a record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlrmScore {
    pub product: usize,
    /// `P(response = 1)`.
    pub probability: f64,
    /// The offer score: the probability shrunk by the model's reliability.
    pub score: f64,
}

impl SlrmModel {
    /// A model from its products.
    pub fn new(target: String, products: Vec<SlrmProduct>) -> Self {
        let mut names: Vec<&str> = Vec::new();
        for product in &products {
            for input in &product.inputs {
                if !names.contains(&input.field.as_str()) {
                    names.push(&input.field);
                }
            }
        }
        let header = ModelHeader {
            name: None,
            function: MiningFunction::Classification,
            algorithm: Some("SelfLearning".into()),
            mining_schema: names
                .into_iter()
                .map(|name| MiningField {
                    name: name.to_string(),
                    usage: Usage::Active,
                    missing_replacement: None,
                    outliers: crate::Outliers::AsIs,
                    low_value: None,
                    high_value: None,
                    invalid_treatment: crate::InvalidTreatment::AsIs,
                    direction: None,
                })
                .collect(),
            output: Vec::new(),
            local_transformations: Vec::new(),
        };
        Self {
            header,
            target,
            products,
        }
    }

    /// The fields the model reads.
    pub fn input_fields(&self) -> impl Iterator<Item = &str> {
        self.header
            .mining_schema
            .iter()
            .filter(|f| f.usage == Usage::Active)
            .map(|f| f.name.as_str())
    }

    /// `P(response = 1)` of every product and its offer score, for the record `value` reads.
    /// `reliability` shrinks the probability towards 0.5 with the number of training cases.
    pub fn scores<'a>(
        &self,
        value: impl Fn(&str) -> Option<&'a Value>,
        reliability: bool,
    ) -> Vec<SlrmScore> {
        let w_emp = if reliability { RELIABILITY_CASES } else { 0.0 };
        self.products
            .iter()
            .enumerate()
            .map(|(product, p)| {
                let probability = p.probability(&value);
                let n = (p.target_counts[0] + p.target_counts[1]) as f64;
                let weight = n + w_emp;
                let score = if weight == 0.0 {
                    0.5
                } else {
                    n / weight * probability + w_emp / weight * 0.5
                };
                SlrmScore {
                    product,
                    probability,
                    score,
                }
            })
            .collect()
    }
}

impl SlrmProduct {
    /// The Naive Bayes `P(response = 1)` for the record `value` reads.
    pub fn probability<'a>(&self, value: &impl Fn(&str) -> Option<&'a Value>) -> f64 {
        let [n0, n1] = self.target_counts;
        let total = (n0 + n1) as f64;
        if total == 0.0 {
            return 0.5;
        }
        let f = 1.0 / total;
        let mut log1 = ((n1 as f64 + f) / (total + 2.0 * f)).ln();
        let mut log0 = ((n0 as f64 + f) / (total + 2.0 * f)).ln();
        for input in &self.inputs {
            let Some(v) = value(&input.field) else {
                continue;
            };
            let key = match (&input.cuts, v) {
                (_, Value::Missing) => continue,
                (Some(cuts), v) => {
                    let x = match v {
                        Value::Number(x) => *x,
                        Value::Text(t) => match t.trim().parse::<f64>() {
                            Ok(x) => x,
                            Err(_) => continue,
                        },
                        Value::Missing => continue,
                    };
                    cuts.iter().filter(|c| **c < x).count().to_string()
                }
                (None, Value::Text(t)) => t.clone(),
                (None, Value::Number(x)) => format!("{x}"),
            };
            let Some(cell) = input
                .cells
                .iter()
                .find(|c| c.value == key || same_number(&c.value, &key))
            else {
                continue;
            };
            let m = input.cells.len() as f64;
            let field1: u64 = input.cells.iter().map(|c| c.n1).sum();
            let field0: u64 = input.cells.iter().map(|c| c.n0).sum();
            log1 += ((cell.n1 as f64 + f) / (field1 as f64 + m * f)).ln();
            log0 += ((cell.n0 as f64 + f) / (field0 as f64 + m * f)).ln();
        }
        // P1 = e^log1 / (e^log1 + e^log0), without overflow
        let high = log1.max(log0);
        let (a, b) = ((log1 - high).exp(), (log0 - high).exp());
        a / (a + b)
    }
}

fn same_number(a: &str, b: &str) -> bool {
    matches!((a.trim().parse::<f64>(), b.trim().parse::<f64>()), (Ok(x), Ok(y)) if x == y)
}

/// Whether a `NaiveBayesModel` element is one of Modeler's self-learning models.
pub(crate) fn is_self_learning(node: roxmltree::Node<'_, '_>) -> bool {
    node.attribute("algorithmName") == Some("SelfLearning")
}

pub(crate) fn parse_product(node: roxmltree::Node<'_, '_>) -> Result<SlrmProduct, PmmlError> {
    let name = node
        .attribute("modelName")
        .ok_or_else(|| PmmlError::Parse("a self-learning model has no modelName".into()))?
        .to_string();
    let threshold = node
        .attribute("threshold")
        .map(|v| parse_f64(v, "SLRM threshold"))
        .transpose()?
        .unwrap_or(0.0);
    let counts_of = |parent: roxmltree::Node<'_, '_>| -> Result<[u64; 2], PmmlError> {
        let mut out = [0u64; 2];
        for c in parent
            .descendants()
            .filter(|n| n.has_tag_name("TargetValueCount"))
        {
            let count = c
                .attribute("count")
                .and_then(|v| v.trim().parse::<f64>().ok())
                .filter(|v| *v >= 0.0 && v.fract() == 0.0)
                .ok_or_else(|| PmmlError::Parse("a count is not a whole number".into()))?
                as u64;
            match c.attribute("value").map(str::trim) {
                Some("0") => out[0] = count,
                Some("1") => out[1] = count,
                other => {
                    return Err(PmmlError::Parse(format!(
                        "a self-learning count for the response value {other:?}"
                    )));
                }
            }
        }
        Ok(out)
    };
    let output = node
        .children()
        .find(|n| n.has_tag_name("BayesOutput"))
        .ok_or_else(|| PmmlError::Parse("a self-learning model has no BayesOutput".into()))?;
    let target_counts = counts_of(output)?;
    let mut inputs = Vec::new();
    let mut cells_total = 0usize;
    if let Some(list) = node.children().find(|n| n.has_tag_name("BayesInputs")) {
        for input in list.children().filter(|n| n.has_tag_name("BayesInput")) {
            if inputs.len() >= MAX_INPUTS {
                return Err(PmmlError::Parse("too many self-learning predictors".into()));
            }
            let field = input
                .attribute("fieldName")
                .ok_or_else(|| PmmlError::Parse("a BayesInput has no fieldName".into()))?
                .to_string();
            let cuts = input
                .descendants()
                .find(|n| n.has_tag_name("Discretize"))
                .map(|d| -> Result<Vec<f64>, PmmlError> {
                    let mut cuts = Vec::new();
                    for bin in d.children().filter(|n| n.has_tag_name("DiscretizeBin")) {
                        if let Some(interval) = bin.children().find(|n| n.has_tag_name("Interval"))
                            && let Some(right) = interval.attribute("rightMargin")
                        {
                            cuts.push(parse_f64(right, "SLRM cut point")?);
                        }
                    }
                    if cuts.windows(2).any(|w| w[0] >= w[1]) {
                        return Err(PmmlError::Parse("SLRM cut points are not ascending".into()));
                    }
                    Ok(cuts)
                })
                .transpose()?;
            let mut cells = Vec::new();
            for pair in input.children().filter(|n| n.has_tag_name("PairCounts")) {
                cells_total += 1;
                if cells_total > MAX_CELLS {
                    return Err(PmmlError::Parse("too many self-learning cells".into()));
                }
                let counts = counts_of(pair)?;
                cells.push(SlrmCell {
                    value: pair
                        .attribute("value")
                        .ok_or_else(|| PmmlError::Parse("PairCounts has no value".into()))?
                        .to_string(),
                    n0: counts[0],
                    n1: counts[1],
                });
            }
            inputs.push(SlrmInput { field, cuts, cells });
        }
    }
    Ok(SlrmProduct {
        name,
        threshold,
        target_counts,
        inputs,
    })
}

pub(crate) fn finish(
    target: Option<String>,
    products: Vec<SlrmProduct>,
) -> Result<SlrmModel, PmmlError> {
    if products.is_empty() || products.len() > MAX_PRODUCTS {
        return Err(PmmlError::Parse(
            "a self-learning model has no or too many products".into(),
        ));
    }
    let target = target.ok_or_else(|| {
        PmmlError::Parse(
            "a self-learning model has no DataField marked isSelfLearningTarget".into(),
        )
    })?;
    Ok(SlrmModel::new(target, products))
}
