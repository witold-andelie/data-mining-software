//! Feature Selection, IBM SPSS Modeler 19: public Algorithms Guide, chapter 17,
//! pp. 153–161 (local text copyright 1994–2016), and Python Scripting and
//! Automation Guide 19.0, pp. 245–247, 331. No IBM implementation is used.
//!
//! Settings / saved XML (exec reads these names):
//! - `screen_missing_values/max_missing_values` = `screenMissingValues/maxMissingValues`:
//!   missing percent > threshold (`fs_screen_missing`: 39/40/41 of 80, 50 retained).
//! - `screen_single_category/max_single_category` = `screenMaxSingleCategory/maxSingleCategory`:
//!   categorical modal count / cases > percent (`fs_screen_dominance`: 63/64 kept, 65 removed).
//! - `screen_num_categories/max_num_categories` = `screenMaxCategoriesAsCases/maxCategoriesAsCases`:
//!   categories / cases > percent, NOT an absolute category count (`fs_screen_categories`: 7/8 kept,
//!   9 removed at 10% of 80). Unique categorical identifiers are screened even with switches off.
//! - `screen_std_dev/min_std_dev` = `screenStdDev/minStdDev`: sample SD < threshold
//!   (`fs_screen_sd`: .49 removed, .50 retained); `screen_coeff_of_var/min_coeff_of_var` =
//!   `screenVariability/minVariability`: absolute sample SD / mean < threshold (raw ratio, not
//!   percent: `fs_screen_cv`, .049 removed, +/-.050 kept); zero mean survives CV screening.
//! - All-null/uninstantiated categorical inputs are ignored and pass through the nugget
//!   (`all_null` in all categorical cases). Constant categorical predictors remain ranked when
//!   dominance screening is off (`fs_pearson`), contrary to the guide's unconditional screen.
//! - Missing target / all-input-missing cases do not train; association uses nonmissing pairs.
//!   GAP: these 15 cases do not isolate the order/denominators of case and variable screening.
//! - `criteria`: `pearson`, `likelihood`, `cramersV`, `lambda` (`fs_pearson`, `fs_likelihood`,
//!   `fs_cramers_v`, `fs_lambda`). Pearson / likelihood importance is 1-p; V and directional
//!   lambda ARE the importance, not 1-p. Nominal constants have p=1; Cramer's V is Infinity
//!   for the saved constant, ranked last (`fs_cramers_v`). Mixed categorical-target inputs
//!   support only Pearson/likelihood; other choices return unsupported.
//! - Numeric vs categorical uses one-way ANOVA; two numeric fields use the two-sided Pearson
//!   correlation t test, equivalently F(1,n-2). `fs_continuous_target` selects `group`.
//!   Saved positive-correlation and ANOVA p-values cancel to zero in this case, whereas negative
//!   correlation retains a tiny tail. We preserve that CDF-subtraction behavior. GAP: exact
//!   floating-point ranks among rescaled positive copies differ and are not asserted.
//! - Guide tie rules: p ascending, statistic descending, df ascending (chi-square) or pair count
//!   descending (F/r); V descending then chi-square; lambda descending then categories ascending;
//!   finally physical column order. Mixed-type exact p ties group predictors by measurement,
//!   then order groups by their first column. The continuous-target oracle instead puts the
//!   categorical ANOVA group first at p=0, even though `group` is the last column; this observed
//!   exception is preserved. GAP: a mixed zero-p permutation probe is needed for other layouts.
//!   `fs_tie_column_order` swaps both column and custom
//!   input order: GAP: an independent permutation is needed to distinguish those two orders.
//! - `unimportant_below/important_above` = `importanceLowThreshold/importanceMediumThreshold`:
//!   importance < low is Unimportant, >= high Important, otherwise Marginal (default .90/.95).
//!   `*_label` only names bands (`fs_importance_levels`, .90/.99, renamed labels).
//!   GAP: no case lands exactly on a band boundary; >= high follows the options wording.
//! - `selection_mode` = `fieldSelectionMode`: `importanceLevel` uses `selectImportantFields`,
//!   `selectMarginalFields`, `selectUnimportantFields`; `topN` uses `topNFields`; `importanceValue`
//!   uses a STRICT comparison with the saved raw value. `fs_importance_value_50` saves 50.0 and
//!   selects nothing, despite the scripting guide advertising 0–100 units. No percent conversion.
//!   GAP: a .5/50 paired probe and exact equality are needed to settle the public API's units.
//! - `custom_fields/target/inputs` = `autoModelFieldSelection` (inverted),
//!   `modelTargetFieldSelection/modelInputFieldSelection`; model name/labels are presentation.
//!   `usePartitionedData` trains only the first declared partition (`fs_training_partition`).
//!   Frequency/weight fields are unused, as documented. Split models are unsupported.
//! - Nugget `rankedFieldSelection/screenedFieldSelection` override automatic choices independently
//!   (`fs_manual_fields`). Only modeled, unselected inputs are removed; all other fields, row
//!   order and metadata survive, in physical order. An empty selection is NOT select-all.
//!   GAP: `fs_default_mixed` has no oracle directory; missing/blank mixed-target behavior needs
//!   a successful isolated case. No automatic list-length formula or split models are claimed.

