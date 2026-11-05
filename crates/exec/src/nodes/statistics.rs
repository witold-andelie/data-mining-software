//! Statistics (Output palette): computes univariate summary statistics for each `examineFields`
//! field and a pairwise Pearson correlation matrix for `correlationFields`, returning a
//! [`Document`] whose tables match Modeler's JSON output (`docs/oracle.md`).
//!
//! Implemented settings (`docs/formats/nodes/statistics.md`):
//! - `examineFields/field`: univariate fields (in listed order; one table per field, titled with
//!   the field name, with `FORMAT_ROW_HEADER` rows in Modeler's fixed display order).
//! - `statistics/setElement`: subset of `{count, mean, sum, min, max, range, variance, sdev,
//!   semean, median, mode}` (default when absent: `count` and `mean`).
//! - `correlationFields/field`: pairwise Pearson correlation matrix (titled `Pearson Correlations`
//!   with header `[""] + fields` and one row per field).
//! - `correlationStrengthMode` (`probability` | `absolute`), `weakProbabilityThreshold`,
//!   `mediumProbabilityThreshold`, `weakCorrelationThreshold`, `mediumCorrelationThreshold`,
//!   `showCorrelationLabels`, `weakCorrelationLabel`, `mediumCorrelationLabel`,
//!   `strongCorrelationLabel`, `perfectCorrelationLabel`.
//! - `outputMode` (`screen`, `file`) with `outputFileType` (`text` `.txt`, `html` `.html`,
//!   `output_object` `.cou`) and `outputFilePath` (`oracle/outputs_more/statistics_file_*`).
//!   `Text` is the tables as tab-indented aligned text and `HTML` as `<dl>` pages (CRLF);
//!   the `.cou` output object (FastInfoset with a save timestamp) stays a `GAP:`.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::nodes::field_util::position;
use crate::operator::{Arity, Operator};
use crate::output::{Document, DocumentTable, NodeOutput, format_real};
use crate::settings;
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::{Field, Storage, Table, Value};
use std::cmp::Ordering;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatKind {
    Count,
    Mean,
    Sum,
    Min,
    Max,
    Range,
    Variance,
    StdDev,
    SeMean,
    Median,
    Mode,
}

