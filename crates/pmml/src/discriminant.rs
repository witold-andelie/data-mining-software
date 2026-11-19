//! Modeler's Discriminant nugget: the model state it keeps in the `model` container
//! (`<spss-ml><model><discriminant-model>`, `corpus/ibm-demos/telco_custcat_discriminant.str`).
//!
//! The state is the canonical discriminant functions (coefficients per predictor and function, and
//! a constant per function), the functions at the group centroids, and the prior of each group.
//! A record's scores on the functions are `a_k + sum_i b_ik x_i`; its squared distance to group `j`
//! is the sum over the functions of `(score_k - centroid_jk)^2` (the functions are scaled so that
//! the pooled within-groups covariance is the identity), and its posterior probability for group
//! `j` is proportional to `prior_j * exp(-d_j^2 / 2)`. The prediction is the group with the largest
//! posterior. A record with a missing predictor has no prediction.

use crate::{
    InvalidTreatment, MiningField, MiningFunction, ModelHeader, Outliers, PmmlError, Usage,
};

const MAX_PREDICTORS: usize = 10_000;
const MAX_GROUPS: usize = 10_000;

/// A discriminant model with the pooled within-groups covariance.
#[derive(Debug, Clone, PartialEq)]
pub struct DiscriminantModel {
    pub header: ModelHeader,
    /// The group field.
    pub target: String,
    /// The predictors, in the order of the coefficient rows.
    pub predictors: Vec<String>,
    /// The group values, as written in the dictionary (`1`, `2`, ...).
    pub groups: Vec<String>,
    /// `coefficients[i][k]`: predictor `i` on function `k`.
    pub coefficients: Vec<Vec<f64>>,
    /// The constant of each function.
    pub constants: Vec<f64>,
    /// `centroids[j][k]`: group `j` on function `k`.
    pub centroids: Vec<Vec<f64>>,
    /// The prior probability of each group.
    pub priors: Vec<f64>,
}

/// The scores of one record.
#[derive(Debug, Clone, PartialEq)]
pub struct DiscriminantScore {
    /// Index of the predicted group in [`DiscriminantModel::groups`].
    pub predicted: usize,
    /// The posterior probability of every group.
    pub posteriors: Vec<f64>,
}

impl DiscriminantModel {
    /// A model from its parts; the header lists the predictors as active fields.
    pub fn new(
        target: String,
        predictors: Vec<String>,
        groups: Vec<String>,
        coefficients: Vec<Vec<f64>>,
        constants: Vec<f64>,
        centroids: Vec<Vec<f64>>,
        priors: Vec<f64>,
    ) -> Result<Self, PmmlError> {
        let (p, g, m) = (predictors.len(), groups.len(), constants.len());
        if p == 0 || p > MAX_PREDICTORS || !(2..=MAX_GROUPS).contains(&g) || m == 0 {
            return Err(PmmlError::Parse(
                "a discriminant model needs predictors, two groups and a function".into(),
            ));
        }
        if coefficients.len() != p
            || coefficients.iter().any(|row| row.len() != m)
            || centroids.len() != g
            || centroids.iter().any(|row| row.len() != m)
            || priors.len() != g
        {
            return Err(PmmlError::Parse(
                "the discriminant model's tables do not fit its predictors, groups and functions"
                    .into(),
            ));
        }
        let mut mining_schema: Vec<MiningField> = predictors
            .iter()
            .map(|name| mining_field(name, Usage::Active))
            .collect();
        mining_schema.push(mining_field(&target, Usage::Target));
        Ok(Self {
            header: ModelHeader {
                name: None,
                function: MiningFunction::Classification,
                algorithm: Some("Discriminant".into()),
                mining_schema,
                output: Vec::new(),
                local_transformations: Vec::new(),
            },
            target,
            predictors,
            groups,
            coefficients,
            constants,
            centroids,
            priors,
        })
    }

