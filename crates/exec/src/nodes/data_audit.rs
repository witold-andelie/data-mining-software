//! Data Audit (Output palette, `DataAuditNode`): produces a [`Document`] with two tables matching
//! Modeler's exported CSV (`docs/oracle.md`, `docs/formats/nodes/dataaudit.md`):
//!
//! 1. **Audit**: one row per audited field with `Field`, `Measurement`, the chosen statistics in
//!    Modeler's display order, `Unique` and `Valid`.
//! 2. **Quality**: one row per audited field with `Field`, `Measurement`, `Outliers`, `Extremes`,
//!    `Action`, `Impute Missing`, `Method`, `% Complete` (3 decimals), `Valid Records` and, with
//!    `breakdown`, `Null Value`, `Empty String`, `White Space`, `Blank Value`.
//!
//! Settings (saved names, as in every oracle stream): `dataAuditUseCustomFields` with
//! `dataAuditFields/field` (else all fields, with the Type node's target as overlay),
//! `dataAuditUserSpecifiedOverlayField`, `dataAuditBasicStatistics` (must be true),
//! `dataAuditAdvancedStatistics`, `dataAuditMedianAndMode`, `qualityCalculateValues`
//! (`records`, `breakdown`), `qualityInvalidValues` (all four kinds), `dataAuditOutlierDetectionMethod`
//! (`std`), `dataAuditStdOutlier`, `dataAuditStdExtreme`. Graphs and sampling settings only affect
//! the on-screen graphs and are ignored; other choices fail with [`OperatorError::Settings`].

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::nodes::field_util::position;
use crate::nodes::statistics::regularized_incomplete_beta;
use crate::operator::{Arity, Operator};
use crate::output::{Document, DocumentTable, NodeOutput};
use crate::settings;
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::{Field, Measurement, Role, Storage, Table, Value, calendar};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatCol {
    Min,
    Max,
    Sum,
    Range,
    Mean,
    SeMean,
    Correlation,
    CorrelationT,
    CorrelationTDf,
    CorrelationTSig,
    Covariance,
    StdDev,
    Variance,
    Skewness,
    SeSkewness,
    Kurtosis,
    SeKurtosis,
    Median,
    Mode,
}

const STAT_DISPLAY_ORDER: &[(StatCol, &str)] = &[
    (StatCol::Min, "Min"),
    (StatCol::Max, "Max"),
    (StatCol::Sum, "Sum"),
    (StatCol::Range, "Range"),
    (StatCol::Mean, "Mean"),
    (StatCol::SeMean, "Mean Std. Err."),
    (StatCol::Correlation, "Correlation"),
    (StatCol::CorrelationT, "Correlation T"),
    (StatCol::CorrelationTDf, "Correlation T df."),
    (StatCol::CorrelationTSig, "Correlation T sig."),
    (StatCol::Covariance, "Covariance"),
    (StatCol::StdDev, "Std. Dev"),
    (StatCol::Variance, "Variance"),
    (StatCol::Skewness, "Skewness"),
    (StatCol::SeSkewness, "Skewness Std. Err."),
    (StatCol::Kurtosis, "Kurtosis"),
    (StatCol::SeKurtosis, "Kurtosis Std. Err."),
    (StatCol::Median, "Median"),
    (StatCol::Mode, "Mode"),
];

const BASIC_STATS: &[StatCol] = &[
    StatCol::Min,
    StatCol::Max,
    StatCol::Mean,
    StatCol::StdDev,
    StatCol::Skewness,
];