use crate::{AlgoError, Dataset, stats};
use dms_types::{Field, Measurement, Storage, Value, ValueSpec};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Criterion {
    Pearson,
    Likelihood,
    CramersV,
    Lambda,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Important,
    Marginal,
    Unimportant,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Selection {
    Levels {
        important: bool,
        marginal: bool,
        unimportant: bool,
    },
    TopN(usize),
    /// Raw saved importance units, including the observed value 50.0.
    Value(f64),
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub max_missing_percent: Option<f64>,
    pub max_single_percent: Option<f64>,
    pub max_categories_percent: Option<f64>,
    pub min_sd: Option<f64>,
    pub min_cv: Option<f64>,
    pub criterion: Criterion,
    pub low: f64,
    pub high: f64,
    pub labels: [String; 3],
    pub selection: Selection,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_missing_percent: Some(70.0),
            max_single_percent: Some(90.0),
            max_categories_percent: Some(95.0),
            min_sd: Some(0.0),
            min_cv: Some(0.1),
            criterion: Criterion::Pearson,
            low: 0.9,
            high: 0.95,
            labels: ["Important".into(), "Marginal".into(), "Unimportant".into()],
            selection: Selection::Levels {
                important: true,
                marginal: false,
                unimportant: false,
            },
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<(), AlgoError> {
        for value in [
            self.max_missing_percent,
            self.max_single_percent,
            self.max_categories_percent,
        ]
        .into_iter()
        .flatten()
        {
            crate::check_cancelled()?;
            if !value.is_finite() || !(0.0..=100.0).contains(&value) {
                return Err(AlgoError::Settings(
                    "Feature Selection screening percent must be in 0..100".into(),
                ));
            }
        }
        if [self.min_sd, self.min_cv]
            .into_iter()
            .flatten()
            .any(|v| !v.is_finite() || v < 0.0)
            || !self.low.is_finite()
            || !self.high.is_finite()
            || !(0.0..=1.0).contains(&self.low)
            || !(self.low..=1.0).contains(&self.high)
        {
            return Err(AlgoError::Settings(
                "Feature Selection requires finite nonnegative SD/CV and 0 <= low <= high <= 1"
                    .into(),
            ));
        }
        match self.selection {
            Selection::TopN(n) if n > 1000 => {
                return Err(AlgoError::Settings(
                    "Feature Selection top N must be in 0..1000".into(),
                ));
            }
            Selection::Value(v) if !v.is_finite() || !(0.0..=100.0).contains(&v) => {
                return Err(AlgoError::Settings(
                    "Feature Selection importance value must be in 0..100".into(),
                ));
            }
            _ => {}
        }
        Ok(())
    }

    pub fn level(&self, importance: f64) -> Level {
        if importance >= self.high {
            Level::Important
        } else if importance < self.low {
            Level::Unimportant
        } else {
            Level::Marginal
        }
    }
}

#[derive(Debug, Clone)]
pub struct RankedField {
    pub position: usize,
    pub name: String,
    /// For V/lambda this is the association measure, as saved by Modeler.
    pub p_value: f64,
    pub importance: f64,
    pub statistic: f64,
    pub df: f64,
    pub pairs: usize,
    pub categories: usize,
    pub level: Level,
    pub label: String,
    kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Categorical,
    Anova,
    Correlation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenReason {
    Missing,
    Categories,
    Dominance,
    StandardDeviation,
    Variation,
    Identifier,
    Empty,
}

#[derive(Debug, Clone)]
pub struct ScreenedField {
    pub position: usize,
    pub name: String,
    pub reason: ScreenReason,
}

#[derive(Debug, Clone)]
pub struct FeatureSelectionModel {
    /// Inputs with an instantiated measurement level. Ignored inputs pass through scoring.
    pub inputs: Vec<usize>,
    pub ranked: Vec<RankedField>,
    pub screened: Vec<ScreenedField>,
    pub selected: Vec<usize>,
}

fn categorical(field: &Field) -> Result<bool, AlgoError> {
    if !matches!(
        field.storage,
        Storage::String | Storage::Integer | Storage::Real
    ) {
        return Err(AlgoError::Unsupported(format!(
            "Feature Selection storage {:?} for {}",
            field.storage, field.name
        )));
    }
    match field.measurement {
        Measurement::Continuous => Ok(false),
        Measurement::Categorical
        | Measurement::Flag
        | Measurement::Nominal
        | Measurement::Ordinal => Ok(true),
        other => Err(AlgoError::Unsupported(format!(
            "Feature Selection measurement {other:?} for {}",
            field.name
        ))),
    }
}

fn value<'a>(row: &'a [Value], position: usize, field: &Field) -> Option<&'a Value> {
    row.get(position).filter(|v| !field.is_missing(v))
}

