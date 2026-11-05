//! Analysis (Output palette, `AnalysisNode`): compares model predictions with the actual
//! target and produces a [`Document`] that matches Modeler's JSON export (`docs/oracle.md`).
//!
//! Settings are the elements saved on the oracle streams (`analysis_coincidence.str`,
//! `analysis_default.str`, `decision_trees_heart.str`). A child of `properties` that is not in
//! that set fails [`OperatorError::Settings`].
//!
//! Counts, percents, confidence figures and AUC/Gini use fixed precisions. Stream
//! `decimal_places` does not change them. In `file` mode (`outputMode` `file` with
//! `outputFileType` `text` `.txt`, `html` `.html` or `output_object` `.cou` and `outputFilePath`,
//! `oracle/outputs_more/analysis_file_*`) the accuracy tables are written as Modeler's
//! tab-indented text or `<dl>` HTML (CRLF, no trailing newline for HTML); coincidence,
//! confidence, partition, breakdown and evaluation tables in file mode stay a `GAP:`
//! (no oracle). The `.cou` output object (FastInfoset with a save timestamp) stays a `GAP:`.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::nodes::field_util::position;
use crate::operator::{Arity, Operator};
use crate::output::{Document, DocumentTable, NodeOutput, format_real};
use crate::settings;
use dms_pmml::Pmml;
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::{Field, Measurement, Role, Storage, Table, Value, ValueSpec};
use std::cmp::Ordering;
use std::sync::Arc;

/// Direct children of `properties` on every Analysis node in the oracle streams, plus the
/// analysis settings those streams actually set. Anything else is rejected.
const KNOWN_SETTINGS: &[&str] = &[
    "analysisBinaryEvaluation",
    "analysisBreakdownBy",
    "analysisCoincidenceMatrix",
    "analysisConfidenceFigures",
    "analysisImproveAccuracy",
    "analysisThreshold",
    "analysisUserCompute",
    "analysisUserDefined",
    "analysisUserElse",
    "analysisUserIf",
    "analysisUserThen",
    "appendTagField",
    "autoLabel",
    "autoOutputName",
    "cacheEnabled",
    "createdBy",
    "creationDate",
    "doReadValues",
    "enabled",
    "fieldDetectionMethod",
    "fieldMappingEssentialFields",
    "annotation",
    "keywords",
    "label",
    "lastSavedAs",
    "lastSavedBy",
    "lastSavedDate",
    "name",
    "outputFilePath",
    "outputFileType",
    "outputMode",
    "outputName",
    "position",
    "repositoryLastModified",
    "repositoryPath",
    "repositoryVersionMarker",
    "scriptExecutionModeChange",
    "splitByPartition",
    "toolTipText",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetectionMethod {
    Metadata,
    Name,
}

#[derive(Debug, Clone)]
struct AnalysisOp {
    detection: DetectionMethod,
    split_by_partition: bool,
    coincidence: bool,
    confidence: bool,
    /// Percent in `(0, 100]`. The accuracy-above row uses it as a cutoff.
    threshold_pct: f64,
    /// Fold factor `f > 0`. The fold row requires accuracy strictly above `2f / (2f + 1)`.
    improve_accuracy: f64,
    binary_evaluation: bool,
    breakdown_by: Vec<String>,
    file: Option<(String, FileKind)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileKind {
    Text,
    Html,
    Output,
}

#[derive(Debug, Clone)]
struct ModelPair {
    pred_name: String,
    pred_col: usize,
    /// `$RC…` for an `$R…` prediction, `$LP…` for `$L…`, when that column exists.
    conf_col: Option<usize>,
}

#[derive(Debug, Clone)]
struct TargetGroup {
    target_name: String,
    target_col: usize,
    models: Vec<ModelPair>,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    reject_unknown(s)?;

    let file = match settings::text(s, "outputMode").as_deref().map(str::trim) {
        None | Some("") | Some("screen") => None,
        Some("file") => {
            let path = settings::text(s, "outputFilePath")
                .filter(|p| !p.trim().is_empty())
                .ok_or_else(|| {
                    OperatorError::Settings(
                        "Analysis outputFilePath is required in file mode".into(),
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
                        "Analysis outputFileType {other:?} is unknown"
                    )));
                }
            };
            Some((path, kind))
        }
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Analysis outputMode {other:?} is not supported"
            )));
        }
    };
    match settings::text(s, "outputFileType")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("html") | Some("text") | Some("output_object") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Analysis outputFileType {other:?} is unknown"
            )));
        }
    }
    let _ = settings::flag(s, "autoOutputName")?;
    let _ = settings::text(s, "outputName");
    let _ = settings::text(s, "outputFilePath");
    let _ = settings::text(s, "analysisUserIf");
    let _ = settings::text(s, "analysisUserThen");
    let _ = settings::text(s, "analysisUserElse");

    if settings::flag(s, "appendTagField")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "Analysis appendTagField is not supported".into(),
        ));
    }
    if settings::flag(s, "analysisUserDefined")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "Analysis analysisUserDefined is not supported".into(),
        ));
    }
    validate_user_compute(s)?;

    let detection = match settings::text(s, "fieldDetectionMethod")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("metadata") => DetectionMethod::Metadata,
        Some("name") => DetectionMethod::Name,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Analysis fieldDetectionMethod {other:?} is unknown"
            )));
        }
    };

    let threshold_pct = settings::real(s, "analysisThreshold")?.unwrap_or(90.0);
    if !(threshold_pct > 0.0 && threshold_pct <= 100.0) {
        return Err(OperatorError::Settings(format!(
            "Analysis analysisThreshold {threshold_pct} must be in (0, 100]"
        )));
    }
    let improve_accuracy = settings::real(s, "analysisImproveAccuracy")?.unwrap_or(2.0);
    if !(improve_accuracy.is_finite() && improve_accuracy > 0.0) {
        return Err(OperatorError::Settings(format!(
            "Analysis analysisImproveAccuracy {improve_accuracy} must be positive"
        )));
    }

    Ok(Box::new(AnalysisOp {
        detection,
        split_by_partition: settings::flag(s, "splitByPartition")?.unwrap_or(true),
        coincidence: settings::flag(s, "analysisCoincidenceMatrix")?.unwrap_or(false),
        confidence: settings::flag(s, "analysisConfidenceFigures")?.unwrap_or(false),
        threshold_pct,
        improve_accuracy,
        binary_evaluation: settings::flag(s, "analysisBinaryEvaluation")?.unwrap_or(false),
        breakdown_by: field_list(s, "analysisBreakdownBy"),
        file,
    }))
}