/// Modeler's fixed display order for univariate statistics rows regardless of the order in which
/// `statistics/setElement` is saved.
const DISPLAY_ORDER: &[(StatKind, &str)] = &[
    (StatKind::Count, "Count"),
    (StatKind::Mean, "Mean"),
    (StatKind::Sum, "Sum"),
    (StatKind::Min, "Min"),
    (StatKind::Max, "Max"),
    (StatKind::Range, "Range"),
    (StatKind::Variance, "Variance"),
    (StatKind::StdDev, "Standard Deviation"),
    (StatKind::SeMean, "Standard Error of Mean"),
    (StatKind::Median, "Median"),
    (StatKind::Mode, "Mode"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StrengthMode {
    Probability,
    Absolute,
}

#[derive(Debug, Clone)]
struct StatisticsOp {
    examine_fields: Vec<String>,
    stats: Vec<StatKind>,
    correlation_fields: Vec<String>,
    strength_mode: StrengthMode,
    weak_prob_threshold: f64,
    medium_prob_threshold: f64,
    weak_corr_threshold: f64,
    medium_corr_threshold: f64,
    show_labels: bool,
    weak_label: String,
    medium_label: String,
    strong_label: String,
    perfect_label: String,
    file: Option<(String, FileKind)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileKind {
    Text,
    Html,
    Output,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();

    let file = match settings::text(s, "outputMode").as_deref().map(str::trim) {
        None | Some("") | Some("screen") => None,
        Some("file") => {
            let path = settings::text(s, "outputFilePath")
                .filter(|p| !p.trim().is_empty())
                .ok_or_else(|| {
                    OperatorError::Settings(
                        "Statistics outputFilePath is required in file mode".into(),
                    )
                })?;
            let kind = match settings::text(s, "outputFileType")
                .as_deref()
                .map(str::trim)
            {
                None | Some("") | Some("text") => FileKind::Text,
                Some("html") => FileKind::Html,
                Some("output_object") => FileKind::Output,
                Some(other) => {
                    return Err(OperatorError::Settings(format!(
                        "Statistics outputFileType {other:?} is unknown"
                    )));
                }
            };
            Some((path, kind))
        }
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Statistics outputMode {other:?} is not supported"
            )));
        }
    };

    let examine_fields = field_list(s, "examineFields");
    let correlation_fields = field_list(s, "correlationFields");
    let stats = parse_stats(s)?;

    let strength_mode = match settings::text(s, "correlationStrengthMode")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("probability") => StrengthMode::Probability,
        Some("absolute") => StrengthMode::Absolute,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Statistics correlationStrengthMode {other:?} is unknown"
            )));
        }
    };

    let weak_prob_threshold = settings::real(s, "weakProbabilityThreshold")?.unwrap_or(0.9);
    let medium_prob_threshold = settings::real(s, "mediumProbabilityThreshold")?.unwrap_or(0.95);
    let weak_corr_threshold = settings::real(s, "weakCorrelationThreshold")?.unwrap_or(0.3333);
    let medium_corr_threshold = settings::real(s, "mediumCorrelationThreshold")?.unwrap_or(0.6666);

    if !(0.0..=1.0).contains(&weak_prob_threshold)
        || !(0.0..=1.0).contains(&medium_prob_threshold)
        || weak_prob_threshold > medium_prob_threshold
    {
        return Err(OperatorError::Settings(format!(
            "Statistics probability thresholds ({weak_prob_threshold}, {medium_prob_threshold}) are invalid"
        )));
    }
    if !(0.0..=1.0).contains(&weak_corr_threshold)
        || !(0.0..=1.0).contains(&medium_corr_threshold)
        || weak_corr_threshold > medium_corr_threshold
    {
        return Err(OperatorError::Settings(format!(
            "Statistics correlation thresholds ({weak_corr_threshold}, {medium_corr_threshold}) are invalid"
        )));
    }

    let show_labels = settings::flag(s, "showCorrelationLabels")?.unwrap_or(true);
    let weak_label =
        settings::text(s, "weakCorrelationLabel").unwrap_or_else(|| "Weak".to_string());
    let medium_label =
        settings::text(s, "mediumCorrelationLabel").unwrap_or_else(|| "Medium".to_string());
    let strong_label =
        settings::text(s, "strongCorrelationLabel").unwrap_or_else(|| "Strong".to_string());
    let perfect_label =
        settings::text(s, "perfectCorrelationLabel").unwrap_or_else(|| "Perfect".to_string());

    Ok(Box::new(StatisticsOp {
        examine_fields,
        stats,
        correlation_fields,
        strength_mode,
        weak_prob_threshold,
        medium_prob_threshold,
        weak_corr_threshold,
        medium_corr_threshold,
        show_labels,
        weak_label,
        medium_label,
        strong_label,
        perfect_label,
        file,
    }))
}