/// Added by `dataAuditAdvancedStatistics`.
const ADVANCED_STATS: [StatCol; 7] = [
    StatCol::Sum,
    StatCol::Range,
    StatCol::SeMean,
    StatCol::Variance,
    StatCol::SeSkewness,
    StatCol::Kurtosis,
    StatCol::SeKurtosis,
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum FieldMode {
    All,
    Selected(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum OverlayMode {
    None,
    Target,
    Selected(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QualityBreakCol {
    Null,
    Empty,
    Whitespace,
    Blank,
}

const QUALITY_BREAK_ORDER: &[(QualityBreakCol, &str)] = &[
    (QualityBreakCol::Null, "Null Value"),
    (QualityBreakCol::Empty, "Empty String"),
    (QualityBreakCol::Whitespace, "White Space"),
    (QualityBreakCol::Blank, "Blank Value"),
];

#[derive(Debug, Clone)]
struct DataAuditOp {
    field_mode: FieldMode,
    overlay_mode: OverlayMode,
    stats: Vec<StatCol>,
    display_stats: bool,
    include_quality: bool,
    quality_breakdown: bool,
    quality_cols: Vec<QualityBreakCol>,
    std_outlier: f64,
    std_extreme: f64,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let choice = |name: &str| settings::text(s, name).map(|t| t.trim().to_string());

    if let Some(mode) = choice("outputMode").filter(|m| !m.is_empty() && m != "screen") {
        return Err(OperatorError::Settings(format!(
            "Data Audit outputMode {mode:?} is not supported"
        )));
    }

    let field_mode = if settings::flag(s, "dataAuditUseCustomFields")?.unwrap_or(false) {
        FieldMode::Selected(
            settings::children(s, "dataAuditFields", "field")
                .into_iter()
                .map(Element::text)
                .filter(|name| !name.is_empty())
                .collect(),
        )
    } else {
        FieldMode::All
    };
    let overlay_mode = match choice("dataAuditUserSpecifiedOverlayField").filter(|f| !f.is_empty())
    {
        Some(field) if field_mode != FieldMode::All => OverlayMode::Selected(field),
        Some(_) | None if field_mode == FieldMode::All => OverlayMode::Target,
        _ => OverlayMode::None,
    };

    // The statistics columns: the basic set, plus the advanced set and median/mode
    // (docs/oracle.md, outputs/dataaudit_full_stats).
    if !settings::flag(s, "dataAuditBasicStatistics")?.unwrap_or(true) {
        return Err(OperatorError::Settings(
            "Data Audit without basic statistics is not supported yet".into(),
        ));
    }
    let mut chosen = BASIC_STATS.to_vec();
    if settings::flag(s, "dataAuditAdvancedStatistics")?.unwrap_or(false) {
        chosen.extend(ADVANCED_STATS);
    }
    if settings::flag(s, "dataAuditMedianAndMode")?.unwrap_or(false) {
        chosen.extend([StatCol::Median, StatCol::Mode]);
    }
    let stats = STAT_DISPLAY_ORDER
        .iter()
        .filter_map(|(stat, _)| chosen.contains(stat).then_some(*stat))
        .collect();

    // Quality: `records` is the valid-record count, `breakdown` one column per invalid kind.
    let mut quality_breakdown = false;
    let mut records = false;
    for item in settings::children(s, "qualityCalculateValues", "setElement") {
        match item.text().trim() {
            "breakdown" => quality_breakdown = true,
            "records" => records = true,
            other => {
                return Err(OperatorError::Settings(format!(
                    "Data Audit qualityCalculateValues item {other:?} is not supported yet"
                )));
            }
        }
    }
    if !records && settings::element(s, "qualityCalculateValues").is_some() {
        return Err(OperatorError::Settings(
            "Data Audit without the valid-record count is not supported yet".into(),
        ));
    }
    let mut invalid = Vec::new();
    if settings::element(s, "qualityInvalidValues").is_none() {
        invalid = QUALITY_BREAK_ORDER.iter().map(|(kind, _)| *kind).collect();
    }
    for item in settings::children(s, "qualityInvalidValues", "setElement") {
        invalid.push(match item.text().trim() {
            "null" => QualityBreakCol::Null,
            "empty" => QualityBreakCol::Empty,
            "whitespace" => QualityBreakCol::Whitespace,
            "blank" => QualityBreakCol::Blank,
            other => {
                return Err(OperatorError::Settings(format!(
                    "Data Audit qualityInvalidValues item {other:?} is unknown"
                )));
            }
        });
    }
    if invalid.len() != QUALITY_BREAK_ORDER.len() {
        return Err(OperatorError::Settings(
            "Data Audit with only some invalid-value kinds is not supported yet".into(),
        ));
    }
    let quality_cols = QUALITY_BREAK_ORDER.iter().map(|(kind, _)| *kind).collect();

    if let Some(method) =
        choice("dataAuditOutlierDetectionMethod").filter(|m| !m.is_empty() && m != "std")
    {
        return Err(OperatorError::Settings(format!(
            "Data Audit outlier detection {method:?} is not supported yet"
        )));
    }
    let std_outlier = settings::real(s, "dataAuditStdOutlier")?.unwrap_or(3.0);
    let std_extreme = settings::real(s, "dataAuditStdExtreme")?.unwrap_or(5.0);
    if std_outlier <= 0.0 || std_extreme <= 0.0 || std_outlier > std_extreme {
        return Err(OperatorError::Settings(format!(
            "Data Audit outlier/extreme thresholds ({std_outlier}, {std_extreme}) are invalid"
        )));
    }

    Ok(Box::new(DataAuditOp {
        field_mode,
        overlay_mode,
        stats,
        display_stats: true,
        include_quality: true,
        quality_breakdown,
        quality_cols,
        std_outlier,
        std_extreme,
    }))
}

impl Operator for DataAuditOp {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(
            "Data Audit is a terminal output node and produces a Document, not a Table".into(),
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
            .ok_or_else(|| OperatorError::Inputs("Data Audit needs one input".into()))?;
        let decimals = context.decimal_places();

        let audited_cols = self.resolve_audited_fields(input)?;
        let overlay = self.resolve_overlay(input, &audited_cols)?;
        let has_continuous_overlay = overlay
            .and_then(|col| input.model().fields().get(col))
            .is_some_and(|f| f.measurement == Measurement::Continuous);

        let mut tables = Vec::new();

        if self.display_stats {
            let active_stats: Vec<(StatCol, &'static str)> = STAT_DISPLAY_ORDER
                .iter()
                .copied()
                .filter(|(col, _)| {
                    // A continuous overlay adds its correlation columns
                    // (team/linear_regression).
                    if matches!(
                        col,
                        StatCol::Correlation
                            | StatCol::CorrelationT
                            | StatCol::CorrelationTDf
                            | StatCol::CorrelationTSig
                            | StatCol::Covariance
                    ) {
                        return has_continuous_overlay;
                    }
                    self.stats.contains(col)
                })
                .collect();

            let mut header = Vec::with_capacity(2 + active_stats.len() + 2);
            header.push("Field".to_string());
            header.push("Measurement".to_string());
            for (_, label) in &active_stats {
                header.push((*label).to_string());
            }
            header.push("Unique".to_string());
            header.push("Valid".to_string());

            let mut rows = Vec::with_capacity(audited_cols.len());
            for &col in &audited_cols {
                let field = input.model().fields().get(col).ok_or_else(|| {
                    OperatorError::Data(format!("input has no field at position {col}"))
                })?;
                rows.push(compute_audit_row(
                    input,
                    col,
                    field,
                    overlay,
                    &active_stats,
                    decimals,
                ));
            }

            tables.push(DocumentTable {
                title: Some("Audit".to_string()),
                header,
                rows,
            });
        }

        if self.include_quality {
            let active_breaks: &[QualityBreakCol] = if self.quality_breakdown {
                &self.quality_cols
            } else {
                &[]
            };

            let mut header = Vec::with_capacity(9 + active_breaks.len());
            header.push("Field".to_string());
            header.push("Measurement".to_string());
            header.push("Outliers".to_string());
            header.push("Extremes".to_string());
            header.push("Action".to_string());
            header.push("Impute Missing".to_string());
            header.push("Method".to_string());
            header.push("% Complete".to_string());
            header.push("Valid Records".to_string());
            for b in active_breaks {
                let label = QUALITY_BREAK_ORDER
                    .iter()
                    .find_map(|(k, l)| (*k == *b).then_some(*l))
                    .unwrap_or("");
                header.push(label.to_string());
            }

            let mut rows = Vec::with_capacity(audited_cols.len());
            for &col in &audited_cols {
                let field = input.model().fields().get(col).ok_or_else(|| {
                    OperatorError::Data(format!("input has no field at position {col}"))
                })?;
                rows.push(self.compute_quality_row(input, col, field, active_breaks));
            }

            tables.push(DocumentTable {
                title: Some("Quality".to_string()),
                header,
                rows,
            });
        }

        Ok(NodeOutput::Document(Arc::new(Document {
            title: String::new(),
            tables,
        })))
    }
}

impl DataAuditOp {
    fn resolve_audited_fields(&self, input: &Table) -> Result<Vec<usize>, OperatorError> {
        let model = input.model();
        match &self.field_mode {
            FieldMode::All => Ok((0..model.len())
                .filter(|&i| {
                    model
                        .fields()
                        .get(i)
                        // Fields whose role is None are not audited (outputs/dataaudit_default:
                        // `id`, `$R-target`); record ids are (team/cluster_analysis `CUSTOMERID`).
                        .is_some_and(|f| f.role != Role::None)
                })
                .collect()),
            FieldMode::Selected(names) => names.iter().map(|name| position(model, name)).collect(),
        }
    }

    fn resolve_overlay(
        &self,
        input: &Table,
        audited_cols: &[usize],
    ) -> Result<Option<usize>, OperatorError> {
        let model = input.model();
        match &self.overlay_mode {
            OverlayMode::None => Ok(None),
            OverlayMode::Selected(name) => position(model, name).map(Some),
            OverlayMode::Target => {
                // Find the target field among audited fields (or all fields).
                let target = audited_cols
                    .iter()
                    .copied()
                    .find(|&i| {
                        model
                            .fields()
                            .get(i)
                            .is_some_and(|f| f.role == Role::Target)
                    })
                    .or_else(|| {
                        (0..model.len()).find(|&i| {
                            model
                                .fields()
                                .get(i)
                                .is_some_and(|f| f.role == Role::Target)
                        })
                    });
                Ok(target)
            }
        }
    }

    fn compute_quality_row(
        &self,
        input: &Table,
        col: usize,
        field: &Field,
        active_breaks: &[QualityBreakCol],
    ) -> Vec<String> {
        let field = &instantiated(field);
        let total_records = input.row_count();
        let mut valid_records = 0usize;
        let mut null_count = 0usize;
        let mut empty_count = 0usize;
        let mut whitespace_count = 0usize;
        let mut blank_count = 0usize;
        let mut cont_values = Vec::new();

        for row in input.rows() {
            let Some(v) = row.get(col) else {
                continue;
            };
            let is_sys_null = v.is_null();
            let is_empty_str = matches!(v, Value::String(s) if s.is_empty());
            let is_ws_str = matches!(v, Value::String(s) if !s.is_empty() && s.chars().all(char::is_whitespace));
            let is_user_blank = field.missing.is_blank(v);

            // Modeler's Data Audit quality rules:
            // - System null (`$null$`), empty string (`""`), whitespace-only strings, and
            //   Type-node user-defined blanks are all counted as missing (not valid).
            // - Breakdown columns are NOT mutually exclusive: e.g. `""` increments Empty String
            //   and White Space, and also Blank Value if `""` is in `field.missing`.
            if is_sys_null {
                null_count += 1;
            }
            if is_empty_str {
                empty_count += 1;
                whitespace_count += 1;
            } else if is_ws_str {
                whitespace_count += 1;
            }
            if is_user_blank {
                blank_count += 1;
            }

            let is_missing = is_sys_null || is_empty_str || is_ws_str || is_user_blank;
            if !is_missing {
                valid_records += 1;
                if field.measurement == Measurement::Continuous
                    && let Some(x) = continuous_value_as_f64(v)
                {
                    cont_values.push(x);
                }
            }
        }

        let (outliers_str, extremes_str, action_str) =
            if field.measurement == Measurement::Continuous {
                let (outliers, extremes) =
                    count_outliers_and_extremes(&cont_values, self.std_outlier, self.std_extreme);
                (
                    outliers.to_string(),
                    extremes.to_string(),
                    "None".to_string(),
                )
            } else {
                ("--".to_string(), "--".to_string(), "--".to_string())
            };

        let pct_complete = if total_records == 0 {
            0.0
        } else {
            (valid_records as f64) * 100.0 / (total_records as f64)
        };

        let mut out = Vec::with_capacity(9 + active_breaks.len());
        out.push(field.name.clone());
        out.push(measurement_label(field.measurement).to_string());
        out.push(outliers_str);
        out.push(extremes_str);
        out.push(action_str);
        out.push("Never".to_string());
        out.push("Fixed".to_string());
        out.push(format!("{pct_complete:.3}"));
        out.push(valid_records.to_string());

        for b in active_breaks {
            let c = match b {
                QualityBreakCol::Null => null_count,
                QualityBreakCol::Empty => empty_count,
                QualityBreakCol::Whitespace => whitespace_count,
                QualityBreakCol::Blank => blank_count,
            };
            out.push(c.to_string());
        }
        out
    }
}

fn count_outliers_and_extremes(
    values: &[f64],
    std_outlier: f64,
    std_extreme: f64,
) -> (usize, usize) {
    let n = values.len();
    if n < 2 {
        return (0, 0);
    }
    let n_f = n as f64;
    let mean = values.iter().sum::<f64>() / n_f;
    let ss: f64 = values
        .iter()
        .map(|&x| {
            let d = x - mean;
            d * d
        })
        .sum();
    let sdev = (ss / (n_f - 1.0)).sqrt();
    if sdev <= 0.0 || !sdev.is_finite() {
        return (0, 0);
    }
    let out_thresh = std_outlier * sdev;
    let ext_thresh = std_extreme * sdev;
    let mut outliers = 0usize;
    let mut extremes = 0usize;
    for &x in values {
        let dist = (x - mean).abs();
        if dist >= ext_thresh {
            extremes += 1;
        } else if dist >= out_thresh {
            outliers += 1;
        }
    }
    (outliers, extremes)
}

fn compute_audit_row(
    input: &Table,
    col: usize,
    field: &Field,
    overlay: Option<usize>,
    active_stats: &[(StatCol, &'static str)],
    decimals: usize,
) -> Vec<String> {
    let field = &instantiated(field);
    let is_continuous = field.measurement == Measurement::Continuous;
    let is_int_storage = field.storage == Storage::Integer;
    let is_numeric_storage = matches!(field.storage, Storage::Integer | Storage::Real);
    // Categorical numbers show their range (team/decision_trees_heart flags), except a record id
    // (team/cluster_analysis `CUSTOMERID`).
    let ordered_number = is_numeric_storage && field.role != Role::RecordId;

    let mut valid_count = 0usize;
    let mut cont_vals: Vec<f64> = Vec::new();
    let mut int_vals: Vec<i64> = Vec::new();
    let mut non_cont_vals: Vec<Value> = Vec::new();

    for row in input.rows() {
        let Some(v) = row.get(col) else {
            continue;
        };
        if is_audit_missing(field, v) {
            continue;
        }
        valid_count += 1;
        if is_continuous {
            if let Some(x) = continuous_value_as_f64(v) {
                cont_vals.push(x);
                if is_int_storage && let Value::Integer(iv) = v {
                    int_vals.push(*iv);
                }
            }
        } else {
            non_cont_vals.push(v.clone());
            if is_numeric_storage && let Some(x) = v.as_f64().filter(|x| x.is_finite()) {
                cont_vals.push(x);
                if is_int_storage && let Value::Integer(iv) = v {
                    int_vals.push(*iv);
                }
            }
        }
    }

    let n = cont_vals.len();
    let n_f = n as f64;
    let sum: f64 = cont_vals.iter().sum();
    let int_sum: i128 = int_vals.iter().map(|&v| i128::from(v)).sum();
    let mean = if n > 0 { sum / n_f } else { f64::NAN };

    let mut m2 = 0.0f64;
    let mut m3 = 0.0f64;
    let mut m4 = 0.0f64;
    if n >= 2 {
        for &v in &cont_vals {
            let d = v - mean;
            let d2 = d * d;
            m2 += d2;
            m3 += d2 * d;
            m4 += d2 * d2;
        }
    }
    let variance = if n >= 2 { m2 / (n_f - 1.0) } else { f64::NAN };
    let sdev = variance.sqrt();
    let semean = if n >= 2 { sdev / n_f.sqrt() } else { f64::NAN };

    let (skewness, seskewness) = if n >= 3 && sdev > 0.0 && sdev.is_finite() {
        let g1 = (n_f / ((n_f - 1.0) * (n_f - 2.0))) * (m3 / (sdev * sdev * sdev));
        let se = ((6.0 * n_f * (n_f - 1.0)) / ((n_f - 2.0) * (n_f + 1.0) * (n_f + 3.0))).sqrt();
        (g1, se)
    } else {
        (f64::NAN, f64::NAN)
    };

    let (kurtosis, sekurtosis) = if n >= 4 && sdev > 0.0 && sdev.is_finite() {
        let s4 = variance * variance;
        let term1 = (n_f * (n_f + 1.0)) / ((n_f - 1.0) * (n_f - 2.0) * (n_f - 3.0)) * (m4 / s4);
        let term2 = (3.0 * (n_f - 1.0) * (n_f - 1.0)) / ((n_f - 2.0) * (n_f - 3.0));
        let g2 = term1 - term2;
        let se = (4.0 * (n_f * n_f - 1.0) * seskewness * seskewness / ((n_f - 3.0) * (n_f + 5.0)))
            .sqrt();
        (g2, se)
    } else {
        (f64::NAN, f64::NAN)
    };

    let mut sorted_cont = cont_vals;
    sorted_cont.sort_by(f64::total_cmp);
    let mut sorted_ints = int_vals;
    sorted_ints.sort_unstable();

    let corr_stats = if is_continuous
        && let Some(ov_col) = overlay
        && ov_col != col
    {
        input
            .model()
            .fields()
            .get(ov_col)
            .filter(|ov_field| ov_field.measurement == Measurement::Continuous)
            .and_then(|ov_field| compute_pair_correlation(input, col, field, ov_col, ov_field))
    } else {
        None
    };

    let mut out = Vec::with_capacity(2 + active_stats.len() + 2);
    out.push(field.name.clone());
    out.push(measurement_label(field.measurement).to_string());

    // A record id shows no unique count (team/cluster_analysis `CUSTOMERID`).
    let record_id = field.role == Role::RecordId;
    // Dates and times show their range as values and no moments (team/stream_2_M).
    let temporal = matches!(
        field.storage,
        Storage::Date | Storage::Time | Storage::Timestamp
    );
    let valid_values = || {
        input
            .rows()
            .iter()
            .filter_map(|row| row.get(col))
            .filter(|v| !is_audit_missing(field, v))
    };

    for &(stat, _) in active_stats {
        let cell = match stat {
            StatCol::Min if temporal => valid_values()
                .min_by(|a, b| a.total_cmp(b))
                .map_or_else(|| "--".to_string(), ToString::to_string),
            StatCol::Max if temporal => valid_values()
                .max_by(|a, b| a.total_cmp(b))
                .map_or_else(|| "--".to_string(), ToString::to_string),
            _ if temporal => "--".to_string(),
            StatCol::Min => {
                if (is_continuous || ordered_number) && n > 0 {
                    if is_int_storage && let Some(&lo) = sorted_ints.first() {
                        lo.to_string()
                    } else {
                        format_real(sorted_cont.first().copied().unwrap_or(f64::NAN), decimals)
                    }
                } else {
                    "--".to_string()
                }
            }
            StatCol::Max => {
                if (is_continuous || ordered_number) && n > 0 {
                    if is_int_storage && let Some(&hi) = sorted_ints.last() {
                        hi.to_string()
                    } else {
                        format_real(sorted_cont.last().copied().unwrap_or(f64::NAN), decimals)
                    }
                } else {
                    "--".to_string()
                }
            }
            StatCol::Sum => {
                if is_continuous && n > 0 {
                    if is_int_storage && sorted_ints.len() == n {
                        int_sum.to_string()
                    } else {
                        format_real(sum, decimals)
                    }
                } else {
                    "--".to_string()
                }
            }
            StatCol::Range => {
                if is_continuous && n > 0 {
                    if is_int_storage
                        && let (Some(&lo), Some(&hi)) = (sorted_ints.first(), sorted_ints.last())
                    {
                        (i128::from(hi) - i128::from(lo)).to_string()
                    } else {
                        let lo = sorted_cont.first().copied().unwrap_or(f64::NAN);
                        let hi = sorted_cont.last().copied().unwrap_or(f64::NAN);
                        format_real(hi - lo, decimals)
                    }
                } else {
                    "--".to_string()
                }
            }
            StatCol::Mean => {
                if is_continuous {
                    format_real(mean, decimals)
                } else {
                    "--".to_string()
                }
            }
            StatCol::SeMean => {
                if is_continuous {
                    format_real(semean, decimals)
                } else {
                    "--".to_string()
                }
            }
            StatCol::Correlation => corr_stats
                .as_ref()
                .map_or_else(|| "--".to_string(), |c| format_real(c.r, decimals)),
            StatCol::CorrelationT => corr_stats
                .as_ref()
                .map_or_else(|| "--".to_string(), |c| format_real(c.t_val, decimals)),
            StatCol::CorrelationTDf => corr_stats
                .as_ref()
                .map_or_else(|| "--".to_string(), |c| format_real(c.df as f64, decimals)),
            StatCol::CorrelationTSig => corr_stats
                .as_ref()
                .map_or_else(|| "--".to_string(), |c| format_real(c.sig, decimals)),
            StatCol::Covariance => corr_stats
                .as_ref()
                .map_or_else(|| "--".to_string(), |c| format_real(c.cov, decimals)),
            StatCol::StdDev => {
                if is_continuous {
                    format_real(sdev, decimals)
                } else {
                    "--".to_string()
                }
            }
            StatCol::Variance => {
                if is_continuous {
                    format_real(variance, decimals)
                } else {
                    "--".to_string()
                }
            }
            StatCol::Skewness => {
                if is_continuous {
                    format_real(skewness, decimals)
                } else {
                    "--".to_string()
                }
            }
            StatCol::SeSkewness => {
                if is_continuous {
                    format_real(seskewness, decimals)
                } else {
                    "--".to_string()
                }
            }
            StatCol::Kurtosis => {
                if is_continuous {
                    format_real(kurtosis, decimals)
                } else {
                    "--".to_string()
                }
            }
            StatCol::SeKurtosis => {
                if is_continuous {
                    format_real(sekurtosis, decimals)
                } else {
                    "--".to_string()
                }
            }
            StatCol::Median => {
                let median = median_sorted(&sorted_cont);
                if is_continuous && is_int_storage && median.fract() == 0.0 {
                    // outputs/dataaudit_full_stats: integer `iv` has median `25`.
                    format!("{median:.0}")
                } else if is_continuous {
                    format_real(median, decimals)
                } else {
                    "--".to_string()
                }
            }
            StatCol::Mode => {
                if is_continuous {
                    format_continuous_mode(&sorted_cont, &sorted_ints, is_int_storage, decimals)
                } else {
                    format_categorical_mode(&non_cont_vals, decimals)
                }
            }
        };
        out.push(cell);
    }

    let unique_str = match field.measurement {
        _ if record_id => "--".to_string(),
        Measurement::Nominal | Measurement::Ordinal | Measurement::Flag => unique_category_count(
            &input
                .rows()
                .iter()
                .filter_map(|row| row.get(col))
                .filter(|v| !v.is_null())
                .cloned()
                .collect::<Vec<_>>(),
        )
        .map_or_else(|| "--".to_string(), |u| u.to_string()),
        _ => "--".to_string(),
    };
    out.push(unique_str);
    out.push(valid_count.to_string());
    out
}

struct CorrStats {
    r: f64,
    t_val: f64,
    df: usize,
    sig: f64,
    cov: f64,
}

fn compute_pair_correlation(
    input: &Table,
    col_x: usize,
    field_x: &Field,
    col_y: usize,
    field_y: &Field,
) -> Option<CorrStats> {
    let mut pairs: Vec<(f64, f64)> = Vec::new();
    for row in input.rows() {
        let (Some(vx), Some(vy)) = (row.get(col_x), row.get(col_y)) else {
            continue;
        };
        if is_audit_missing(field_x, vx) || is_audit_missing(field_y, vy) {
            continue;
        }
        if let (Some(x), Some(y)) = (continuous_value_as_f64(vx), continuous_value_as_f64(vy)) {
            pairs.push((x, y));
        }
    }
    let n = pairs.len();
    if n < 3 {
        return None;
    }
    let n_f = n as f64;
    let mx = pairs.iter().map(|(x, _)| *x).sum::<f64>() / n_f;
    let my = pairs.iter().map(|(_, y)| *y).sum::<f64>() / n_f;
    let mut sxx = 0.0f64;
    let mut syy = 0.0f64;
    let mut sxy = 0.0f64;
    for &(x, y) in &pairs {
        let dx = x - mx;
        let dy = y - my;
        sxx += dx * dx;
        syy += dy * dy;
        sxy += dx * dy;
    }
    let cov = sxy / (n_f - 1.0);
    let denom = (sxx * syy).sqrt();
    if denom <= 0.0 || !denom.is_finite() {
        return None;
    }
    let r = (sxy / denom).clamp(-1.0, 1.0);
    let df = n - 2;
    let df_f = df as f64;
    let r2 = r * r;
    let (t_val, sig) = if r2 >= 1.0 {
        (f64::INFINITY, 0.0)
    } else {
        let t = r * (df_f / (1.0 - r2)).sqrt();
        let x = (1.0 - r2).clamp(0.0, 1.0);
        let p = regularized_incomplete_beta(0.5 * df_f, 0.5, x).clamp(0.0, 1.0);
        (t, p)
    };
    Some(CorrStats {
        r,
        t_val,
        df,
        sig,
        cov,
    })
}

fn is_audit_missing(field: &Field, v: &Value) -> bool {
    if field.is_missing(v) {
        return true;
    }
    matches!(v, Value::String(s) if s.chars().all(char::is_whitespace))
}

/// CLEM's date baseline (`crates/clem/src/functions/dates.rs`).
const BASELINE_YEAR: i64 = 1900;

/// Converts a cell value to `f64` for continuous Data Audit statistics.
/// Date and Timestamp values are converted to seconds since `1900-01-01`. Time values are seconds
/// since midnight.
fn continuous_value_as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Integer(i) => Some(*i as f64),
        Value::Real(r) if r.is_finite() => Some(*r),
        Value::Date(days) => {
            let base_days = calendar::days_from_civil(BASELINE_YEAR, 1, 1)?;
            let delta_days = *days - base_days;
            Some((delta_days as f64) * 86_400.0)
        }
        Value::Time(micros) => Some((*micros as f64) / 1_000_000.0),
        Value::Timestamp(micros) => {
            let base_days = calendar::days_from_civil(BASELINE_YEAR, 1, 1)?;
            let base_micros = base_days * calendar::MICROS_PER_DAY;
            Some(((*micros - base_micros) as f64) / 1_000_000.0)
        }
        _ => None,
    }
}

/// The field with the measurement level Data Audit reports after reading the data: a categorical
/// field whose values were not read is Nominal (team/cluster_analysis `ADDRESS`), an
/// uninstantiated one Continuous when numeric and Nominal otherwise.
fn instantiated(field: &Field) -> Field {
    let mut field = field.clone();
    field.measurement = match field.measurement {
        Measurement::Categorical => Measurement::Nominal,
        Measurement::Default => match field.storage {
            Storage::Integer
            | Storage::Real
            | Storage::Date
            | Storage::Time
            | Storage::Timestamp => Measurement::Continuous,
            _ => Measurement::Nominal,
        },
        other => other,
    };
    field
}

fn measurement_label(measure: Measurement) -> &'static str {
    match measure {
        Measurement::Continuous => "Continuous",
        Measurement::Nominal => "Nominal",
        Measurement::Ordinal => "Ordinal",
        Measurement::Flag => "Flag",
        Measurement::Typeless => "Typeless",
        Measurement::Default => "Uninstantiated",
        Measurement::Categorical => "Categorical",
        Measurement::Collection => "Collection",
        Measurement::Geospatial => "Geospatial",
    }
}

/// A statistic as Data Audit prints it: `--` when it does not exist.
fn format_real(value: f64, decimals: usize) -> String {
    if value.is_finite() {
        crate::output::format_real(value, decimals)
    } else {
        "--".to_string()
    }
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

/// Footnote marker `\u{00b9}` (`¹`) appended by Modeler's Data Audit when multiple values tie for
/// the modal frequency.
const MODE_TIE_MARKER: char = '\u{00b9}';

fn format_continuous_mode(
    sorted: &[f64],
    sorted_ints: &[i64],
    is_int: bool,
    decimals: usize,
) -> String {
    if sorted.is_empty() {
        return "--".to_string();
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
            Some(v) if tied => format!("{v}{MODE_TIE_MARKER}"),
            Some(v) => v.to_string(),
            None => "--".to_string(),
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
        Some(v) if tied => format!("{v:.decimals$}{MODE_TIE_MARKER}"),
        Some(v) => format!("{v:.decimals$}"),
        None => "--".to_string(),
    }
}

fn format_categorical_mode(values: &[Value], decimals: usize) -> String {
    if values.is_empty() {
        return "--".to_string();
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(Value::total_cmp);

    let mut best_val: Option<&Value> = None;
    let mut best_count: usize = 0;
    let mut tied = false;
    let mut i = 0;
    while i < sorted.len() {
        let v = &sorted[i];
        let mut j = i + 1;
        while j < sorted.len() && sorted[j].total_cmp(v) == Ordering::Equal {
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

    let Some(v) = best_val else {
        return "--".to_string();
    };
    let text = match v {
        Value::Real(r) if r.is_finite() => format!("{r:.decimals$}"),
        other => other.to_string(),
    };
    if tied {
        format!("{text}{MODE_TIE_MARKER}")
    } else {
        text
    }
}

/// The distinct non-null values seen, blanks included, not the declared ones
/// (outputs/dataaudit_default `grp`: A, B, C and an empty string; team/cluster_analysis
/// `DEFAULTED` declares 0 and 1 but holds a third value). More than Modeler's default
/// `max_set_size` (250) prints `--`.
fn unique_category_count(valid_values: &[Value]) -> Option<usize> {
    let distinct = count_distinct_values(valid_values);
    (distinct <= 250).then_some(distinct)
}

fn count_distinct_values(values: &[Value]) -> usize {
    if values.is_empty() {
        return 0;
    }
    let mut sorted: Vec<&Value> = values.iter().collect();
    sorted.sort_by(|a, b| a.total_cmp(b));
    sorted.dedup_by(|a, b| a.total_cmp(b) == Ordering::Equal);
    sorted.len()
}

#[allow(dead_code)]
fn _unused_hashmap_marker(_: &HashMap<(), ()>) {}