fn reject_unknown(settings: Option<&Element>) -> Result<(), OperatorError> {
    let Some(settings) = settings else {
        return Ok(());
    };
    for child in settings.child_elements() {
        let name = child.name.local.as_ref();
        if !KNOWN_SETTINGS.contains(&name) {
            return Err(OperatorError::Settings(format!(
                "Analysis setting {name:?} is not supported"
            )));
        }
    }
    Ok(())
}

fn validate_user_compute(s: Option<&Element>) -> Result<(), OperatorError> {
    for el in settings::children(s, "analysisUserCompute", "setElement") {
        match el.text().trim() {
            "mean" | "sum" | "min" | "max" | "sdev" => {}
            other => {
                return Err(OperatorError::Settings(format!(
                    "Analysis analysisUserCompute item {other:?} is unknown"
                )));
            }
        }
    }
    Ok(())
}

fn field_list(s: Option<&Element>, path: &str) -> Vec<String> {
    settings::children(s, path, "field")
        .into_iter()
        .map(Element::text)
        .filter(|name| !name.is_empty())
        .collect()
}

impl Operator for AnalysisOp {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(
            "Analysis is a terminal output node and produces a Document, not a Table".into(),
        ))
    }

    fn produce(
        &self,
        inputs: &[Arc<Table>],
        _model: Option<Arc<Pmml>>,
        context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        // Fixed precisions (`docs/oracle.md`); the stream decimal setting is not applied.
        let _decimals = context.decimal_places();
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Analysis needs one input".into()))?;

        let breakdown: Vec<(String, usize)> = self
            .breakdown_by
            .iter()
            .map(|name| Ok((name.clone(), position(input.model(), name)?)))
            .collect::<Result<Vec<_>, OperatorError>>()?;

        let part_col = self.partition_column(input);
        let targets = self.detect_targets(input);
        let all_rows: Vec<usize> = (0..input.row_count()).collect();
        let mut tables = Vec::new();

        for target in &targets {
            let Some(target_field) = input.model().fields().get(target.target_col) else {
                continue;
            };
            let results = format!("Results for output field {}", target.target_name);
            let partitioned =
                part_col.is_some_and(|col| {
                    input.model().fields().get(col).is_some_and(|field| {
                        !observed_values(input, col, field, &all_rows).is_empty()
                    })
                });
            // A breakdown makes the unsplit block "Overall Results" (`analysis_coincidence`).
            // A partition field, with no breakdown, makes it "Individual Models" (team streams).
            // Both together is not in the oracle: the breakdown name wins.
            let wrapper = if !breakdown.is_empty() {
                Some("Overall Results")
            } else if partitioned {
                Some("Individual Models")
            } else {
                None
            };
            let mut prefix = vec![results.clone()];
            if let Some(wrapper) = wrapper {
                prefix.push(wrapper.to_string());
            }
            for model in &target.models {
                self.emit_model(
                    input,
                    target,
                    target_field,
                    model,
                    &all_rows,
                    part_col,
                    &prefix,
                    &mut tables,
                );
            }
            if target.models.len() >= 2 && is_categorical(target_field) {
                emit_agreement(
                    input,
                    target,
                    target_field,
                    &all_rows,
                    part_col,
                    std::slice::from_ref(&results),
                    &mut tables,
                );
            }
            if self.binary_evaluation {
                emit_evaluation(
                    input,
                    target,
                    target_field,
                    &all_rows,
                    part_col,
                    std::slice::from_ref(&results),
                    &mut tables,
                );
            }

            for (b_name, b_col) in &breakdown {
                let Some(b_field) = input.model().fields().get(*b_col) else {
                    continue;
                };
                let split_root = format!(
                    "Output field {}, splitting by field {b_name}",
                    target.target_name
                );
                for cat in observed_values(input, *b_col, b_field, &all_rows) {
                    let cat_title =
                        format!("{b_name} = {}", format_cell_value(&cat, Some(b_field)));
                    let sub_rows = filter_rows(input, &all_rows, *b_col, &cat);
                    let prefix = vec![split_root.clone(), cat_title];
                    for model in &target.models {
                        self.emit_model(
                            input,
                            target,
                            target_field,
                            model,
                            &sub_rows,
                            part_col,
                            &prefix,
                            &mut tables,
                        );
                    }
                }
            }
        }

        let doc = Document {
            title: String::new(),
            tables,
        };
        self.write_file(&doc, context)?;
        Ok(NodeOutput::Document(Arc::new(doc)))
    }
}

impl AnalysisOp {
    fn write_file(&self, doc: &Document, context: &ExecContext) -> Result<(), OperatorError> {
        let Some((path, kind)) = &self.file else {
            return Ok(());
        };
        if context.export_dir.is_none() {
            if matches!(kind, FileKind::Output) {
                return Err(OperatorError::Settings(
                    "GAP: Analysis: Output (.cou) file is FastInfoset with a save timestamp and has no byte-stable oracle"
                        .into(),
                ));
            }
            if !analysis_file_supported(doc) {
                return Err(OperatorError::Settings(
                    "GAP: Analysis: file output for coincidence, confidence, partition, breakdown or evaluation tables has no oracle"
                        .into(),
                ));
            }
            return Ok(());
        }
        match kind {
            FileKind::Output => Err(OperatorError::Settings(
                "GAP: Analysis: Output (.cou) file is FastInfoset with a save timestamp and has no byte-stable oracle"
                    .into(),
            )),
            FileKind::Text => {
                if !analysis_file_supported(doc) {
                    return Err(OperatorError::Settings(
                        "GAP: Analysis: file output for coincidence, confidence, partition, breakdown or evaluation tables has no oracle"
                            .into(),
                    ));
                }
                let text = analysis_text(doc);
                write_bytes(&context.export_path(path), text.as_bytes())?;
                Ok(())
            }
            FileKind::Html => {
                if !analysis_file_supported(doc) {
                    return Err(OperatorError::Settings(
                        "GAP: Analysis: file output for coincidence, confidence, partition, breakdown or evaluation tables has no oracle"
                            .into(),
                    ));
                }
                let html = analysis_html(doc);
                write_bytes(&context.export_path(path), html.as_bytes())?;
                Ok(())
            }
        }
    }
    fn partition_column(&self, input: &Table) -> Option<usize> {
        if !self.split_by_partition {
            return None;
        }
        input
            .model()
            .fields()
            .iter()
            .position(|field| field.role == Role::Partition)
    }