impl Operator for StatisticsOp {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(
            "Statistics is a terminal output node and produces a Document, not a Table".into(),
        ))
    }

    fn produce(
        &self,
        inputs: &[Arc<Table>],
        _model: Option<Arc<dms_pmml::Pmml>>,
        context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Statistics needs one input".into()))?;
        let decimals = context.decimal_places();
        let mut tables = Vec::new();

        for name in &self.examine_fields {
            let col = position(input.model(), name)?;
            let field = input.model().fields().get(col).ok_or_else(|| {
                OperatorError::Data(format!("input has no field at position {col}"))
            })?;
            let rows = compute_univariate(input, col, field, &self.stats, decimals);
            tables.push(DocumentTable {
                title: Some(name.clone()),
                header: Vec::new(),
                rows,
            });
        }

        if !self.correlation_fields.is_empty() {
            let positions: Vec<(usize, &Field)> = self
                .correlation_fields
                .iter()
                .map(|name| {
                    let col = position(input.model(), name)?;
                    let field = input.model().fields().get(col).ok_or_else(|| {
                        OperatorError::Data(format!("input has no field at position {col}"))
                    })?;
                    Ok((col, field))
                })
                .collect::<Result<Vec<_>, OperatorError>>()?;

            let mut header = Vec::with_capacity(self.correlation_fields.len() + 1);
            header.push(String::new());
            header.extend(self.correlation_fields.iter().cloned());

            let mut rows = Vec::with_capacity(self.correlation_fields.len());
            for (i, (col_i, field_i)) in positions.iter().enumerate() {
                let mut row = Vec::with_capacity(self.correlation_fields.len() + 1);
                row.push(field_i.name.clone());
                for (j, (col_j, field_j)) in positions.iter().enumerate() {
                    let cell = if i == j {
                        self.format_self_correlation(*col_i, field_i, input, decimals)
                    } else {
                        self.format_pair_correlation(
                            *col_i, field_i, *col_j, field_j, input, decimals,
                        )
                    };
                    row.push(cell);
                }
                rows.push(row);
            }

            tables.push(DocumentTable {
                title: Some("Pearson Correlations".to_string()),
                header,
                rows,
            });
        }

        let doc = Document {
            title: String::new(),
            tables,
        };
        self.write_file(&doc, context)?;
        Ok(NodeOutput::Document(Arc::new(doc)))
    }
}

impl StatisticsOp {
    fn write_file(&self, doc: &Document, context: &ExecContext) -> Result<(), OperatorError> {
        let Some((path, kind)) = &self.file else {
            return Ok(());
        };
        if context.export_dir.is_none() {
            if matches!(kind, FileKind::Output) {
                return Err(OperatorError::Settings(
                    "GAP: Statistics: Output (.cou) file is FastInfoset with a save timestamp and has no byte-stable oracle"
                        .into(),
                ));
            }
            return Ok(());
        }
        match kind {
            FileKind::Output => Err(OperatorError::Settings(
                "GAP: Statistics: Output (.cou) file is FastInfoset with a save timestamp and has no byte-stable oracle"
                    .into(),
            )),
            FileKind::Text => {
                let text = statistics_text(doc);
                write_bytes(&context.export_path(path), text.as_bytes())?;
                Ok(())
            }
            FileKind::Html => {
                let html = statistics_html(doc);
                write_bytes(&context.export_path(path), html.as_bytes())?;
                Ok(())
            }
        }
    }
}

fn field_list(s: Option<&Element>, path: &str) -> Vec<String> {
    settings::children(s, path, "field")
        .into_iter()
        .map(Element::text)
        .filter(|name| !name.is_empty())
        .collect()
}

fn parse_stats(s: Option<&Element>) -> Result<Vec<StatKind>, OperatorError> {
    if settings::element(s, "statistics").is_none() {
        return Ok(vec![StatKind::Count, StatKind::Mean]);
    }
    let mut set = Vec::new();
    for el in settings::children(s, "statistics", "setElement") {
        let kind = match el.text().trim() {
            "count" => StatKind::Count,
            "mean" => StatKind::Mean,
            "sum" => StatKind::Sum,
            "min" => StatKind::Min,
            "max" => StatKind::Max,
            "range" => StatKind::Range,
            "variance" => StatKind::Variance,
            "sdev" => StatKind::StdDev,
            "semean" => StatKind::SeMean,
            "median" => StatKind::Median,
            "mode" => StatKind::Mode,
            other => {
                return Err(OperatorError::Settings(format!(
                    "Statistics unknown statistic {other:?}"
                )));
            }
        };
        if !set.contains(&kind) {
            set.push(kind);
        }
    }
    Ok(set)
}

