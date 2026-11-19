//! Modeler's Cox regression nugget: a `GeneralRegressionModel` named `COXREG` whose extensions
//! carry the end-time field, the reference point and the baseline cumulative hazard table.
//!
//! The model gives `H(t | x) = H0(t) * exp(sum_p beta_p * (x_p - ref_p))`, where `H0` is the
//! step function of the table (0 before its first time, the last value after its last time).
//! The survival function is `S(t | x) = exp(-H(t | x))`. A nugget scores a record at several
//! future times, so the generic one-prediction scorer does not apply: the Cox apply node calls
//! [`CoxModel::linear_predictor`] and [`CoxModel::conditional`] itself.
//!
//! Strata and interaction terms are not read (a model that has them is rejected as unsupported):
//! how Modeler writes a stratified baseline is not known.

use crate::parse::{parse_f64, parse_model_header, req_attr};
use crate::{ModelHeader, PmmlError, Usage, Value};

const MAX_TERMS: usize = 100_000;
const MAX_BASELINE_POINTS: usize = 10_000_000;

/// A Cox regression model.
#[derive(Debug, Clone, PartialEq)]
pub struct CoxModel {
    pub header: ModelHeader,
    /// The field holding the survival time of a training record.
    pub time_field: String,
    /// The status field (the predicted field of the mining schema).
    pub event_field: String,
    /// The value of the status field that means the event happened.
    pub event_value: String,
    /// One term per parameter.
    pub terms: Vec<CoxTerm>,
    /// The baseline cumulative hazard `(time, H0)`, times ascending.
    pub baseline: Vec<(f64, f64)>,
}

/// A parameter: the coefficient of a covariate, or of the indicator of one factor level.
#[derive(Debug, Clone, PartialEq)]
pub struct CoxTerm {
    pub field: String,
    /// The factor level this parameter is the indicator of; `None` for a covariate.
    pub level: Option<String>,
    pub beta: f64,
    /// The value of the term at the baseline.
    pub reference: f64,
}

/// The outcome of scoring one record at one future time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoxScore {
    /// `S(t0 + future | x) / S(t0 | x)`: the probability of surviving the interval.
    pub survival: f64,
    /// `H(t0 + future | x) - H(t0 | x)`.
    pub cumulative_hazard: f64,
}

impl CoxModel {
    /// The fields a record must carry for the model to score it.
    pub fn input_fields(&self) -> impl Iterator<Item = &str> {
        self.terms.iter().map(|term| term.field.as_str())
    }

    /// The largest event time of the baseline: later times are not scored.
    pub fn max_event_time(&self) -> Option<f64> {
        self.baseline.last().map(|(time, _)| *time)
    }

    /// `sum_p beta_p * (x_p - ref_p)` for the record `value` reads; `None` when a field the
    /// model uses is missing or a covariate is not a number.
    pub fn linear_predictor<'a>(&self, value: impl Fn(&str) -> Option<&'a Value>) -> Option<f64> {
        let mut lp = 0.0;
        for term in &self.terms {
            let x = match (&term.level, value(&term.field)?) {
                (_, Value::Missing) => return None,
                (None, Value::Number(n)) => *n,
                (None, Value::Text(text)) => text.trim().parse::<f64>().ok()?,
                (Some(level), v) => {
                    if crate::expr::value_matches_str(v, level) {
                        1.0
                    } else {
                        0.0
                    }
                }
            };
            lp += term.beta * (x - term.reference);
        }
        lp.is_finite().then_some(lp)
    }

    /// The baseline cumulative hazard at `time`.
    pub fn baseline_hazard(&self, time: f64) -> f64 {
        let n = self.baseline.partition_point(|(t, _)| *t <= time);
        n.checked_sub(1)
            .and_then(|i| self.baseline.get(i))
            .map_or(0.0, |(_, h)| *h)
    }

    /// Survival over the interval from `past` to `past + future`, given the record's linear
    /// predictor. `None` when the total time lies beyond the longest event time of the baseline
    /// (Algorithms Guide: such records get `$null$`), or when the record cannot have survived
    /// `past`.
    pub fn conditional(&self, lp: f64, past: f64, future: f64) -> Option<CoxScore> {
        let total = past + future;
        if !(past.is_finite() && future.is_finite() && past >= 0.0 && future >= 0.0) {
            return None;
        }
        if total > self.max_event_time()? {
            return None;
        }
        let scale = lp.exp();
        let before = self.baseline_hazard(past) * scale;
        let after = self.baseline_hazard(total) * scale;
        let hazard = after - before;
        if !hazard.is_finite() {
            return None;
        }
        // exp(-after) / exp(-before) without underflow of both
        let survival = (-hazard).exp().clamp(0.0, 1.0);
        Some(CoxScore {
            survival,
            cumulative_hazard: hazard.max(0.0),
        })
    }
}

/// The `Extension` children of `node` named `name`.
fn extension<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children()
        .find(|c| c.has_tag_name("Extension") && c.attribute("name") == Some(name))
}

/// Whether a `GeneralRegressionModel` element is a Cox model.
pub(crate) fn is_cox(node: roxmltree::Node<'_, '_>) -> bool {
    node.attribute("modelName") == Some("COXREG")
}

