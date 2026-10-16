//! Means (Output palette): compares means across groups of a categorical field (one-way ANOVA
//! F-test), returning a [`Document`] with one table matching Modeler's exported CSV
//! (`docs/oracle.md`).
//!
//! Implemented settings (`ModelerScriptingAutomation-19.0.txt` "means properties"):
//! - `method`: `betweenGroups` (default); `withinRecords` (paired samples t-test) is rejected with
//!   [`OperatorError::Settings`].
//! - `groupingField`: categorical grouping field (required in `betweenGroups` mode).
//! - `meansGroupedFields/field`: continuous test fields to compare across groups (required).
//! - `viewMode` (`simple` | `advanced`), `importanceShowLabels`, `importanceLowThreshold`,
//!   `importanceMediumThreshold`, `importanceLowLabel`, `importanceMediumLabel`,
//!   `importanceHighLabel`. Note: Modeler's delimited (`.csv`) export always emits the full
//!   advanced columns (`Mean (g)`, `Std. Dev. (g)`, `Std. Err. (g)`, `Count (g)` per group, plus
//!   `F-Test`, `df1`, `df2`, `Importance` as raw numeric `1 - p` without `/Label`).
//! - `outputMode` (`screen` only); file/printer output modes are rejected with
//!   [`OperatorError::Settings`].

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::nodes::field_util::position;
use crate::nodes::statistics::regularized_incomplete_beta;
use crate::operator::{Arity, Operator};
use crate::output::{Document, DocumentTable, NodeOutput, format_real};
use crate::settings;
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::{Field, Table, Value, ValueSpec};
use std::cmp::Ordering;
use std::sync::Arc;

#[derive(Debug, Clone)]
struct MeansOp {
    grouping_field: String,
    test_fields: Vec<String>,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();

    match settings::text(s, "outputMode").as_deref().map(str::trim) {
        None | Some("") | Some("screen") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Means outputMode {other:?} is not supported"
            )));
        }
    }

    match settings::text(s, "method").as_deref().map(str::trim) {
        None | Some("") | Some("betweenGroups") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Means method {other:?} is not supported yet"
            )));
        }
    }

    match settings::text(s, "viewMode").as_deref().map(str::trim) {
        None | Some("") | Some("simple") | Some("advanced") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Means viewMode {other:?} is unknown"
            )));
        }
    }

    let low = settings::real(s, "importanceLowThreshold")?.unwrap_or(0.9);
    let med = settings::real(s, "importanceMediumThreshold")?.unwrap_or(0.95);
    if !(0.0..=1.0).contains(&low) || !(0.0..=1.0).contains(&med) || low > med {
        return Err(OperatorError::Settings(format!(
            "Means importance thresholds ({low}, {med}) are invalid"
        )));
    }
    let _ = settings::flag(s, "importanceShowLabels")?;

    let grouping_field = settings::text(s, "groupingField")
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| OperatorError::Settings("Means requires a groupingField".into()))?;

    let test_fields: Vec<String> = settings::children(s, "meansGroupedFields", "field")
        .into_iter()
        .map(Element::text)
        .filter(|name| !name.is_empty())
        .collect();
    if test_fields.is_empty() {
        return Err(OperatorError::Settings(
            "Means requires at least one field in meansGroupedFields".into(),
        ));
    }

    Ok(Box::new(MeansOp {
        grouping_field,
        test_fields,
    }))
}

impl Operator for MeansOp {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(
            "Means is a terminal output node and produces a Document, not a Table".into(),
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
            .ok_or_else(|| OperatorError::Inputs("Means needs one input".into()))?;
        let decimals = context.decimal_places();

        let group_col = position(input.model(), &self.grouping_field)?;
        let group_field = input.model().fields().get(group_col).ok_or_else(|| {
            OperatorError::Data(format!("input has no field at position {group_col}"))
        })?;
        let groups = group_categories(input, group_col, group_field);

        let mut header = Vec::with_capacity(1 + groups.len() * 4 + 4);
        header.push("Field".to_string());
        for g in &groups {
            let label = group_label(g);
            header.push(format!("Mean ({label})"));
            header.push(format!("Std. Dev. ({label})"));
            header.push(format!("Std. Err. ({label})"));
            header.push(format!("Count ({label})"));
        }
        header.push("F-Test".to_string());
        header.push("df1".to_string());
        header.push("df2".to_string());
        header.push("Importance".to_string());

        let mut rows = Vec::with_capacity(self.test_fields.len());
        for test_name in &self.test_fields {
            let test_col = position(input.model(), test_name)?;
            let test_field = input.model().fields().get(test_col).ok_or_else(|| {
                OperatorError::Data(format!("input has no field at position {test_col}"))
            })?;
            rows.push(compute_means_row(
                input, group_col, &groups, test_col, test_field, decimals,
            ));
        }

        let table = DocumentTable {
            title: None,
            header,
            rows,
        };
        Ok(NodeOutput::Document(Arc::new(Document {
            title: String::new(),
            tables: vec![table],
        })))
    }
}