fn compute_univariate(
    input: &Table,
    col: usize,
    field: &Field,
    selected: &[StatKind],
    decimals: usize,
) -> Vec<Vec<String>> {
    let mut values: Vec<f64> = Vec::new();
    let mut int_values: Vec<i64> = Vec::new();
    let is_int = field.storage == Storage::Integer;

    for row in input.rows() {
        let Some(v) = row.get(col) else {
            continue;
        };
        if field.is_missing(v) {
            continue;
        }
        if let Some(x) = v.as_f64()
            && x.is_finite()
        {
            values.push(x);
            if is_int && let Value::Integer(iv) = v {
                int_values.push(*iv);
            }
        }
    }

    let n = values.len();
    let n_f = n as f64;
    let sum: f64 = values.iter().sum();
    let int_sum: i128 = int_values.iter().map(|&v| i128::from(v)).sum();
    // Modeler updates the mean and the sum of squared deviations record by record (Welford):
    // the team/cluster_analysis Statistics means and variances match it to the last digit where
    // `sum / n` and the two-pass sum of squares do not.
    let (mut mean, mut squares) = (0.0f64, 0.0f64);
    for (k, &x) in values.iter().enumerate() {
        let delta = x - mean;
        mean += delta / (k + 1) as f64;
        squares += delta * (x - mean);
    }
    if n == 0 {
        mean = f64::NAN;
    }
    let variance = if n >= 2 {
        squares / (n_f - 1.0)
    } else {
        f64::NAN
    };
    let sdev = variance.sqrt();
    let semean = if n >= 2 {
        (squares / ((n_f - 1.0) * n_f)).sqrt()
    } else {
        f64::NAN
    };

    let mut sorted = values.clone();
    sorted.sort_by(f64::total_cmp);
    let mut sorted_ints = int_values;
    sorted_ints.sort_unstable();

    let mut rows = Vec::new();
    for (kind, label) in DISPLAY_ORDER {
        if !selected.contains(kind) {
            continue;
        }
        let formatted = match kind {
            StatKind::Count => n.to_string(),
            StatKind::Mean => format_real_or_null(mean, decimals),
            StatKind::Sum => {
                if n == 0 {
                    "$null$".to_string()
                } else if is_int && sorted_ints.len() == n {
                    int_sum.to_string()
                } else {
                    format_real_or_null(sum, decimals)
                }
            }
            StatKind::Min => {
                if n == 0 {
                    "$null$".to_string()
                } else if is_int && let Some(&v) = sorted_ints.first() {
                    v.to_string()
                } else {
                    format_real_or_null(sorted.first().copied().unwrap_or(f64::NAN), decimals)
                }
            }
            StatKind::Max => {
                if n == 0 {
                    "$null$".to_string()
                } else if is_int && let Some(&v) = sorted_ints.last() {
                    v.to_string()
                } else {
                    format_real_or_null(sorted.last().copied().unwrap_or(f64::NAN), decimals)
                }
            }
            StatKind::Range => {
                if n == 0 {
                    "$null$".to_string()
                } else if is_int
                    && let (Some(&lo), Some(&hi)) = (sorted_ints.first(), sorted_ints.last())
                {
                    (i128::from(hi) - i128::from(lo)).to_string()
                } else {
                    let lo = sorted.first().copied().unwrap_or(f64::NAN);
                    let hi = sorted.last().copied().unwrap_or(f64::NAN);
                    format_real_or_null(hi - lo, decimals)
                }
            }
            StatKind::Variance => format_real_or_null(variance, decimals),
            StatKind::StdDev => format_real_or_null(sdev, decimals),
            StatKind::SeMean => format_real_or_null(semean, decimals),
            StatKind::Median => {
                let med = median_sorted(&sorted);
                // An integer field's median prints as an integer when it is one
                // (outputs/statistics_correlations: `iv` median `25`).
                if is_int && med.is_finite() && med.fract() == 0.0 {
                    format!("{med:.0}")
                } else {
                    format_real_or_null(med, decimals)
                }
            }
            StatKind::Mode => format_mode(&sorted, &sorted_ints, is_int, decimals),
        };
        rows.push(vec![(*label).to_string(), formatted]);
    }
    rows
}