    /// The model Modeler keeps in a nugget's `model` container.
    pub fn from_spssml(xml: &str) -> Result<Self, PmmlError> {
        let document = roxmltree::Document::parse(xml)
            .map_err(|error| PmmlError::Parse(format!("discriminant model: {error}")))?;
        let model = document
            .descendants()
            .find(|n| n.has_tag_name("discriminant-model"))
            .ok_or_else(|| PmmlError::Parse("no discriminant-model element".into()))?;
        let target = model
            .attribute("target-variable-name")
            .ok_or_else(|| PmmlError::Parse("the discriminant model names no target".into()))?
            .to_string();
        let number = |node: roxmltree::Node<'_, '_>| -> Result<f64, PmmlError> {
            node.attribute("value")
                .and_then(|v| v.trim().parse::<f64>().ok())
                .filter(|v| v.is_finite())
                .ok_or_else(|| PmmlError::Parse("a discriminant cell has no number".into()))
        };
        let index = |node: roxmltree::Node<'_, '_>, name: &str| -> Result<usize, PmmlError> {
            node.attribute(name)
                .and_then(|v| v.trim().parse::<usize>().ok())
                .filter(|v| *v >= 1 && *v <= MAX_PREDICTORS)
                .ok_or_else(|| PmmlError::Parse(format!("a discriminant cell has no {name}")))
        };
        // function coefficients: a row per predictor, and the cells without a row are the constants
        let mut predictors: Vec<String> = Vec::new();
        let mut cells: Vec<(String, usize, f64)> = Vec::new();
        let mut constants: Vec<(usize, f64)> = Vec::new();
        for cell in model
            .descendants()
            .filter(|n| n.has_tag_name("function-coefficients"))
            .flat_map(|f| f.children().filter(|n| n.has_tag_name("cell")))
        {
            let column = index(cell, "column")?;
            match cell.attribute("row") {
                Some(row) => {
                    if !predictors.iter().any(|p| p == row) {
                        predictors.push(row.to_string());
                    }
                    cells.push((row.to_string(), column, number(cell)?));
                }
                None => constants.push((column, number(cell)?)),
            }
        }
        let functions = constants.iter().map(|(c, _)| *c).max().unwrap_or(0);
        let mut coefficients = vec![vec![0.0; functions]; predictors.len()];
        for (row, column, value) in cells {
            let i = predictors.iter().position(|p| *p == row).unwrap_or(0);
            *coefficients
                .get_mut(i)
                .and_then(|r| r.get_mut(column - 1))
                .ok_or_else(|| PmmlError::Parse("a coefficient column is out of range".into()))? =
                value;
        }
        let mut constant = vec![0.0; functions];
        for (column, value) in constants {
            *constant
                .get_mut(column - 1)
                .ok_or_else(|| PmmlError::Parse("a constant column is out of range".into()))? =
                value;
        }
        // priors and the group values
        let priors_node = model
            .descendants()
            .find(|n| n.has_tag_name("prior-vector"))
            .ok_or_else(|| PmmlError::Parse("the discriminant model has no priors".into()))?;
        let mut groups = Vec::new();
        let mut priors = Vec::new();
        for value in priors_node
            .children()
            .filter(|n| n.has_tag_name("prior-value"))
        {
            groups.push(
                value
                    .attribute("target-category")
                    .ok_or_else(|| PmmlError::Parse("a prior names no group".into()))?
                    .to_string(),
            );
            priors.push(number(value)?);
        }
        let mut centroids = vec![vec![0.0; functions]; groups.len()];
        for cell in model
            .descendants()
            .filter(|n| n.has_tag_name("fgc-matrix"))
            .flat_map(|f| f.children().filter(|n| n.has_tag_name("cell")))
        {
            let (row, column) = (index(cell, "row")?, index(cell, "column")?);
            *centroids
                .get_mut(row - 1)
                .and_then(|r| r.get_mut(column - 1))
                .ok_or_else(|| PmmlError::Parse("a centroid cell is out of range".into()))? =
                number(cell)?;
        }
        Self::new(
            target,
            predictors,
            groups,
            coefficients,
            constant,
            centroids,
            priors,
        )
    }

    /// The scores of a record given its predictors (in [`Self::predictors`] order); `None` when
    /// one is missing.
    pub fn score(&self, values: &[Option<f64>]) -> Option<DiscriminantScore> {
        if values.len() != self.predictors.len() {
            return None;
        }
        let x: Vec<f64> = values
            .iter()
            .map(|v| v.filter(|x| x.is_finite()))
            .collect::<Option<_>>()?;
        let scores: Vec<f64> = (0..self.constants.len())
            .map(|k| {
                self.constants[k]
                    + x.iter()
                        .zip(&self.coefficients)
                        .map(|(x, row)| x * row[k])
                        .sum::<f64>()
            })
            .collect();
        let logs: Vec<f64> = self
            .centroids
            .iter()
            .zip(&self.priors)
            .map(|(centroid, prior)| {
                let d2: f64 = scores
                    .iter()
                    .zip(centroid)
                    .map(|(s, c)| (s - c) * (s - c))
                    .sum();
                if *prior > 0.0 {
                    prior.ln() - 0.5 * d2
                } else {
                    f64::NEG_INFINITY
                }
            })
            .collect();
        let high = logs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if !high.is_finite() {
            return None;
        }
        let weights: Vec<f64> = logs.iter().map(|l| (l - high).exp()).collect();
        let total: f64 = weights.iter().sum();
        let predicted = logs.iter().position(|l| *l == high).unwrap_or_default();
        Some(DiscriminantScore {
            predicted,
            posteriors: weights.iter().map(|w| w / total).collect(),
        })
    }
}

fn mining_field(name: &str, usage: Usage) -> MiningField {
    MiningField {
        name: name.to_string(),
        usage,
        missing_replacement: None,
        outliers: Outliers::AsIs,
        low_value: None,
        high_value: None,
        invalid_treatment: InvalidTreatment::AsIs,
        direction: None,
    }
}