/// Group categories in Modeler order: declared `ValueSpec::Categories` / `Flag` values if
/// instantiated, otherwise distinct values seen in the column sorted by `Value::total_cmp`
/// (with `Value::Null` included if present in the data).
fn group_categories(input: &Table, col: usize, field: &Field) -> Vec<Value> {
    let mut cats: Vec<Value> = match &field.values {
        ValueSpec::Categories(vals) if !vals.is_empty() => vals.clone(),
        ValueSpec::Flag {
            true_value,
            false_value,
        } => {
            let mut v = vec![false_value.clone(), true_value.clone()];
            v.sort_by(Value::total_cmp);
            v.dedup_by(|a, b| a.total_cmp(b) == Ordering::Equal);
            v
        }
        _ => Vec::new(),
    };
    if cats.is_empty() {
        for row in input.rows() {
            if let Some(v) = row.get(col)
                && !cats.iter().any(|c| c.total_cmp(v) == Ordering::Equal)
            {
                cats.push(v.clone());
            }
        }
        cats.sort_by(Value::total_cmp);
    } else {
        // Also include system null if it occurs in the data and isn't already in declared categories.
        let has_null = input
            .rows()
            .iter()
            .any(|r| r.get(col).is_some_and(Value::is_null));
        if has_null && !cats.iter().any(Value::is_null) {
            cats.insert(0, Value::Null);
        }
    }
    cats
}

fn group_label(v: &Value) -> String {
    match v {
        Value::Null => "$null$".to_string(),
        other => other.to_string(),
    }
}

fn compute_means_row(
    input: &Table,
    group_col: usize,
    groups: &[Value],
    test_col: usize,
    test_field: &Field,
    decimals: usize,
) -> Vec<String> {
    let mut per_group: Vec<Vec<f64>> = vec![Vec::new(); groups.len()];
    for row in input.rows() {
        let (Some(gv), Some(tv)) = (row.get(group_col), row.get(test_col)) else {
            continue;
        };
        if test_field.is_missing(tv) {
            continue;
        }
        let Some(x) = tv.as_f64().filter(|x| x.is_finite()) else {
            continue;
        };
        if let Some(idx) = groups
            .iter()
            .position(|g| g.total_cmp(gv) == Ordering::Equal)
        {
            per_group[idx].push(x);
        }
    }

    let mut out = Vec::with_capacity(1 + groups.len() * 4 + 4);
    out.push(test_field.name.clone());

    let mut non_empty_groups = 0usize;
    let mut total_n = 0usize;
    let mut total_sum = 0.0f64;
    let mut group_stats: Vec<(usize, f64, f64)> = Vec::with_capacity(groups.len());

    for vals in &per_group {
        let n = vals.len();
        let sum: f64 = vals.iter().sum();
        let mean = if n > 0 { sum / (n as f64) } else { f64::NAN };
        let ss_within: f64 = if n >= 2 {
            vals.iter()
                .map(|&v| {
                    let d = v - mean;
                    d * d
                })
                .sum()
        } else {
            0.0
        };
        let sdev = if n >= 2 {
            (ss_within / ((n - 1) as f64)).sqrt()
        } else {
            f64::NAN
        };
        let serr = if n >= 2 {
            sdev / (n as f64).sqrt()
        } else {
            f64::NAN
        };

        out.push(if mean.is_finite() {
            format_real(mean, decimals)
        } else {
            "$null$".to_string()
        });
        out.push(if sdev.is_finite() {
            format_real(sdev, decimals)
        } else {
            "$null$".to_string()
        });
        out.push(if serr.is_finite() {
            format_real(serr, decimals)
        } else {
            "--".to_string()
        });
        out.push(n.to_string());

        if n > 0 {
            non_empty_groups += 1;
            total_n += n;
            total_sum += sum;
        }
        group_stats.push((n, mean, ss_within));
    }

    if non_empty_groups < 2 || total_n <= non_empty_groups {
        out.push("--".to_string());
        out.push("--".to_string());
        out.push("--".to_string());
        out.push("--".to_string());
        return out;
    }

    let grand_mean = total_sum / (total_n as f64);
    let mut ssb = 0.0f64;
    let mut ssw = 0.0f64;
    for &(n, mean, ss_within) in &group_stats {
        if n > 0 {
            let d = mean - grand_mean;
            ssb += (n as f64) * d * d;
            ssw += ss_within;
        }
    }

    let df1 = non_empty_groups - 1;
    let df2 = total_n - non_empty_groups;
    let df1_f = df1 as f64;
    let df2_f = df2 as f64;
    let msb = ssb / df1_f;
    let msw = ssw / df2_f;

    if msw <= 0.0 || !msw.is_finite() {
        out.push("--".to_string());
        out.push(df1.to_string());
        out.push(df2.to_string());
        out.push("--".to_string());
        return out;
    }

    let f_stat = (msb / msw).max(0.0);
    // p-value = P(F >= f_stat) = I_{df2 / (df2 + df1 * f_stat)}(df2 / 2, df1 / 2)
    let x = (df2_f / (df2_f + df1_f * f_stat)).clamp(0.0, 1.0);
    let p_val = regularized_incomplete_beta(0.5 * df2_f, 0.5 * df1_f, x);
    let importance = (1.0 - p_val).clamp(0.0, 1.0);

    out.push(format_real(f_stat, decimals));
    out.push(df1.to_string());
    out.push(df2.to_string());
    out.push(format_real(importance, decimals));
    out
}