fn median_sorted(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return f64::NAN;
    }
    let mid = n / 2;
    if n.is_multiple_of(2) {
        let a = sorted.get(mid - 1).copied().unwrap_or(f64::NAN);
        let b = sorted.get(mid).copied().unwrap_or(f64::NAN);
        (a + b) / 2.0
    } else {
        sorted.get(mid).copied().unwrap_or(f64::NAN)
    }
}

fn format_mode(sorted: &[f64], sorted_ints: &[i64], is_int: bool, decimals: usize) -> String {
    if sorted.is_empty() {
        return "$null$".to_string();
    }
    if is_int && sorted_ints.len() == sorted.len() {
        let mut best_val: Option<i64> = None;
        let mut best_count: usize = 0;
        let mut tied = false;
        let mut i = 0;
        while i < sorted_ints.len() {
            let v = sorted_ints[i];
            let mut j = i + 1;
            while j < sorted_ints.len() && sorted_ints[j] == v {
                j += 1;
            }
            let count = j - i;
            match count.cmp(&best_count) {
                Ordering::Greater => {
                    best_count = count;
                    best_val = Some(v);
                    tied = false;
                }
                Ordering::Equal => {
                    tied = true;
                }
                Ordering::Less => {}
            }
            i = j;
        }
        return match best_val {
            Some(v) if tied => format!("{v}*"),
            Some(v) => v.to_string(),
            None => "$null$".to_string(),
        };
    }

    let mut best_val: Option<f64> = None;
    let mut best_count: usize = 0;
    let mut tied = false;
    let mut i = 0;
    while i < sorted.len() {
        let v = sorted[i];
        let mut j = i + 1;
        while j < sorted.len() && sorted[j].total_cmp(&v) == Ordering::Equal {
            j += 1;
        }
        let count = j - i;
        match count.cmp(&best_count) {
            Ordering::Greater => {
                best_count = count;
                best_val = Some(v);
                tied = false;
            }
            Ordering::Equal => {
                tied = true;
            }
            Ordering::Less => {}
        }
        i = j;
    }
    match best_val {
        Some(v) if tied => format!("{v:.decimals$}*"),
        Some(v) => format!("{v:.decimals$}"),
        None => "$null$".to_string(),
    }
}

impl StatisticsOp {
    fn format_self_correlation(
        &self,
        col: usize,
        field: &Field,
        input: &Table,
        decimals: usize,
    ) -> String {
        let mut vals = Vec::new();
        for row in input.rows() {
            if let Some(v) = row.get(col)
                && !field.is_missing(v)
                && let Some(x) = v.as_f64()
                && x.is_finite()
            {
                vals.push(x);
            }
        }
        if vals.len() < 2 {
            return "--".to_string();
        }
        let mean = vals.iter().sum::<f64>() / (vals.len() as f64);
        let ss: f64 = vals.iter().map(|&x| (x - mean) * (x - mean)).sum();
        if ss <= 0.0 {
            return "--".to_string();
        }
        self.format_r(1.0, vals.len(), decimals)
    }