    fn detect_targets(&self, input: &Table) -> Vec<TargetGroup> {
        let fields = input.model().fields();
        let mut groups: Vec<TargetGroup> = Vec::new();
        for (col, field) in fields.iter().enumerate() {
            let Some((prefix, index, target_name)) = parse_prediction_field(&field.name) else {
                continue;
            };
            let Some(target_col) = input.model().position(target_name) else {
                continue;
            };
            let Some(target_field) = fields.get(target_col) else {
                continue;
            };
            if self.detection == DetectionMethod::Metadata
                && !matches!(target_field.role, Role::Target | Role::Both)
            {
                continue;
            }
            let conf_name = confidence_field_name(prefix, index, target_name);
            let pair = ModelPair {
                pred_name: field.name.clone(),
                pred_col: col,
                conf_col: input.model().position(&conf_name),
            };
            if let Some(group) = groups.iter_mut().find(|g| g.target_col == target_col) {
                group.models.push(pair);
            } else {
                groups.push(TargetGroup {
                    target_name: target_name.to_string(),
                    target_col,
                    models: vec![pair],
                });
            }
        }
        groups
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_model(
        &self,
        input: &Table,
        target: &TargetGroup,
        target_field: &Field,
        model: &ModelPair,
        rows: &[usize],
        part_col: Option<usize>,
        prefix: &[String],
        tables: &mut Vec<DocumentTable>,
    ) {
        let Some(pred_field) = input.model().fields().get(model.pred_col) else {
            return;
        };
        let comparing = format!("Comparing {} with {}", model.pred_name, target.target_name);
        let mut chain = prefix.to_vec();
        chain.push(comparing);

        tables.push(accuracy_table(
            input,
            target.target_col,
            target_field,
            model.pred_col,
            pred_field,
            rows,
            part_col,
            &chain,
        ));

        if self.coincidence && is_categorical(target_field) {
            let matrix_title = format!(
                "Coincidence Matrix for {} (rows show actuals)",
                model.pred_name
            );
            let mut matrix_chain = chain.clone();
            matrix_chain.push(matrix_title);
            tables.extend(coincidence_tables(
                input,
                target.target_col,
                target_field,
                model.pred_col,
                pred_field,
                rows,
                part_col,
                &matrix_chain,
            ));
        }

        if self.confidence
            && let Some(conf_col) = model.conf_col
            && let Some(conf_field) = input.model().fields().get(conf_col)
        {
            let mut conf_chain = chain;
            conf_chain.push(format!("Confidence Values Report for {}", conf_field.name));
            tables.push(confidence_table(
                input,
                target.target_col,
                target_field,
                model.pred_col,
                pred_field,
                conf_col,
                conf_field,
                rows,
                self.threshold_pct,
                self.improve_accuracy,
                &conf_chain,
            ));
        }
    }
}

fn is_categorical(field: &Field) -> bool {
    !matches!(field.measurement, Measurement::Continuous)
}

/// `$R` / `$R1` / `$L` / `$L2` prediction columns. `$RC`, `$RRP`, `$LP` and `$LRP` do not match:
/// after the `R` or `L` the rest of the head must be digits only.
fn parse_prediction_field(name: &str) -> Option<(&str, &str, &str)> {
    let rest = name.strip_prefix('$')?;
    let (head, target) = rest.split_once('-')?;
    if target.is_empty() || head.is_empty() {
        return None;
    }
    let (prefix, index) = if let Some(index) = head.strip_prefix('L') {
        ("L", index)
    } else if let Some(index) = head.strip_prefix('R') {
        ("R", index)
    } else {
        return None;
    };
    if index.chars().all(|c| c.is_ascii_digit()) {
        Some((prefix, index, target))
    } else {
        None
    }
}

fn confidence_field_name(prefix: &str, index: &str, target: &str) -> String {
    let tag = if prefix == "L" { "LP" } else { "RC" };
    format!("${tag}{index}-{target}")
}

#[allow(clippy::too_many_arguments)]
fn accuracy_table(
    input: &Table,
    target_col: usize,
    target_field: &Field,
    pred_col: usize,
    pred_field: &Field,
    rows: &[usize],
    part_col: Option<usize>,
    titles: &[String],
) -> DocumentTable {
    let cats = partition_categories(input, part_col, rows);
    if let Some(col) = part_col
        && !cats.is_empty()
    {
        let part_field = input.model().fields().get(col);
        let mut header = Vec::with_capacity(1 + cats.len() * 2);
        header.push(partition_header_name(part_field));
        for cat in &cats {
            header.push(format_cell_value(cat, part_field));
            header.push(String::new());
        }
        let mut correct = vec!["Correct".to_string()];
        let mut wrong = vec!["Wrong".to_string()];
        let mut total = vec!["Total".to_string()];
        for cat in &cats {
            let sub = filter_rows(input, rows, col, cat);
            let (ok, bad, n) =
                count_correct(input, target_col, target_field, pred_col, pred_field, &sub);
            correct.push(format_int(ok));
            correct.push(format_pct(ok, n));
            wrong.push(format_int(bad));
            wrong.push(format_pct(bad, n));
            total.push(format_int(n));
            total.push(String::new());
        }
        return titled(titles, header, vec![correct, wrong, total]);
    }
    let (ok, bad, n) = count_correct(input, target_col, target_field, pred_col, pred_field, rows);
    titled(
        titles,
        Vec::new(),
        vec![
            vec!["Correct".to_string(), format_int(ok), format_pct(ok, n)],
            vec!["Wrong".to_string(), format_int(bad), format_pct(bad, n)],
            vec!["Total".to_string(), format_int(n), String::new()],
        ],
    )
}

fn count_correct(
    input: &Table,
    target_col: usize,
    target_field: &Field,
    pred_col: usize,
    pred_field: &Field,
    rows: &[usize],
) -> (usize, usize, usize) {
    let mut ok = 0usize;
    let mut bad = 0usize;
    for &ri in rows {
        let Some(row) = input.rows().get(ri) else {
            continue;
        };
        let (Some(tv), Some(pv)) = (row.get(target_col), row.get(pred_col)) else {
            continue;
        };
        if target_field.is_missing(tv) || pred_field.is_missing(pv) {
            continue;
        }
        if values_match(tv, pv) {
            ok += 1;
        } else {
            bad += 1;
        }
    }
    (ok, bad, ok + bad)
}

#[allow(clippy::too_many_arguments)]
fn coincidence_tables(
    input: &Table,
    target_col: usize,
    target_field: &Field,
    pred_col: usize,
    pred_field: &Field,
    rows: &[usize],
    part_col: Option<usize>,
    titles: &[String],
) -> Vec<DocumentTable> {
    let cats = partition_categories(input, part_col, rows);
    if let Some(col) = part_col
        && !cats.is_empty()
    {
        let part_field = input.model().fields().get(col);
        return cats
            .iter()
            .map(|cat| {
                let sub = filter_rows(input, rows, col, cat);
                let top_left = format!(
                    "{} = {}",
                    partition_header_name(part_field),
                    format_cell_value(cat, part_field)
                );
                one_matrix(
                    input,
                    target_col,
                    target_field,
                    pred_col,
                    pred_field,
                    &sub,
                    &top_left,
                    titles,
                )
            })
            .collect();
    }
    vec![one_matrix(
        input,
        target_col,
        target_field,
        pred_col,
        pred_field,
        rows,
        "",
        titles,
    )]
}

#[allow(clippy::too_many_arguments)]
fn one_matrix(
    input: &Table,
    target_col: usize,
    target_field: &Field,
    pred_col: usize,
    pred_field: &Field,
    rows: &[usize],
    top_left: &str,
    titles: &[String],
) -> DocumentTable {
    let mut pairs: Vec<(Value, Value)> = Vec::new();
    for &ri in rows {
        let Some(row) = input.rows().get(ri) else {
            continue;
        };
        let (Some(tv), Some(pv)) = (row.get(target_col), row.get(pred_col)) else {
            continue;
        };
        if target_field.is_missing(tv) || pred_field.is_missing(pv) {
            continue;
        }
        pairs.push((tv.clone(), pv.clone()));
    }
    let present: Vec<Value> = pairs
        .iter()
        .flat_map(|(tv, pv)| [tv.clone(), pv.clone()])
        .collect();
    let columns = ordered_categories(target_field, &present);
    let row_cats: Vec<Value> = columns
        .iter()
        .filter(|cat| pairs.iter().any(|(tv, _)| values_match(tv, cat)))
        .cloned()
        .collect();

    let mut table_rows = Vec::with_capacity(1 + row_cats.len());
    let mut header = Vec::with_capacity(1 + columns.len());
    header.push(top_left.to_string());
    for cat in &columns {
        header.push(format_cell_value(cat, Some(target_field)));
    }
    table_rows.push(header);
    for row_cat in &row_cats {
        let mut out = Vec::with_capacity(1 + columns.len());
        out.push(format_cell_value(row_cat, Some(target_field)));
        for col_cat in &columns {
            let n = pairs
                .iter()
                .filter(|(tv, pv)| values_match(tv, row_cat) && values_match(pv, col_cat))
                .count();
            out.push(format_int(n));
        }
        table_rows.push(out);
    }
    titled(titles, Vec::new(), table_rows)
}

#[allow(clippy::too_many_arguments)]
fn confidence_table(
    input: &Table,
    target_col: usize,
    target_field: &Field,
    pred_col: usize,
    pred_field: &Field,
    conf_col: usize,
    conf_field: &Field,
    rows: &[usize],
    threshold_pct: f64,
    improve_fold: f64,
    titles: &[String],
) -> DocumentTable {
    let mut records: Vec<(f64, bool)> = Vec::new();
    for &ri in rows {
        let Some(row) = input.rows().get(ri) else {
            continue;
        };
        let (Some(tv), Some(pv), Some(cv)) =
            (row.get(target_col), row.get(pred_col), row.get(conf_col))
        else {
            continue;
        };
        if target_field.is_missing(tv) || pred_field.is_missing(pv) || conf_field.is_missing(cv) {
            continue;
        }
        let Some(c) = cv.as_f64().filter(|x| x.is_finite()) else {
            continue;
        };
        records.push((c, values_match(tv, pv)));
    }

    let n = records.len();
    let (range, mean_ok, mean_bad, always_ok, always_bad, accuracy_label, accuracy_value, fold) =
        if n == 0 {
            (
                "--".to_string(),
                "--".to_string(),
                "--".to_string(),
                "--".to_string(),
                "--".to_string(),
                format!("{}% Accuracy Above", format_trimmed(threshold_pct, 2)),
                "--".to_string(),
                "--".to_string(),
            )
        } else {
            confidence_figures(&records, threshold_pct, improve_fold)
        };

    let fold_label = format!("{} Fold Correct Above", format_fold(improve_fold));
    titled(
        titles,
        Vec::new(),
        vec![
            vec!["Range".to_string(), range],
            vec!["Mean Correct".to_string(), mean_ok],
            vec!["Mean Incorrect".to_string(), mean_bad],
            vec!["Always Correct Above".to_string(), always_ok],
            vec!["Always Incorrect Below".to_string(), always_bad],
            vec![accuracy_label, accuracy_value],
            vec![fold_label, fold],
        ],
    )
}

fn confidence_figures(
    records: &[(f64, bool)],
    threshold_pct: f64,
    improve_fold: f64,
) -> (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
) {
    let n = records.len();
    let min_c = records
        .iter()
        .map(|(c, _)| *c)
        .fold(f64::INFINITY, f64::min);
    let max_c = records
        .iter()
        .map(|(c, _)| *c)
        .fold(f64::NEG_INFINITY, f64::max);
    let range = format!(
        "{} - {}",
        format_trimmed(min_c, 3),
        format_trimmed(max_c, 3)
    );

    let correct: Vec<f64> = records
        .iter()
        .filter_map(|&(c, ok)| ok.then_some(c))
        .collect();
    let incorrect: Vec<f64> = records
        .iter()
        .filter_map(|&(c, ok)| (!ok).then_some(c))
        .collect();
    let mean_ok = mean_text(&correct);
    let mean_bad = mean_text(&incorrect);

    // Max incorrect confidence; the parenthetical is the share of cases strictly above it.
    let always_ok = if incorrect.is_empty() {
        format!("{} (100% of cases)", format_trimmed(min_c, 3))
    } else {
        let t = incorrect.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let above = records.iter().filter(|&&(c, _)| c > t).count();
        format!(
            "{} ({} of cases)",
            format_trimmed(t, 3),
            format_pct(above, n)
        )
    };
    // Min correct confidence; the parenthetical is the share of cases strictly below it.
    let always_bad = if correct.is_empty() {
        format!("{} (100% of cases)", format_trimmed(max_c, 3))
    } else {
        let t = correct.iter().copied().fold(f64::INFINITY, f64::min);
        let below = records.iter().filter(|&&(c, _)| c < t).count();
        format!(
            "{} ({} of cases)",
            format_trimmed(t, 3),
            format_pct(below, n)
        )
    };

    let distinct = distinct_confidences(records);
    // Smallest observed cutoff whose strict-above set reaches the configured accuracy.
    // The row label is the accuracy actually achieved, not the configured threshold.
    let target = threshold_pct / 100.0;
    let mut accuracy: Option<(f64, usize, usize)> = None;
    for &t in &distinct {
        let (ok, tot) = strict_above(records, t);
        if tot > 0 && (ok as f64) / (tot as f64) + 1e-12 >= target {
            accuracy = Some((t, ok, tot));
            break;
        }
    }
    let (accuracy_label, accuracy_value) = match accuracy {
        Some((t, ok, tot)) => {
            let achieved = (ok as f64) * 100.0 / (tot as f64);
            (
                format!("{}% Accuracy Above", format_trimmed(achieved, 2)),
                format_trimmed(t, 3),
            )
        }
        None => (
            format!("{}% Accuracy Above", format_trimmed(threshold_pct, 2)),
            "--".to_string(),
        ),
    };

    // `analysis_coincidence` (improve = 2): smallest t with accuracy of `c > t` strictly above
    // 0.8, shown as `0.6 (88.89% of cases)`. The parenthetical is that accuracy, not coverage.
    // Equivalent to `correct * (2f + 1) > total * 2f`. Only f = 2 is in the oracle; a larger
    // integer multiple of f would also fit those three cells.
    let factor = 2.0 * improve_fold;
    let denom = factor + 1.0;
    let mut fold = "--".to_string();
    for &t in &distinct {
        let (ok, tot) = strict_above(records, t);
        if tot == 0 {
            continue;
        }
        let lhs = (ok as f64) * denom;
        let rhs = (tot as f64) * factor;
        if lhs > rhs + 1e-9 {
            fold = format!(
                "{} ({} of cases)",
                format_trimmed(t, 3),
                format_pct(ok, tot)
            );
            break;
        }
    }

    (
        range,
        mean_ok,
        mean_bad,
        always_ok,
        always_bad,
        accuracy_label,
        accuracy_value,
        fold,
    )
}

fn mean_text(values: &[f64]) -> String {
    if values.is_empty() {
        "--".to_string()
    } else {
        format_trimmed(values.iter().sum::<f64>() / (values.len() as f64), 3)
    }
}

fn distinct_confidences(records: &[(f64, bool)]) -> Vec<f64> {
    let mut values: Vec<f64> = records.iter().map(|&(c, _)| c).collect();
    values.sort_by(f64::total_cmp);
    values.dedup_by(|a, b| a.total_cmp(b) == Ordering::Equal);
    values
}

fn strict_above(records: &[(f64, bool)], t: f64) -> (usize, usize) {
    let mut ok = 0usize;
    let mut tot = 0usize;
    for &(c, correct) in records {
        if c > t {
            tot += 1;
            if correct {
                ok += 1;
            }
        }
    }
    (ok, tot)
}

fn emit_agreement(
    input: &Table,
    target: &TargetGroup,
    target_field: &Field,
    rows: &[usize],
    part_col: Option<usize>,
    prefix: &[String],
    tables: &mut Vec<DocumentTable>,
) {
    let preds: Vec<(usize, &Field)> = target
        .models
        .iter()
        .filter_map(|model| {
            input
                .model()
                .fields()
                .get(model.pred_col)
                .map(|field| (model.pred_col, field))
        })
        .collect();
    if preds.len() < 2 {
        return;
    }
    let names = target
        .models
        .iter()
        .map(|model| model.pred_name.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    // Two spaces after "between": `model_comparison` validation.
    let mut chain = prefix.to_vec();
    chain.push(format!("Agreement between  {names}"));
    let (agree_rows, correct_rows, header) = agreement_body(
        input,
        target.target_col,
        target_field,
        &preds,
        rows,
        part_col,
    );
    tables.push(titled(&chain, header.clone(), agree_rows));
    chain.push(format!("Comparing Agreement with {}", target.target_name));
    tables.push(titled(&chain, header, correct_rows));
}

fn agreement_body(
    input: &Table,
    target_col: usize,
    target_field: &Field,
    preds: &[(usize, &Field)],
    rows: &[usize],
    part_col: Option<usize>,
) -> (Vec<Vec<String>>, Vec<Vec<String>>, Vec<String>) {
    let cats = partition_categories(input, part_col, rows);
    if let Some(col) = part_col
        && !cats.is_empty()
    {
        let part_field = input.model().fields().get(col);
        let mut header = Vec::with_capacity(1 + cats.len() * 2);
        header.push(partition_header_name(part_field));
        let mut agree = vec!["Agree".to_string()];
        let mut disagree = vec!["Disagree".to_string()];
        let mut total = vec!["Total".to_string()];
        let mut correct = vec!["Correct".to_string()];
        let mut wrong = vec!["Wrong".to_string()];
        let mut correct_total = vec!["Total".to_string()];
        for cat in &cats {
            header.push(format_cell_value(cat, part_field));
            header.push(String::new());
            let sub = filter_rows(input, rows, col, cat);
            let counts = count_agreement(input, target_col, target_field, preds, &sub);
            push_pair(&mut agree, counts.agree, counts.compared);
            push_pair(&mut disagree, counts.disagree, counts.compared);
            total.push(format_int(counts.compared));
            total.push(String::new());
            push_pair(&mut correct, counts.correct, counts.agreed_with_target);
            push_pair(&mut wrong, counts.wrong, counts.agreed_with_target);
            correct_total.push(format_int(counts.agreed_with_target));
            correct_total.push(String::new());
        }
        return (
            vec![agree, disagree, total],
            vec![correct, wrong, correct_total],
            header,
        );
    }
    let counts = count_agreement(input, target_col, target_field, preds, rows);
    (
        vec![
            row3(
                "Agree",
                counts.agree,
                format_pct(counts.agree, counts.compared),
            ),
            row3(
                "Disagree",
                counts.disagree,
                format_pct(counts.disagree, counts.compared),
            ),
            row3("Total", counts.compared, String::new()),
        ],
        vec![
            row3(
                "Correct",
                counts.correct,
                format_pct(counts.correct, counts.agreed_with_target),
            ),
            row3(
                "Wrong",
                counts.wrong,
                format_pct(counts.wrong, counts.agreed_with_target),
            ),
            row3("Total", counts.agreed_with_target, String::new()),
        ],
        Vec::new(),
    )
}

fn push_pair(row: &mut Vec<String>, num: usize, den: usize) {
    row.push(format_int(num));
    row.push(format_pct(num, den));
}

fn row3(label: &str, n: usize, pct: String) -> Vec<String> {
    vec![label.to_string(), format_int(n), pct]
}

struct AgreementCounts {
    agree: usize,
    disagree: usize,
    compared: usize,
    correct: usize,
    wrong: usize,
    agreed_with_target: usize,
}

fn count_agreement(
    input: &Table,
    target_col: usize,
    target_field: &Field,
    preds: &[(usize, &Field)],
    rows: &[usize],
) -> AgreementCounts {
    let mut agree = 0usize;
    let mut disagree = 0usize;
    let mut correct = 0usize;
    let mut wrong = 0usize;
    for &ri in rows {
        let Some(row) = input.rows().get(ri) else {
            continue;
        };
        let mut values: Vec<&Value> = Vec::with_capacity(preds.len());
        let mut missing = false;
        for &(col, field) in preds {
            match row.get(col) {
                Some(value) if !field.is_missing(value) => values.push(value),
                _ => {
                    missing = true;
                    break;
                }
            }
        }
        if missing || values.is_empty() {
            continue;
        }
        let Some(first) = values.first() else {
            continue;
        };
        if values
            .iter()
            .skip(1)
            .all(|value| values_match(value, first))
        {
            agree += 1;
            if let Some(tv) = row.get(target_col)
                && !target_field.is_missing(tv)
            {
                if values_match(tv, first) {
                    correct += 1;
                } else {
                    wrong += 1;
                }
            }
        } else {
            disagree += 1;
        }
    }
    AgreementCounts {
        agree,
        disagree,
        compared: agree + disagree,
        correct,
        wrong,
        agreed_with_target: correct + wrong,
    }
}

fn emit_evaluation(
    input: &Table,
    target: &TargetGroup,
    target_field: &Field,
    rows: &[usize],
    part_col: Option<usize>,
    prefix: &[String],
    tables: &mut Vec<DocumentTable>,
) {
    let Some((negative, positive)) = binary_poles(input, target.target_col, target_field, rows)
    else {
        return;
    };
    let models: Vec<&ModelPair> = target
        .models
        .iter()
        .filter(|model| model.conf_col.is_some())
        .collect();
    if models.is_empty() {
        return;
    }
    let mut chain = prefix.to_vec();
    chain.push("Evaluation Metrics".to_string());
    let cats = partition_categories(input, part_col, rows);
    if let Some(col) = part_col
        && !cats.is_empty()
    {
        let part_field = input.model().fields().get(col);
        let mut header = Vec::with_capacity(1 + cats.len() * 2);
        header.push(partition_header_name(part_field));
        let mut sub = Vec::with_capacity(1 + cats.len() * 2);
        sub.push("Model".to_string());
        for cat in &cats {
            header.push(format_cell_value(cat, part_field));
            header.push(String::new());
            sub.push("AUC".to_string());
            sub.push("Gini".to_string());
        }
        let mut body = vec![sub];
        for model in models {
            body.push(eval_row(
                input,
                target,
                target_field,
                model,
                rows,
                Some(col),
                &cats,
                &negative,
                &positive,
            ));
        }
        tables.push(titled(&chain, header, body));
        return;
    }
    let mut body = Vec::with_capacity(models.len());
    for model in models {
        body.push(eval_row(
            input,
            target,
            target_field,
            model,
            rows,
            None,
            &[],
            &negative,
            &positive,
        ));
    }
    tables.push(titled(
        &chain,
        vec!["Model".to_string(), "AUC".to_string(), "Gini".to_string()],
        body,
    ));
}

#[allow(clippy::too_many_arguments)]
fn eval_row(
    input: &Table,
    target: &TargetGroup,
    target_field: &Field,
    model: &ModelPair,
    rows: &[usize],
    part_col: Option<usize>,
    cats: &[Value],
    negative: &Value,
    positive: &Value,
) -> Vec<String> {
    let mut row = vec![model.pred_name.clone()];
    let Some(pred_field) = input.model().fields().get(model.pred_col) else {
        return row;
    };
    let Some(conf_col) = model.conf_col else {
        return row;
    };
    let Some(conf_field) = input.model().fields().get(conf_col) else {
        return row;
    };
    let slices: Vec<Vec<usize>> = if let Some(col) = part_col {
        cats.iter()
            .map(|cat| filter_rows(input, rows, col, cat))
            .collect()
    } else {
        vec![rows.to_vec()]
    };
    for slice in &slices {
        let (auc, gini) = auc_gini(
            input,
            target.target_col,
            target_field,
            model.pred_col,
            pred_field,
            conf_col,
            conf_field,
            negative,
            positive,
            slice,
        );
        row.push(auc);
        row.push(gini);
    }
    row
}

fn binary_poles(
    input: &Table,
    target_col: usize,
    target_field: &Field,
    rows: &[usize],
) -> Option<(Value, Value)> {
    match &target_field.values {
        ValueSpec::Flag {
            true_value,
            false_value,
        } => return Some((false_value.clone(), true_value.clone())),
        ValueSpec::Categories(vals) => {
            let declared: Vec<Value> = vals
                .iter()
                .filter(|v| !target_field.is_missing(v))
                .cloned()
                .collect();
            if declared.len() == 2 {
                return Some((declared[0].clone(), declared[1].clone()));
            }
            if declared.len() > 2 {
                return None;
            }
        }
        _ => {}
    }
    let observed = observed_values(input, target_col, target_field, rows);
    if observed.len() == 2 {
        Some((observed[0].clone(), observed[1].clone()))
    } else {
        None
    }
}

/// Mann-Whitney AUC with tie averages. The score is the confidence when the prediction is the
/// positive class and `1 - confidence` otherwise. Gini is `2 * AUC - 1` before either is rounded
/// to 3 decimals. Swapping the two classes leaves both numbers unchanged.
#[allow(clippy::too_many_arguments)]
fn auc_gini(
    input: &Table,
    target_col: usize,
    target_field: &Field,
    pred_col: usize,
    pred_field: &Field,
    conf_col: usize,
    conf_field: &Field,
    negative: &Value,
    positive: &Value,
    rows: &[usize],
) -> (String, String) {
    let mut scores: Vec<(f64, bool)> = Vec::new();
    for &ri in rows {
        let Some(row) = input.rows().get(ri) else {
            continue;
        };
        let (Some(tv), Some(pv), Some(cv)) =
            (row.get(target_col), row.get(pred_col), row.get(conf_col))
        else {
            continue;
        };
        if target_field.is_missing(tv) || pred_field.is_missing(pv) || conf_field.is_missing(cv) {
            continue;
        }
        let Some(c) = cv.as_f64().filter(|x| x.is_finite()) else {
            continue;
        };
        let positive_case = if values_match(tv, positive) {
            true
        } else if values_match(tv, negative) {
            false
        } else {
            continue;
        };
        let score = if values_match(pv, positive) {
            c
        } else if values_match(pv, negative) {
            1.0 - c
        } else {
            continue;
        };
        scores.push((score, positive_case));
    }
    let n_pos = scores.iter().filter(|&&(_, pos)| pos).count();
    let n_neg = scores.len().saturating_sub(n_pos);
    if n_pos == 0 || n_neg == 0 {
        return ("--".to_string(), "--".to_string());
    }
    scores.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut u = 0.0f64;
    let mut neg_seen = 0usize;
    let mut i = 0usize;
    while i < scores.len() {
        let score = scores[i].0;
        let mut j = i;
        let mut group_pos = 0usize;
        let mut group_neg = 0usize;
        while j < scores.len() && (scores[j].0 - score).abs() <= 1e-12 {
            if scores[j].1 {
                group_pos += 1;
            } else {
                group_neg += 1;
            }
            j += 1;
        }
        u += (group_pos as f64) * ((neg_seen as f64) + 0.5 * (group_neg as f64));
        neg_seen += group_neg;
        i = j;
    }
    let auc = (u / ((n_pos as f64) * (n_neg as f64))).clamp(0.0, 1.0);
    let gini = 2.0 * auc - 1.0;
    (format_trimmed(auc, 3), format_trimmed(gini, 3))
}

fn partition_categories(input: &Table, part_col: Option<usize>, rows: &[usize]) -> Vec<Value> {
    let Some(col) = part_col else {
        return Vec::new();
    };
    let Some(field) = input.model().fields().get(col) else {
        return Vec::new();
    };
    observed_values(input, col, field, rows)
}

fn partition_header_name(field: Option<&Field>) -> String {
    match field {
        Some(field) => format!("'{}'", field.name),
        None => "'Partition'".to_string(),
    }
}

fn observed_values(input: &Table, col: usize, field: &Field, rows: &[usize]) -> Vec<Value> {
    let mut present = Vec::new();
    for &ri in rows {
        let Some(value) = input.rows().get(ri).and_then(|row| row.get(col)) else {
            continue;
        };
        if field.is_missing(value) || present.iter().any(|seen| values_match(seen, value)) {
            continue;
        }
        present.push(value.clone());
    }
    ordered_categories(field, &present)
}

/// Declared category order, else flag values in `total_cmp` order, else sorted observed values.
/// Categories that do not occur in `present` are dropped; extra observed values are appended.
fn ordered_categories(field: &Field, present: &[Value]) -> Vec<Value> {
    let mut ordered = match &field.values {
        ValueSpec::Categories(vals) if !vals.is_empty() => vals
            .iter()
            .filter(|v| !field.is_missing(v))
            .cloned()
            .collect(),
        ValueSpec::Flag {
            true_value,
            false_value,
        } => {
            let mut vals = vec![false_value.clone(), true_value.clone()];
            vals.sort_by(Value::total_cmp);
            vals.dedup_by(|a, b| values_match(a, b));
            vals
        }
        _ => Vec::new(),
    };
    if ordered.is_empty() {
        let mut vals = present.to_vec();
        vals.sort_by(Value::total_cmp);
        vals.dedup_by(|a, b| values_match(a, b));
        return vals;
    }
    for value in present {
        if !ordered.iter().any(|cat| values_match(cat, value)) {
            ordered.push(value.clone());
        }
    }
    ordered.retain(|cat| present.iter().any(|value| values_match(cat, value)));
    ordered
}

fn filter_rows(input: &Table, rows: &[usize], col: usize, value: &Value) -> Vec<usize> {
    rows.iter()
        .copied()
        .filter(|&ri| {
            input
                .rows()
                .get(ri)
                .and_then(|row| row.get(col))
                .is_some_and(|cell| values_match(cell, value))
        })
        .collect()
}

fn values_match(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Integer(i), Value::Real(r)) | (Value::Real(r), Value::Integer(i)) => {
            (*i as f64) == *r
        }
        _ => a.total_cmp(b) == Ordering::Equal,
    }
}

