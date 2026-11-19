//! Modeler's Temporal Causal Model nugget: one `TCMXML` document per modelled series
//! (`corpus/ibm-demos/tcm_kpi.str`, the `InputTCMXML` container).
//!
//! A series' model is `y_t = b0 + sum over predictors j and lags l of c_{j,l} x_{j,t-l}`: the
//! intercept and, for each selected predictor series, its coefficients on lags `1..=L`. A series
//! with no other predictor than itself is an autoregression (`algorithmName="AR"`) kept for the
//! series that other models use as predictors. The documents name the series by their position
//! in the field list, so the names come from the caller (`TSDPOutput.json`'s `metricFieldList`).
//!
//! The documents write each coefficient array from the largest lag to lag 1; this model keeps
//! them in lag order `1..=L`.

use crate::parse::{parse_f64, req_attr};
use crate::{MiningField, MiningFunction, ModelHeader, PmmlError};

const MAX_SERIES: usize = 10_000;
const MAX_LAG: usize = 1_000;
const MAX_PREDICTORS: usize = 1_000;

/// A TCM model: the series and their models.
#[derive(Debug, Clone, PartialEq)]
pub struct TcmModel {
    pub header: ModelHeader,
    /// The number of lags of every model.
    pub lag: usize,
    /// The series, in the order the documents number them.
    pub series: Vec<String>,
    /// One model per modelled series.
    pub targets: Vec<TcmTarget>,
}

/// The model of one series.
#[derive(Debug, Clone, PartialEq)]
pub struct TcmTarget {
    /// The series this model predicts (position in [`TcmModel::series`]).
    pub series: usize,
    /// An autoregression on the series alone, kept for use as a predictor of other series.
    pub ar_only: bool,
    pub intercept: f64,
    pub predictors: Vec<TcmPredictor>,
}

/// A predictor series of a model.
#[derive(Debug, Clone, PartialEq)]
pub struct TcmPredictor {
    /// The predictor series (position in [`TcmModel::series`]).
    pub series: usize,
    /// The coefficients of lags `1..=L`.
    pub coefficients: Vec<f64>,
}