    fn format_pair_correlation(
        &self,
        col_x: usize,
        field_x: &Field,
        col_y: usize,
        field_y: &Field,
        input: &Table,
        decimals: usize,
    ) -> String {
        let mut pairs: Vec<(f64, f64)> = Vec::new();
        for row in input.rows() {
            let (Some(vx), Some(vy)) = (row.get(col_x), row.get(col_y)) else {
                continue;
            };
            if field_x.is_missing(vx) || field_y.is_missing(vy) {
                continue;
            }
            if let (Some(x), Some(y)) = (vx.as_f64(), vy.as_f64())
                && x.is_finite()
                && y.is_finite()
            {
                pairs.push((x, y));
            }
        }
        let n = pairs.len();
        if n < 2 {
            return "--".to_string();
        }
        let n_f = n as f64;
        let mx = pairs.iter().map(|(x, _)| *x).sum::<f64>() / n_f;
        let my = pairs.iter().map(|(_, y)| *y).sum::<f64>() / n_f;
        let mut sxx = 0.0;
        let mut syy = 0.0;
        let mut sxy = 0.0;
        for &(x, y) in &pairs {
            let dx = x - mx;
            let dy = y - my;
            sxx += dx * dx;
            syy += dy * dy;
            sxy += dx * dy;
        }
        let denom = (sxx * syy).sqrt();
        if denom <= 0.0 || !denom.is_finite() {
            return "--".to_string();
        }
        let r = (sxy / denom).clamp(-1.0, 1.0);
        self.format_r(r, n, decimals)
    }

    fn format_r(&self, r: f64, n: usize, decimals: usize) -> String {
        let num = format!("{r:.decimals$}");
        if !self.show_labels {
            return num;
        }
        let label = self.strength_label(r, n);
        format!("{num}/{label}")
    }

    fn strength_label(&self, r: f64, n: usize) -> &str {
        let abs_r = r.abs();
        if (1.0 - abs_r) <= 1e-12 {
            return &self.perfect_label;
        }
        match self.strength_mode {
            StrengthMode::Absolute => {
                if abs_r < self.weak_corr_threshold {
                    &self.weak_label
                } else if abs_r < self.medium_corr_threshold {
                    &self.medium_label
                } else {
                    &self.strong_label
                }
            }
            StrengthMode::Probability => {
                // Modeler's Importance / Correlation probability: 1 - p, where p is the two-tailed
                // t-test p-value with df = n - 2 and t^2 = (n - 2) * r^2 / (1 - r^2).
                // Note: Modeler's probability mode compares against weak_corr_threshold and
                // medium_corr_threshold when saved in team/cluster_analysis.str or against
                // probability thresholds? Wait: let's check cluster_analysis!
                let importance = correlation_importance(r, n);
                if importance < self.weak_prob_threshold {
                    &self.weak_label
                } else if importance < self.medium_prob_threshold {
                    &self.medium_label
                } else {
                    &self.strong_label
                }
            }
        }
    }
}

fn format_real_or_null(value: f64, decimals: usize) -> String {
    format_real(value, decimals)
}

/// Computes `1.0 - p` for a Pearson correlation `r` on `n` pairs, where `p` is the two-tailed
/// Student-t significance with `df = n - 2`.
pub(crate) fn correlation_importance(r: f64, n: usize) -> f64 {
    if n <= 2 {
        return 0.0;
    }
    let r2 = r * r;
    if r2 >= 1.0 {
        return 1.0;
    }
    let df = (n - 2) as f64;
    // x = df / (df + t^2) = 1 - r^2
    let x = (1.0 - r2).clamp(0.0, 1.0);
    let p = regularized_incomplete_beta(0.5 * df, 0.5, x);
    (1.0 - p).clamp(0.0, 1.0)
}

/// Regularized incomplete beta function $I_x(a, b)$ using Lentz's continued fraction and Lanczos
/// log-gamma.
pub(crate) fn regularized_incomplete_beta(a: f64, b: f64, x: f64) -> f64 {
    if !x.is_finite() || !a.is_finite() || !b.is_finite() || a <= 0.0 || b <= 0.0 {
        return f64::NAN;
    }
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let ln_beta = ln_gamma(a) + ln_gamma(b) - ln_gamma(a + b);
    let front = (a * x.ln() + b * (1.0 - x).ln() - ln_beta).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        (front * beta_cf(a, b, x) / a).clamp(0.0, 1.0)
    } else {
        (1.0 - front * beta_cf(b, a, 1.0 - x) / b).clamp(0.0, 1.0)
    }
}