fn titled(titles: &[String], header: Vec<String>, rows: Vec<Vec<String>>) -> DocumentTable {
    DocumentTable {
        title: if titles.is_empty() {
            None
        } else {
            Some(titles.join(" | "))
        },
        header,
        rows,
    }
}

fn format_cell_value(value: &Value, field: Option<&Field>) -> String {
    match value {
        Value::Null => "$null$".to_string(),
        Value::Real(r) => format!("{r:.6}"),
        Value::Integer(i) if field.is_some_and(|f| f.storage == Storage::Real) => {
            format!("{:.6}", *i as f64)
        }
        other => other.to_string(),
    }
}

fn format_int(n: usize) -> String {
    let text = n.to_string();
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len() + text.len() / 3);
    for (i, &byte) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(char::from(byte));
    }
    out
}

fn format_pct(num: usize, den: usize) -> String {
    if den == 0 {
        return "0%".to_string();
    }
    let pct = (num as f64) * 100.0 / (den as f64);
    format!("{}%", format_trimmed(pct, 2))
}

fn format_trimmed(value: f64, decimals: usize) -> String {
    if !value.is_finite() {
        return "--".to_string();
    }
    let text = format_real(value, decimals);
    let Some((int_part, frac)) = text.split_once('.') else {
        return text;
    };
    let frac = frac.trim_end_matches('0');
    if frac.is_empty() {
        int_part.to_string()
    } else {
        format!("{int_part}.{frac}")
    }
}