fn number(v: &Value) -> Result<f64, AlgoError> {
    v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| {
        AlgoError::Data(
            "Feature Selection needs finite numeric values for continuous fields".into(),
        )
    })
}

fn field<'a>(dataset: &Dataset<'a>, position: usize) -> Result<&'a Field, AlgoError> {
    dataset.table.model().fields().get(position).ok_or_else(|| {
        AlgoError::Data(format!(
            "Feature Selection field index {position} is out of range"
        ))
    })
}

/// Learn screens, association rankings and automatic selections. Weights are deliberately unused.
pub fn train(
    dataset: &Dataset<'_>,
    settings: &Settings,
) -> Result<FeatureSelectionModel, AlgoError> {
    crate::check_cancelled()?;
    settings.validate()?;
    if dataset.inputs.len() > 256
        || dataset.table.row_count() > 1_000_000
        || dataset
            .table
            .row_count()
            .saturating_mul(dataset.table.model().len())
            > 16_000_000
    {
        return Err(AlgoError::Data(
            "Feature Selection limit: 256 inputs / 1000000 rows / 16000000 cells".into(),
        ));
    }
    let target = field(dataset, dataset.target)?;
    let target_cat = categorical(target)?;
    let mut inputs = Vec::new();
    let mut unique = BTreeSet::new();
    for &position in dataset.inputs {
        crate::check_cancelled()?;
        if position == dataset.target || !unique.insert(position) {
            return Err(AlgoError::Settings(
                "Feature Selection target/input overlap or duplicate input".into(),
            ));
        }
        let f = field(dataset, position)?;
        if f.measurement == Measurement::Typeless
            && dataset
                .table
                .rows()
                .iter()
                .all(|r| value(r, position, f).is_none())
        {
            continue;
        }
        let cat = categorical(f)?;
        // The all-null categorical field is absent from Modeler's saved model/inputs.
        if cat
            && (matches!(&f.values, ValueSpec::Categories(v) if v.is_empty())
                || (matches!(f.values, ValueSpec::Unknown)
                    && dataset
                        .table
                        .rows()
                        .iter()
                        .all(|r| value(r, position, f).is_none())))
        {
            continue;
        }
        inputs.push(position);
    }
    if target_cat
        && matches!(settings.criterion, Criterion::CramersV | Criterion::Lambda)
        && inputs
            .iter()
            .any(|p| field(dataset, *p).is_ok_and(|f| f.measurement == Measurement::Continuous))
    {
        return Err(AlgoError::Unsupported(
            "Feature Selection Cramer's V/lambda with mixed predictors".into(),
        ));
    }
    let rows: Vec<_> = dataset
        .table
        .rows()
        .iter()
        .filter(|r| {
            value(r, dataset.target, target).is_some()
                && inputs
                    .iter()
                    .any(|p| field(dataset, *p).is_ok_and(|f| value(r, *p, f).is_some()))
        })
        .collect();
    if rows.is_empty() {
        return Err(AlgoError::Data(
            "Feature Selection has no valid training cases".into(),
        ));
    }
    let mut model = FeatureSelectionModel {
        inputs: inputs.clone(),
        ranked: Vec::new(),
        screened: Vec::new(),
        selected: Vec::new(),
    };
    for position in inputs {
        crate::check_cancelled()?;
        let input = field(dataset, position)?;
        let cat = categorical(input)?;
        let pairs: Vec<_> = rows
            .iter()
            .filter_map(|r| {
                Some((
                    value(r, position, input)?,
                    value(r, dataset.target, target)?,
                ))
            })
            .collect();
        let counts = if cat {
            counts(pairs.iter().map(|(x, _)| *x))?
        } else {
            BTreeMap::new()
        };
        let n = pairs.len();
        let total = rows.len() as f64;
        let mut reason = if settings
            .max_missing_percent
            .is_some_and(|m| (rows.len().saturating_sub(n)) as f64 * 100.0 > m * total)
        {
            Some(ScreenReason::Missing)
        } else {
            None
        };
        if cat {
            if reason.is_none()
                && settings
                    .max_categories_percent
                    .is_some_and(|m| counts.len() as f64 * 100.0 > m * total)
            {
                reason = Some(ScreenReason::Categories);
            }
            if reason.is_none()
                && settings.max_single_percent.is_some_and(|m| {
                    counts.values().copied().fold(0.0, f64::max) * 100.0 > m * total
                })
            {
                reason = Some(ScreenReason::Dominance);
            }
            if reason.is_none() && n > 0 && counts.len() == n {
                reason = Some(ScreenReason::Identifier);
            }
        } else {
            let values = pairs
                .iter()
                .map(|(x, _)| number(x))
                .collect::<Result<Vec<_>, _>>()?;
            let (mean, ss) = moments(&values)?;
            let sd = if n > 1 {
                (ss / (n - 1) as f64).sqrt()
            } else {
                0.0
            };
            if reason.is_none() && (n == 0 || sd == 0.0) {
                reason = Some(ScreenReason::Empty);
            }
            if reason.is_none() && settings.min_sd.is_some_and(|m| below(sd, m)) {
                reason = Some(ScreenReason::StandardDeviation);
            }
            if reason.is_none()
                && mean != 0.0
                && settings.min_cv.is_some_and(|m| below(sd / mean.abs(), m))
            {
                reason = Some(ScreenReason::Variation);
            }
        }
        if let Some(reason) = reason {
            model.screened.push(ScreenedField {
                position,
                name: input.name.clone(),
                reason,
            });
            continue;
        }
        let association = match (cat, target_cat) {
            (true, true) => categorical_association(&pairs, settings.criterion)?,
            (false, false) => correlation(&pairs)?,
            _ => anova(&pairs, cat)?,
        };
        let importance = association.importance;
        let level = settings.level(importance);
        let label = match level {
            Level::Important => settings.labels.first(),
            Level::Marginal => settings.labels.get(1),
            Level::Unimportant => settings.labels.get(2),
        }
        .cloned()
        .unwrap_or_default();
        model.ranked.push(RankedField {
            position,
            name: input.name.clone(),
            p_value: association.p,
            importance,
            statistic: association.statistic,
            df: association.df,
            pairs: n,
            categories: counts.len(),
            level,
            label,
            kind: association.kind,
        });
    }
    sort_ranking(&mut model.ranked, settings.criterion);
    model.selected = model
        .ranked
        .iter()
        .enumerate()
        .filter(|(rank, f)| match settings.selection {
            Selection::TopN(n) => *rank < n,
            Selection::Value(v) => f.importance > v,
            Selection::Levels {
                important,
                marginal,
                unimportant,
            } => match f.level {
                Level::Important => important,
                Level::Marginal => marginal,
                Level::Unimportant => unimportant,
            },
        })
        .map(|(_, f)| f.position)
        .collect();
    Ok(model)
}