fn beta_cf(a: f64, b: f64, x: f64) -> f64 {
    const MAX_ITER: usize = 200;
    const EPS: f64 = 3.0e-15;
    const FPMIN: f64 = 1.0e-300;

    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < FPMIN {
        d = FPMIN;
    }
    d = 1.0 / d;
    let mut h = d;

    for m in 1..=MAX_ITER {
        let m_f = m as f64;
        let m2 = 2.0 * m_f;
        let aa1 = m_f * (b - m_f) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa1 * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa1 / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        h *= d * c;

        let aa2 = -(a + m_f) * (qab + m_f) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa2 * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa2 / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < EPS {
            break;
        }
    }
    h
}

#[allow(clippy::excessive_precision)]
fn ln_gamma(z: f64) -> f64 {
    const COEFFS: [f64; 9] = [
        0.999_999_999_999_809_93,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_13,
        -176.615_029_162_140_59,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_571_6e-6,
        1.505_632_735_149_311_6e-7,
    ];
    let x = z - 1.0;
    let mut sum = COEFFS[0];
    for (i, &c) in COEFFS.iter().enumerate().skip(1) {
        sum += c / (x + (i as f64));
    }
    let t = x + 7.5;
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + sum.ln()
}

fn write_bytes(path: &std::path::Path, bytes: &[u8]) -> Result<(), OperatorError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| OperatorError::Io(format!("{}: {e}", parent.display())))?;
    }
    std::fs::write(path, bytes)
        .map_err(|e| OperatorError::Io(format!("{}: {e}", path.display())))?;
    Ok(())
}

fn statistics_text(doc: &Document) -> String {
    let mut out = String::new();
    for table in &doc.tables {
        let Some(title) = &table.title else {
            continue;
        };
        if title == "Pearson Correlations" {
            out.push_str("Pearson Correlations\r\n");
            // Header: empty first cell plus field names, right-aligned to column widths.
            let ncols = table.header.len();
            let mut widths = vec![0usize; ncols];
            widths[0] = table.rows.iter().map(|r| r[0].len()).max().unwrap_or(0);
            for (ci, width) in widths.iter_mut().enumerate().skip(1) {
                let mut w = table.header.get(ci).map(|h| h.len()).unwrap_or(0);
                for row in &table.rows {
                    if let Some(cell) = row.get(ci) {
                        w = w.max(cell.len());
                    }
                }
                *width = w;
            }
            out.push('\t');
            for (ci, head) in table.header.iter().enumerate() {
                if ci == 0 {
                    out.push_str(&" ".repeat(widths[0] + 12));
                } else {
                    if ci > 1 {
                        out.push_str("   ");
                    }
                    out.push_str(&" ".repeat(widths[ci].saturating_sub(head.len())));
                    out.push_str(head);
                }
            }
            out.push_str("\r\n");
            for row in &table.rows {
                out.push('\t');
                out.push_str(&row[0]);
                out.push_str(&" ".repeat(widths[0] - row[0].len() + 12));
                for (ci, cell) in row.iter().enumerate().skip(1) {
                    if ci > 1 {
                        out.push_str("   ");
                    }
                    out.push_str(&" ".repeat(widths[ci].saturating_sub(cell.len())));
                    out.push_str(cell);
                }
                out.push_str("\r\n");
            }
            out.push_str("\r\n");
            continue;
        }
        // Univariate table: title, "Statistics" sub-title, then label/value rows.
        out.push_str(title);
        out.push_str("\r\n");
        out.push_str("\tStatistics\r\n");
        let label_width = table.rows.iter().map(|r| r[0].len()).max().unwrap_or(0);
        let value_width = table.rows.iter().map(|r| r[1].len()).max().unwrap_or(0);
        for row in &table.rows {
            out.push_str("\t\t");
            out.push_str(&row[0]);
            out.push_str(&" ".repeat(label_width - row[0].len() + 12));
            out.push_str(&" ".repeat(value_width.saturating_sub(row[1].len())));
            out.push_str(&row[1]);
            out.push_str("\r\n");
        }
        out.push_str("\r\n");
        // Footnote for tied modes, as Modeler writes it.
        if table.rows.iter().any(|r| r[1].ends_with('*')) {
            out.push_str("\t\t*Multiple modes exist.  The smallest value is shown.\r\n");
        }
    }
    out
}