impl TcmModel {
    /// A model from its series names and models.
    pub fn new(series: Vec<String>, lag: usize, targets: Vec<TcmTarget>) -> Self {
        let header = ModelHeader {
            name: None,
            function: MiningFunction::Other("temporal causal model".into()),
            algorithm: Some("TCM".into()),
            mining_schema: series
                .iter()
                .map(|name| MiningField {
                    name: name.clone(),
                    usage: crate::Usage::Active,
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
            lag,
            series,
            targets,
        }
    }

    /// The model read from the `TCMXML` documents of a nugget's container. `series` names the
    /// series by position.
    pub fn from_documents(series: Vec<String>, documents: &[&str]) -> Result<Self, PmmlError> {
        if series.len() > MAX_SERIES {
            return Err(PmmlError::Parse("TCM has too many series".into()));
        }
        let mut targets = Vec::new();
        let mut lag = 0usize;
        for text in documents {
            let document = roxmltree::Document::parse(text)
                .map_err(|error| PmmlError::Parse(format!("TCMXML: {error}")))?;
            let model = document
                .descendants()
                .find(|n| n.has_tag_name("TemporalCausalModeling"))
                .ok_or_else(|| PmmlError::Parse("TCMXML has no TemporalCausalModeling".into()))?;
            let position = |text: &str| -> Result<usize, PmmlError> {
                text.trim()
                    .parse::<usize>()
                    .ok()
                    .filter(|i| *i < series.len())
                    .ok_or_else(|| PmmlError::Parse(format!("TCMXML names series {text:?}")))
            };
            let target = position(req_attr(model, "targetField")?)?;
            let max_lags = req_attr(model, "maxLags")?
                .trim()
                .parse::<usize>()
                .ok()
                .filter(|l| (1..=MAX_LAG).contains(l))
                .ok_or_else(|| PmmlError::Parse("TCMXML has a bad maxLags".into()))?;
            if lag != 0 && lag != max_lags {
                return Err(PmmlError::Unsupported(
                    "TCM models with different numbers of lags".into(),
                ));
            }
            lag = max_lags;
            let ar_only = model.attribute("algorithmName") == Some("AR");
            let predictors_node = model
                .children()
                .find(|n| n.has_tag_name("Predictors"))
                .ok_or_else(|| PmmlError::Parse("TCMXML has no Predictors".into()))?;
            let intercept = predictors_node
                .children()
                .find(|n| n.has_tag_name("Intercept"))
                .map(|n| parse_f64(req_attr(n, "value")?, "TCM intercept"))
                .transpose()?
                .unwrap_or(0.0);
            let mut predictors = Vec::new();
            for node in predictors_node
                .children()
                .filter(|n| n.has_tag_name("Predictor"))
            {
                if predictors.len() >= MAX_PREDICTORS {
                    return Err(PmmlError::Parse("TCM model has too many predictors".into()));
                }
                let mut coefficients = node
                    .children()
                    .find(|n| n.has_tag_name("Coefficients"))
                    .and_then(|n| n.descendants().find(|a| a.has_tag_name("Array")))
                    .map(|a| {
                        a.text()
                            .unwrap_or_default()
                            .split_whitespace()
                            .map(|t| parse_f64(t, "TCM coefficient"))
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .transpose()?
                    .unwrap_or_default();
                if coefficients.len() != max_lags {
                    return Err(PmmlError::Parse(
                        "a TCM predictor does not have one coefficient per lag".into(),
                    ));
                }
                // the document lists lag L first
                coefficients.reverse();
                predictors.push(TcmPredictor {
                    series: position(req_attr(node, "name")?)?,
                    coefficients,
                });
            }
            targets.push(TcmTarget {
                series: target,
                ar_only,
                intercept,
                predictors,
            });
        }
        if targets.is_empty() {
            return Err(PmmlError::Parse("TCM model has no series".into()));
        }
        targets.sort_by_key(|t| t.series);
        Ok(Self::new(series, lag, targets))
    }

    /// The one-step-ahead fitted values of a target over aligned series `columns` (one per
    /// series, `None` for a missing value): entry `t` is `None` for the first `L` time points and
    /// wherever a value the model reads is missing.
    pub fn fitted(&self, target: &TcmTarget, columns: &[Vec<Option<f64>>]) -> Vec<Option<f64>> {
        let n = columns.first().map_or(0, Vec::len);
        (0..n)
            .map(|t| {
                if t < self.lag {
                    return None;
                }
                let mut y = target.intercept;
                for predictor in &target.predictors {
                    let column = columns.get(predictor.series)?;
                    for (k, coefficient) in predictor.coefficients.iter().enumerate() {
                        y += coefficient * (*column.get(t - 1 - k)?)?;
                    }
                }
                y.is_finite().then_some(y)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r#"<?xml version="1.0"?><TCMXML><TemporalCausalModeling algorithmName="TCM" targetField="1" maxLags="2"><Predictors><Intercept value="10"/><Predictor name="1"><Coefficients><Array n="2" type="real">0.5 2</Array></Coefficients></Predictor><Predictor name="0"><Coefficients><Array n="2" type="real">0.25 1</Array></Coefficients></Predictor></Predictors></TemporalCausalModeling></TCMXML>"#;

    #[test]
    fn tcm_model_reads_documents_and_fits_one_step_ahead() {
        let model = TcmModel::from_documents(vec!["a".into(), "b".into()], &[DOC]).unwrap();
        assert_eq!(model.lag, 2);
        let target = &model.targets[0];
        // the document lists lag 2 first
        assert_eq!(target.predictors[0].coefficients, vec![2.0, 0.5]);
        let a = vec![Some(1.0), Some(2.0), Some(3.0), None, Some(5.0)];
        let b = vec![Some(1.0), Some(1.0), Some(2.0), Some(4.0), Some(8.0)];
        let fitted = model.fitted(target, &[a, b]);
        assert_eq!(fitted[0], None);
        assert_eq!(fitted[1], None);
        // t = 2: 10 + 2*b1 + 0.5*b0 + 1*a1 + 0.25*a0 = 10 + 2 + 0.5 + 2 + 0.25
        assert!((fitted[2].unwrap() - 14.75).abs() < 1e-12);
        // t = 4 reads a[3], which is missing
        assert_eq!(fitted[4], None);
    }

    #[test]
    fn tcm_model_rejects_bad_documents() {
        assert!(TcmModel::from_documents(vec!["a".into()], &[DOC]).is_err());
        assert!(TcmModel::from_documents(vec!["a".into(), "b".into()], &["<x/>"]).is_err());
        assert!(TcmModel::from_documents(vec!["a".into(), "b".into()], &[]).is_err());
    }
}