// Oracle .50 and .050 boundaries survive round-off in exported decimal training values.
fn below(value: f64, threshold: f64) -> bool {
    value < threshold && threshold - value > 32.0 * f64::EPSILON * value.abs().max(threshold.abs())
}

fn counts<'a>(values: impl Iterator<Item = &'a Value>) -> Result<BTreeMap<String, f64>, AlgoError> {
    let mut result = BTreeMap::new();
    for v in values {
        crate::check_cancelled()?;
        *result.entry(v.to_string()).or_insert(0.0) += 1.0;
        if result.len() > 4096 {
            return Err(AlgoError::Data(
                "Feature Selection limit: 4096 distinct values per field".into(),
            ));
        }
    }
    Ok(result)
}

fn moments(values: &[f64]) -> Result<(f64, f64), AlgoError> {
    if values.is_empty() {
        return Ok((0.0, 0.0));
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let ss = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>();
    if !mean.is_finite() || !ss.is_finite() {
        return Err(AlgoError::Data(
            "Feature Selection numeric range overflow".into(),
        ));
    }
    Ok((mean, ss))
}

struct Association {
    p: f64,
    importance: f64,
    statistic: f64,
    df: f64,
    kind: Kind,
}

// CDF subtraction reproduces the rounding of saved chi-square/F p-values, including zero tails.
fn cdf_tail(p: f64) -> f64 {
    1.0 - (1.0 - p)
}

fn categorical_association(
    pairs: &[(&Value, &Value)],
    criterion: Criterion,
) -> Result<Association, AlgoError> {
    let x = counts(pairs.iter().map(|(x, _)| *x))?;
    let y = counts(pairs.iter().map(|(_, y)| *y))?;
    let mut cells = BTreeMap::<(String, String), f64>::new();
    for (a, b) in pairs {
        crate::check_cancelled()?;
        *cells.entry((a.to_string(), b.to_string())).or_default() += 1.0;
    }
    let n = pairs.len() as f64;
    let df = x.len().saturating_sub(1) as f64 * y.len().saturating_sub(1) as f64;
    // Sum O^2/E - N over observed cells includes the zero cells without a dense allocation.
    let mut pearson = -n;
    let mut likelihood = 0.0;
    let mut conditional_correct = BTreeMap::<&str, f64>::new();
    for ((a, b), observed) in &cells {
        crate::check_cancelled()?;
        let expected = x.get(a).copied().unwrap_or(0.0) * y.get(b).copied().unwrap_or(0.0) / n;
        if expected > 0.0 {
            pearson += observed * observed / expected;
            likelihood += 2.0 * observed * (observed / expected).ln();
        }
        let max = conditional_correct.entry(a).or_default();
        *max = max.max(*observed);
    }
    // Normalize signed zero: total ordering must not put an empty-pair field after df=1 ties.
    pearson = if pearson > 0.0 { pearson } else { 0.0 };
    let statistic = if criterion == Criterion::Likelihood {
        likelihood.max(0.0)
    } else {
        pearson
    };
    let p = match criterion {
        Criterion::Pearson | Criterion::Likelihood => cdf_tail(stats::chi_square_p(statistic, df)),
        Criterion::CramersV if x.len() == 1 && y.len() > 1 => f64::INFINITY,
        Criterion::CramersV => {
            let denominator = n * x.len().saturating_sub(1).min(y.len().saturating_sub(1)) as f64;
            if denominator > 0.0 {
                (pearson / denominator).sqrt().min(1.0)
            } else {
                0.0
            }
        }
        Criterion::Lambda => {
            let errors = n - y.values().copied().fold(0.0, f64::max);
            if errors > 0.0 {
                ((conditional_correct.values().sum::<f64>() - (n - errors)) / errors)
                    .clamp(0.0, 1.0)
            } else {
                0.0
            }
        }
    };
    let importance = if matches!(criterion, Criterion::CramersV | Criterion::Lambda) {
        p
    } else {
        1.0 - p
    };
    Ok(Association {
        p,
        importance,
        statistic,
        df,
        kind: Kind::Categorical,
    })
}

fn anova(
    pairs: &[(&Value, &Value)],
    predictor_categorical: bool,
) -> Result<Association, AlgoError> {
    let mut groups = BTreeMap::<String, Vec<f64>>::new();
    let mut values = Vec::new();
    for (x, y) in pairs {
        crate::check_cancelled()?;
        let (category, continuous) = if predictor_categorical {
            (x, y)
        } else {
            (y, x)
        };
        let v = number(continuous)?;
        values.push(v);
        groups.entry(category.to_string()).or_default().push(v);
    }
    if groups.len() > 4096 {
        return Err(AlgoError::Data(
            "Feature Selection limit: 4096 ANOVA groups".into(),
        ));
    }
    let (mean, _) = moments(&values)?;
    let mut within = 0.0;
    let mut between = 0.0;
    for group in groups.values() {
        crate::check_cancelled()?;
        let (group_mean, ss) = moments(group)?;
        within += ss;
        between += group.len() as f64 * (group_mean - mean).powi(2);
    }
    let df = groups.len().saturating_sub(1) as f64;
    let residual = pairs.len().saturating_sub(groups.len()) as f64;
    let (statistic, p) = if df <= 0.0 || residual <= 0.0 {
        (0.0, 1.0)
    } else if within == 0.0 {
        (f64::INFINITY, 0.0)
    } else {
        let f = between / df / (within / residual);
        (f, cdf_tail(stats::f_p(f, df, residual)))
    };
    if between.is_infinite() || within.is_infinite() {
        return Err(AlgoError::Data("Feature Selection ANOVA overflow".into()));
    }
    Ok(Association {
        p,
        importance: 1.0 - p,
        statistic,
        df,
        kind: Kind::Anova,
    })
}

fn correlation(pairs: &[(&Value, &Value)]) -> Result<Association, AlgoError> {
    let xs = pairs
        .iter()
        .map(|(x, _)| number(x))
        .collect::<Result<Vec<_>, _>>()?;
    let ys = pairs
        .iter()
        .map(|(_, y)| number(y))
        .collect::<Result<Vec<_>, _>>()?;
    let (mx, xx) = moments(&xs)?;
    let (my, yy) = moments(&ys)?;
    let xy: f64 = xs.iter().zip(&ys).map(|(x, y)| (x - mx) * (y - my)).sum();
    let r = if xx > 0.0 && yy > 0.0 {
        (xy / xx.sqrt() / yy.sqrt()).clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let r2 = r * r;
    let df = pairs.len().saturating_sub(2) as f64;
    let p = if df <= 0.0 || xx == 0.0 || yy == 0.0 {
        1.0
    } else if r2 >= 1.0 {
        0.0
    } else {
        let tail = stats::f_p(r2 * df / (1.0 - r2), 1.0, df);
        if r >= 0.0 {
            2.0 * cdf_tail(tail / 2.0)
        } else {
            tail
        }
    };
    if !xy.is_finite() {
        return Err(AlgoError::Data(
            "Feature Selection correlation overflow".into(),
        ));
    }
    Ok(Association {
        p,
        importance: 1.0 - p,
        statistic: r2,
        df,
        kind: Kind::Correlation,
    })
}

fn sort_ranking(fields: &mut [RankedField], criterion: Criterion) {
    // First order exact p-value groups; then sort within each measurement group. This avoids
    // a non-transitive comparator when statistics have different meanings across groups.
    fields.sort_by(|a, b| {
        let special = |f: &RankedField| !f.importance.is_finite();
        special(a)
            .cmp(&special(b))
            .then_with(|| {
                if matches!(criterion, Criterion::CramersV | Criterion::Lambda)
                    && a.kind == Kind::Categorical
                    && b.kind == Kind::Categorical
                {
                    b.importance.total_cmp(&a.importance)
                } else {
                    a.p_value.total_cmp(&b.p_value)
                }
            })
            .then(a.position.cmp(&b.position))
    });
    let mut start = 0;
    while start < fields.len() {
        let Some(first) = fields.get(start) else {
            break;
        };
        let key = first.p_value;
        let mut end = start.saturating_add(1);
        while fields.get(end).is_some_and(|f| f.p_value == key) {
            end = end.saturating_add(1);
        }
        if let Some(group) = fields.get_mut(start..end) {
            let first_cat = group
                .iter()
                .filter(|f| f.kind == Kind::Categorical)
                .map(|f| f.position)
                .min()
                .unwrap_or(usize::MAX);
            let first_num = group
                .iter()
                .filter(|f| f.kind != Kind::Categorical)
                .map(|f| f.position)
                .min()
                .unwrap_or(usize::MAX);
            group.sort_by(|a, b| {
                let group_order = |f: &RankedField| {
                    if f.kind == Kind::Categorical {
                        first_cat
                    } else {
                        first_num
                    }
                };
                let continuous_target_group = |f: &RankedField| f.kind == Kind::Correlation;
                continuous_target_group(a)
                    .cmp(&continuous_target_group(b))
                    .then(group_order(a).cmp(&group_order(b)))
                    .then_with(|| {
                        if a.kind == Kind::Categorical && criterion == Criterion::Lambda {
                            a.categories.cmp(&b.categories)
                        } else {
                            b.statistic.total_cmp(&a.statistic).then_with(|| {
                                if a.kind == Kind::Categorical && criterion != Criterion::CramersV {
                                    a.df.total_cmp(&b.df)
                                } else if a.kind != Kind::Categorical {
                                    b.pairs.cmp(&a.pairs)
                                } else {
                                    Ordering::Equal
                                }
                            })
                        }
                    })
                    .then(a.position.cmp(&b.position))
            });
        }
        start = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dms_types::{DataModel, Storage, Table};

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-10, "{a} != {b}");
    }

    #[test]
    fn contingency_statistics_and_directional_lambda() {
        // Synthetic 2x2: [[30,10],[10,30]], Pearson=20, G2=20.92992575.
        let a = Value::string("a");
        let b = Value::string("b");
        let pairs: Vec<_> = std::iter::repeat_n((&a, &a), 30)
            .chain(std::iter::repeat_n((&a, &b), 10))
            .chain(std::iter::repeat_n((&b, &a), 10))
            .chain(std::iter::repeat_n((&b, &b), 30))
            .collect();
        let p = categorical_association(&pairs, Criterion::Pearson).unwrap();
        close(p.statistic, 20.0);
        close(p.p, 0.00000774421643101597);
        close(p.importance, 1.0 - p.p);
        let g = categorical_association(&pairs, Criterion::Likelihood).unwrap();
        close(g.statistic, 20.929_925_750_581_92);
        close(g.p, 0.00000476393847959056);
        close(
            categorical_association(&pairs, Criterion::CramersV)
                .unwrap()
                .importance,
            0.5,
        );
        close(
            categorical_association(&pairs, Criterion::Lambda)
                .unwrap()
                .importance,
            0.5,
        );
        // Asymmetric target proportions distinguish directional from symmetric lambda.
        let pairs: Vec<_> = std::iter::repeat_n((&a, &a), 30)
            .chain(std::iter::repeat_n((&a, &b), 10))
            .chain(std::iter::repeat_n((&b, &a), 5))
            .chain(std::iter::repeat_n((&b, &b), 15))
            .collect();
        close(
            categorical_association(&pairs, Criterion::Lambda)
                .unwrap()
                .importance,
            0.4,
        );
    }

    #[test]
    fn anova_and_correlation_known_statistics() {
        let a = Value::string("a");
        let b = Value::string("b");
        let values: Vec<_> = [1., 2., 3., 4., 5., 6.].map(Value::Real).into();
        let pairs: Vec<_> = values
            .iter()
            .enumerate()
            .map(|(i, v)| (if i < 3 { &a } else { &b }, v))
            .collect();
        let result = anova(&pairs, true).unwrap();
        close(result.statistic, 13.5);
        close(result.p, stats::f_p(13.5, 1., 4.));
        let y: Vec<_> = [1., 2., 4., 3., 6., 5.].map(Value::Real).into();
        let positive: Vec<_> = values.iter().zip(&y).collect();
        let negative: Vec<_> = values
            .iter()
            .map(|v| Value::real(-v.as_f64().unwrap()))
            .collect();
        let result = correlation(&positive).unwrap();
        close(result.statistic, (31.0_f64 / 35.0).powi(2));
        close(
            result.p,
            correlation(&negative.iter().zip(&y).collect::<Vec<_>>())
                .unwrap()
                .p,
        );
        let pairs: Vec<_> = std::iter::repeat_n((&a, &values[0]), 3)
            .chain(std::iter::repeat_n((&b, &values[5]), 3))
            .collect();
        assert_eq!(anova(&pairs, true).unwrap().p, 0.0);
    }

    fn settings_off() -> Settings {
        Settings {
            max_missing_percent: None,
            max_single_percent: None,
            max_categories_percent: None,
            min_sd: None,
            min_cv: None,
            selection: Selection::TopN(1000),
            ..Settings::default()
        }
    }

    #[test]
    fn bands_value_units_and_invalid_settings() {
        let s = Settings {
            low: 0.9,
            high: 0.99,
            ..settings_off()
        };
        assert_eq!(s.level(0.899), Level::Unimportant);
        assert_eq!(s.level(0.9), Level::Marginal);
        assert_eq!(s.level(0.99), Level::Important);
        for selection in [
            Selection::Value(f64::NAN),
            Selection::Value(-1.0),
            Selection::TopN(usize::MAX),
        ] {
            assert!(
                Settings {
                    selection,
                    ..s.clone()
                }
                .validate()
                .is_err()
            );
        }
        assert!(Settings { high: 0.8, ..s }.validate().is_err());
    }

    #[test]
    fn extreme_correlation_tails_keep_the_observed_sign_asymmetry() {
        let xs: Vec<_> = (0..80)
            .map(|i| Value::real(if i % 2 == 0 { -1.0 } else { 1.0 }))
            .collect();
        let ys: Vec<_> = xs
            .iter()
            .enumerate()
            .map(|(i, x)| Value::real(2.0 * x.as_f64().unwrap() + (i % 5) as f64 * 0.2))
            .collect();
        let negative: Vec<_> = xs
            .iter()
            .map(|x| Value::real(-x.as_f64().unwrap()))
            .collect();
        let positive = correlation(&xs.iter().zip(&ys).collect::<Vec<_>>()).unwrap();
        let negative = correlation(&negative.iter().zip(&ys).collect::<Vec<_>>()).unwrap();
        assert_eq!(positive.p, 0.0);
        assert!(negative.p > 0.0 && negative.p < 1e-50);
        assert_eq!(positive.importance, negative.importance);
    }

    #[test]
    fn limits_nonfinite_data_and_unsupported_mixed_criteria_return_errors() {
        let mut target = Field::new("target", Storage::String);
        target.measurement = Measurement::Nominal;
        let mut x = Field::new("x", Storage::Real);
        x.measurement = Measurement::Continuous;
        let table = Table::new(
            DataModel::new(vec![target, x]).unwrap(),
            vec![vec![Value::string("a"), Value::Real(f64::INFINITY)]],
        )
        .unwrap();
        let data = Dataset {
            table: &table,
            target: 0,
            inputs: &[1],
            frequency: None,
        };
        assert!(train(&data, &settings_off()).is_err());
        assert!(matches!(
            train(
                &data,
                &Settings {
                    criterion: Criterion::CramersV,
                    ..settings_off()
                }
            ),
            Err(AlgoError::Unsupported(_))
        ));
        assert!(
            train(
                &Dataset {
                    inputs: &vec![1; 257],
                    ..data
                },
                &settings_off()
            )
            .is_err()
        );
    }

    #[test]
    fn screening_strict_boundaries_user_missing_and_empty_cases() {
        let mut target = Field::new("target", Storage::String);
        target.measurement = Measurement::Nominal;
        let mut input = Field::new("input", Storage::Real);
        input.measurement = Measurement::Continuous;
        let rows = vec![
            vec![Value::string("a"), Value::Real(-0.5)],
            vec![Value::string("b"), Value::Real(0.5)],
            vec![Value::Null, Value::Real(1e100)],
            vec![Value::string("a"), Value::Null],
        ];
        let table = Table::new(DataModel::new(vec![target, input]).unwrap(), rows).unwrap();
        let data = Dataset {
            table: &table,
            target: 0,
            inputs: &[1],
            frequency: None,
        };
        let s = Settings {
            min_sd: Some(0.5_f64.sqrt()),
            ..settings_off()
        };
        assert!(train(&data, &s).unwrap().screened.is_empty());
        assert_eq!(
            train(
                &data,
                &Settings {
                    min_sd: Some(0.71),
                    ..s
                }
            )
            .unwrap()
            .screened
            .len(),
            1
        );
        assert!(
            train(
                &Dataset {
                    target: usize::MAX,
                    ..data
                },
                &settings_off()
            )
            .is_err()
        );
        assert!(
            train(
                &Dataset {
                    inputs: &[usize::MAX],
                    ..data
                },
                &settings_off()
            )
            .is_err()
        );
        assert!(
            train(
                &Dataset {
                    inputs: &[1, 1],
                    ..data
                },
                &settings_off()
            )
            .is_err()
        );
        assert!(
            train(
                &Dataset {
                    inputs: &[],
                    ..data
                },
                &settings_off()
            )
            .is_err()
        );
    }
}