fn statistics_html(doc: &Document) -> String {
    let mut lines = vec![
        "<HTML><HEAD>".to_string(),
        "<META HTTP-EQUIV=\"Cache-Control\" CONTENT=\"no-cache\">".to_string(),
        "<META HTTP-EQUIV=\"Pragma\" CONTENT=\"no-cache\">".to_string(),
        "<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html;charset=UTF-8\">".to_string(),
        "</HEAD>".to_string(),
        "<dl>".to_string(),
    ];
    for table in &doc.tables {
        let Some(title) = &table.title else {
            continue;
        };
        if title == "Pearson Correlations" {
            lines.push(format!("<dt>{title}</dt>"));
            lines.push("<dd><dl>".to_string());
            let ncols = table.header.len();
            let mut widths = vec![0usize; ncols];
            widths[0] = table.rows.iter().map(|r| r[0].len()).max().unwrap_or(0);
            for (ci, width) in widths.iter_mut().enumerate().skip(1) {
                let mut w = table.header.get(ci).map(|h| h.len()).unwrap_or(0);
                for row in &table.rows {
                    if let Some(cell) = row.get(ci) {
                        w = w.max(cell.len());
                    }
                }
                *width = w;
            }
            let mut header_line = String::new();
            for (ci, head) in table.header.iter().enumerate() {
                if ci == 0 {
                    header_line.push_str(&" ".repeat(widths[0] + 12));
                } else {
                    if ci > 1 {
                        header_line.push_str("   ");
                    }
                    header_line.push_str(&" ".repeat(widths[ci].saturating_sub(head.len())));
                    header_line.push_str(head);
                }
            }
            lines.push(format!("<pre>{header_line}"));
            for row in &table.rows {
                let mut line = String::new();
                line.push_str(&row[0]);
                line.push_str(&" ".repeat(widths[0] - row[0].len() + 12));
                for (ci, cell) in row.iter().enumerate().skip(1) {
                    if ci > 1 {
                        line.push_str("   ");
                    }
                    line.push_str(&" ".repeat(widths[ci].saturating_sub(cell.len())));
                    line.push_str(cell);
                }
                lines.push(line);
            }
            lines.push("</pre>".to_string());
            lines.push("</dl>".to_string());
            lines.push("</dd>".to_string());
            continue;
        }
        lines.push(format!("<dt>{title}</dt>"));
        lines.push("<dd><dl>".to_string());
        lines.push("<dt>Statistics</dt>".to_string());
        lines.push("<dd><dl>".to_string());
        let label_width = table.rows.iter().map(|r| r[0].len()).max().unwrap_or(0);
        let value_width = table.rows.iter().map(|r| r[1].len()).max().unwrap_or(0);
        for (ri, row) in table.rows.iter().enumerate() {
            let mut line = String::new();
            line.push_str(&row[0]);
            line.push_str(&" ".repeat(label_width - row[0].len() + 12));
            line.push_str(&" ".repeat(value_width.saturating_sub(row[1].len())));
            line.push_str(&row[1]);
            if ri == 0 {
                lines.push(format!("<pre>{line}"));
            } else {
                lines.push(line);
            }
        }
        lines.push("</pre>".to_string());
        if table.rows.iter().any(|r| r[1].ends_with('*')) {
            lines.push("<dt>*Multiple modes exist.  The smallest value is shown.</dt>".to_string());
        }
        lines.push("</dl>".to_string());
        lines.push("</dd>".to_string());
        lines.push("</dl>".to_string());
        lines.push("</dd>".to_string());
    }
    lines.push("</dl>".to_string());
    lines.push("</HTML>".to_string());
    lines.join("\r\n")
}