pub(crate) fn parse(node: roxmltree::Node<'_, '_>) -> Result<CoxModel, PmmlError> {
    let header = parse_model_header(node)?;
    let time_field = extension(node, "endTimeVariable")
        .and_then(|e| e.attribute("value"))
        .ok_or_else(|| PmmlError::Parse("Cox model has no endTimeVariable".into()))?
        .to_string();
    let (event_field, event_value) = {
        let target = node
            .children()
            .find(|c| c.has_tag_name("MiningSchema"))
            .and_then(|schema| {
                schema.children().find(|f| {
                    f.has_tag_name("MiningField") && f.attribute("usageType") == Some("predicted")
                })
            })
            .ok_or_else(|| PmmlError::Parse("Cox model has no status field".into()))?;
        let value = target
            .descendants()
            .find(|n| n.has_tag_name("Value") && n.attribute("property") != Some("invalid"))
            .and_then(|n| n.attribute("value"))
            .ok_or_else(|| PmmlError::Parse("Cox model has no event value".into()))?;
        (req_attr(target, "name")?.to_string(), value.to_string())
    };
    if !header
        .mining_schema
        .iter()
        .any(|f| f.usage == Usage::Target)
    {
        return Err(PmmlError::Parse("Cox model has no predicted field".into()));
    }

    // parameter -> (predictor, level)
    let mut parameters: Vec<(String, f64)> = Vec::new();
    let mut cells: std::collections::HashMap<String, Vec<(String, Option<String>)>> =
        std::collections::HashMap::new();
    let factor_names: std::collections::HashSet<String> = node
        .children()
        .find(|c| c.has_tag_name("FactorList"))
        .map(|list| {
            list.children()
                .filter(|p| p.has_tag_name("Predictor"))
                .filter_map(|p| p.attribute("name").map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if let Some(matrix) = node.children().find(|c| c.has_tag_name("PPMatrix")) {
        for cell in matrix.children().filter(|c| c.has_tag_name("PPCell")) {
            let parameter = req_attr(cell, "parameterName")?.to_string();
            let predictor = req_attr(cell, "predictorName")?.to_string();
            let level = factor_names
                .contains(&predictor)
                .then(|| req_attr(cell, "value").map(str::to_string))
                .transpose()?;
            cells.entry(parameter).or_default().push((predictor, level));
        }
    }
    let matrix = node
        .children()
        .find(|c| c.has_tag_name("ParamMatrix"))
        .ok_or_else(|| PmmlError::Parse("Cox model has no ParamMatrix".into()))?;
    for cell in matrix.children().filter(|c| c.has_tag_name("PCell")) {
        if parameters.len() >= MAX_TERMS {
            return Err(PmmlError::Parse("Cox model has too many parameters".into()));
        }
        parameters.push((
            req_attr(cell, "parameterName")?.to_string(),
            parse_f64(req_attr(cell, "beta")?, "Cox beta")?,
        ));
    }
    let mut references: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    if let Some(point) = extension(node, "referencePoint") {
        for base in point.children().filter(|c| c.has_tag_name("X-BasePoint")) {
            references.insert(
                req_attr(base, "parameterName")?.to_string(),
                parse_f64(req_attr(base, "value")?, "Cox reference point")?,
            );
        }
    }
    let mut terms = Vec::with_capacity(parameters.len());
    for (name, beta) in parameters {
        let parts = cells
            .get(&name)
            .ok_or_else(|| PmmlError::Parse(format!("Cox parameter {name} is not in PPMatrix")))?;
        let [(field, level)] = parts.as_slice() else {
            return Err(PmmlError::Unsupported(
                "Cox model with interaction terms".into(),
            ));
        };
        terms.push(CoxTerm {
            field: field.clone(),
            level: level.clone(),
            beta,
            reference: references.get(&name).copied().unwrap_or(0.0),
        });
    }

    let tables = extension(node, "X-BaseCumHazardTables")
        .ok_or_else(|| PmmlError::Parse("Cox model has no baseline hazard table".into()))?;
    let mut tables = tables
        .children()
        .filter(|c| c.has_tag_name("X-BaseCumHazardTable"));
    let (Some(table), None) = (tables.next(), tables.next()) else {
        return Err(PmmlError::Unsupported(
            "stratified Cox model (not one baseline table)".into(),
        ));
    };
    let mut baseline = Vec::new();
    for point in table.children().filter(|c| c.has_tag_name("X-Baseline")) {
        if baseline.len() >= MAX_BASELINE_POINTS {
            return Err(PmmlError::Parse("Cox baseline table is too long".into()));
        }
        baseline.push((
            parse_f64(req_attr(point, "time")?, "Cox baseline time")?,
            parse_f64(req_attr(point, "cumHazard")?, "Cox baseline hazard")?,
        ));
    }
    if baseline.windows(2).any(|w| w[0].0 >= w[1].0) {
        return Err(PmmlError::Parse(
            "Cox baseline times are not ascending".into(),
        ));
    }
    Ok(CoxModel {
        header,
        time_field,
        event_field,
        event_value,
        terms,
        baseline,
    })
}