/// Fold factors print with a decimal (`2` → `2.0`) so the row reads `2.0 Fold Correct Above`.
fn format_fold(value: f64) -> String {
    let text = format_trimmed(value, 3);
    if text.contains('.') {
        text
    } else {
        format!("{text}.0")
    }
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

/// Only accuracy-only tables with no partition header are written in file mode.
fn analysis_file_supported(doc: &Document) -> bool {
    if doc.tables.is_empty() {
        return false;
    }
    for table in &doc.tables {
        let Some(title) = &table.title else {
            return false;
        };
        if title.contains("Coincidence")
            || title.contains("Confidence")
            || title.contains("Evaluation Metrics")
            || title.contains("Agreement")
            || title.contains("Overall Results")
            || title.contains("Individual Models")
            || title.contains("splitting by")
        {
            return false;
        }
        if !table.header.is_empty() {
            return false;
        }
        if table.rows.len() != 3 || table.rows.iter().any(|r| r.len() != 3) {
            return false;
        }
    }
    true
}

fn analysis_text(doc: &Document) -> String {
    let mut out = String::new();
    for table in &doc.tables {
        let Some(title) = &table.title else {
            continue;
        };
        let levels: Vec<&str> = title.split(" | ").collect();
        for (depth, level) in levels.iter().enumerate() {
            out.push_str(&"\t".repeat(depth));
            out.push_str(level);
            out.push_str("\r\n");
        }
        let depth = levels.len();
        let indent = "\t".repeat(depth);
        // Column widths from the data: label padded to the longest label plus a fixed gap,
        // counts and percents right-aligned to their longest values.
        let label_width = table.rows.iter().map(|r| r[0].len()).max().unwrap_or(0);
        let count_width = table.rows.iter().map(|r| r[1].len()).max().unwrap_or(0);
        let pct_width = table.rows.iter().map(|r| r[2].len()).max().unwrap_or(0);
        for row in &table.rows {
            let label_pad = label_width - row[0].len() + 12;
            let count_pad = count_width.saturating_sub(row[1].len());
            let pct_pad = pct_width.saturating_sub(row[2].len());
            out.push_str(&indent);
            out.push_str(&row[0]);
            out.push_str(&" ".repeat(label_pad));
            out.push_str(&" ".repeat(count_pad));
            out.push_str(&row[1]);
            out.push_str("   ");
            out.push_str(&" ".repeat(pct_pad));
            out.push_str(&row[2]);
            out.push_str("\r\n");
        }
        out.push_str("\r\n");
    }
    out
}

fn analysis_html(doc: &Document) -> String {
    let mut lines = vec![
        "<HTML><HEAD>".to_string(),
        "<META HTTP-EQUIV=\"Cache-Control\" CONTENT=\"no-cache\">".to_string(),
        "<META HTTP-EQUIV=\"Pragma\" CONTENT=\"no-cache\">".to_string(),
        "<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html;charset=UTF-8\">".to_string(),
        "</HEAD>".to_string(),
        "<dl>".to_string(),
    ];
    // Group tables by their first title level ("Results for output field ...").
    let mut i = 0;
    while i < doc.tables.len() {
        let Some(title) = doc.tables[i].title.clone() else {
            i += 1;
            continue;
        };
        let levels: Vec<String> = title.split(" | ").map(|s| s.to_string()).collect();
        if levels.is_empty() {
            i += 1;
            continue;
        }
        lines.push(format!("<dt>{}</dt>", levels[0]));
        lines.push("<dd><dl>".to_string());
        // All tables in the oracle file case share the same first level; emit each second level.
        let first = levels[0].clone();
        while i < doc.tables.len() {
            let Some(t) = doc.tables[i].title.clone() else {
                i += 1;
                continue;
            };
            let lv: Vec<String> = t.split(" | ").map(|s| s.to_string()).collect();
            if lv.is_empty() || lv[0] != first {
                break;
            }
            if lv.len() >= 2 {
                lines.push(format!("<dt>{}</dt>", lv[1]));
                lines.push("<dd><dl>".to_string());
            }
            let table = &doc.tables[i];
            // Same alignment as the text file but without leading tabs; the first row shares
            // its line with `<pre>`.
            let label_width = table.rows.iter().map(|r| r[0].len()).max().unwrap_or(0);
            let count_width = table.rows.iter().map(|r| r[1].len()).max().unwrap_or(0);
            let pct_width = table.rows.iter().map(|r| r[2].len()).max().unwrap_or(0);
            for (ri, row) in table.rows.iter().enumerate() {
                let label_pad = label_width - row[0].len() + 12;
                let count_pad = count_width.saturating_sub(row[1].len());
                let pct_pad = pct_width.saturating_sub(row[2].len());
                let mut line = String::new();
                line.push_str(&row[0]);
                line.push_str(&" ".repeat(label_pad));
                line.push_str(&" ".repeat(count_pad));
                line.push_str(&row[1]);
                line.push_str("   ");
                line.push_str(&" ".repeat(pct_pad));
                line.push_str(&row[2]);
                if ri == 0 {
                    lines.push(format!("<pre>{line}"));
                } else {
                    lines.push(line);
                }
            }
            // The oracle <pre> block ends with spaces on the Total line then a line of spaces?
            // Actually the Total line is padded as above; no extra blank inside <pre>.
            lines.push("</pre>".to_string());
            if lv.len() >= 2 {
                lines.push("</dl>".to_string());
                lines.push("</dd>".to_string());
            }
            i += 1;
        }
        lines.push("</dl>".to_string());
        lines.push("</dd>".to_string());
    }
    lines.push("</dl>".to_string());
    lines.push("</HTML>".to_string());
    lines.join("\r\n")
}
