//! Ignored diagnostic: print our C5.0 tree beside every `TreeModel` Modeler exported
//! for the same builder, then diff the lines.
//!
//! `C50_LABEL` and `C50_CASE` select the corpus stream and its oracle directory.
//! Training goes through the stream unless `C50_WEIGHTS` is set (`field` uses the
//! table's `weight` field, `unit` uses none, anything else is a file with one weight
//! per training row). In that mode `C50_SEVERITY` (default 75), `C50_GLOBAL` (`0`/`1`,
//! default 1), `C50_MIN_CHILD` (default 2) and `C50_GROUP` (`0`/`1`, default 0)
//! override the direct `train_tree` call. A rejected training prints the error and
//! still prints Modeler's trees.
//!
//! ```text
//! cargo test -p dms-exec --features corpus --test c50_tree_diff -- --ignored --nocapture
//! ```
#![cfg(feature = "corpus")]

mod support;

use dms_algo::Dataset;
use dms_algo::c50::{C50Settings, train_tree};
use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_pmml::{BooleanOperator, Model, Node, Operator, Pmml, Predicate};
use dms_stream::Stream;
use dms_strfmt::{ArchiveLimits, read_archive};
use dms_types::{Role, Table, Value};
use serde_json::Value as Json;
use std::path::Path;
use std::sync::Arc;

const MAX_PRINT_DEPTH: usize = 64;
const MAX_PREDICATE_DEPTH: usize = 32;
const MAX_DIFF_CELLS: usize = 4_000_000;
const MAX_WEIGHT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_WEIGHTS: usize = 1_000_000;

struct Loaded {
    stream: Stream,
    context: ExecContext,
    uid: String,
    manifest: Json,
    modeler: Pmml,
}

struct TextDiff {
    structure_diff: String,
    structure: usize,
    exact_diff: String,
    exact: usize,
}

enum Edit<'a> {
    Ours(&'a str),
    Modeler(&'a str),
}

fn format_number(value: f64) -> String {
    value.to_string()
}

/// A predicate value that is a finite number is printed with [`format_number`].
/// Categories stay as stored text.
fn format_token(text: &str) -> String {
    match text.parse::<f64>() {
        Ok(value) if value.is_finite() => format_number(value),
        _ => text.to_string(),
    }
}

fn format_predicate(predicate: &Predicate) -> String {
    format_predicate_at(predicate, 0)
}

fn format_predicate_at(predicate: &Predicate, depth: usize) -> String {
    if depth > MAX_PREDICATE_DEPTH {
        return "...".to_string();
    }
    match predicate {
        Predicate::True => "root".to_string(),
        Predicate::False => "false".to_string(),
        Predicate::Simple {
            field,
            operator,
            value,
        } => format_simple(field, *operator, value.as_deref()),
        Predicate::SimpleSet {
            field,
            is_in,
            values,
        } => {
            let word = if *is_in { "in" } else { "not in" };
            let listed = values
                .iter()
                .map(|value| format!("\"{}\"", format_token(value)))
                .collect::<Vec<_>>()
                .join(" ");
            if listed.is_empty() {
                format!("{field} {word}")
            } else {
                format!("{field} {word} {listed}")
            }
        }
        Predicate::Compound {
            operator,
            predicates,
        } => {
            let word = match operator {
                BooleanOperator::And => "and",
                BooleanOperator::Or => "or",
                BooleanOperator::Xor => "xor",
                BooleanOperator::Surrogate => "surrogate",
            };
            let inner = predicates
                .iter()
                .map(|child| format_predicate_at(child, depth.saturating_add(1)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{word}({inner})")
        }
    }
}

fn format_simple(field: &str, operator: Operator, value: Option<&str>) -> String {
    let op = match operator {
        Operator::Equal => "=",
        Operator::NotEqual => "!=",
        Operator::LessThan => "<",
        Operator::LessOrEqual => "<=",
        Operator::GreaterThan => ">",
        Operator::GreaterOrEqual => ">=",
        Operator::IsMissing => "isMissing",
        Operator::IsNotMissing => "isNotMissing",
    };
    match (operator, value) {
        (Operator::IsMissing | Operator::IsNotMissing, _) => format!("{field} {op}"),
        (_, Some(value)) => format!("{field} {op} {}", format_token(value)),
        (_, None) => format!("{field} {op}"),
    }
}

fn format_node_line(node: &Node) -> String {
    let pred = format_predicate(&node.predicate);
    let count = match node.record_count {
        Some(value) => format_number(value),
        None => String::new(),
    };
    let dist = node
        .distribution
        .iter()
        .map(|item| format!("{}{}", item.value, format_number(item.record_count)))
        .collect::<Vec<_>>()
        .join(" ");
    let mut line = match node.score.as_deref() {
        Some(score) if !score.is_empty() => format!("{pred} -> {score} [{count}]"),
        _ => format!("{pred} -> [{count}]"),
    };
    if !dist.is_empty() {
        line.push(' ');
        line.push_str(&dist);
    }
    line
}

fn format_forest(roots: &[&Node]) -> String {
    let mut out = String::new();
    let many = roots.len() > 1;
    for (index, root) in roots.iter().enumerate() {
        if many {
            let number = index.saturating_add(1);
            out.push_str(&format!("=== tree {number}\n"));
        }
        write_node(root, 0, &mut out);
    }
    out
}

fn write_node(node: &Node, depth: usize, out: &mut String) {
    if depth > MAX_PRINT_DEPTH {
        push_indent(out, MAX_PRINT_DEPTH);
        out.push_str("...\n");
        return;
    }
    push_indent(out, depth);
    out.push_str(&format_node_line(node));
    out.push('\n');
    for child in &node.children {
        write_node(child, depth.saturating_add(1), out);
    }
}

fn push_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn mask_numbers(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        if let Some(end) = number_end(&chars, index) {
            out.push('N');
            index = end;
        } else if let Some(ch) = chars.get(index) {
            out.push(*ch);
            index = index.saturating_add(1);
        } else {
            break;
        }
    }
    out
}

fn number_end(chars: &[char], start: usize) -> Option<usize> {
    let first = *chars.get(start)?;
    let mut index = start;
    if first == '-' {
        if !chars.get(start.saturating_add(1))?.is_ascii_digit() {
            return None;
        }
        index = index.saturating_add(1);
    } else if !first.is_ascii_digit() {
        return None;
    }
    index = take_digits(chars, index);
    if chars.get(index) == Some(&'.')
        && chars
            .get(index.saturating_add(1))
            .is_some_and(|ch| ch.is_ascii_digit())
    {
        index = take_digits(chars, index.saturating_add(1));
    }
    if matches!(chars.get(index), Some('e' | 'E')) {
        let mut exponent = index.saturating_add(1);
        if matches!(chars.get(exponent), Some('+' | '-')) {
            exponent = exponent.saturating_add(1);
        }
        if chars.get(exponent).is_some_and(|ch| ch.is_ascii_digit()) {
            index = take_digits(chars, exponent);
        }
    }
    if index == start { None } else { Some(index) }
}

fn take_digits(chars: &[char], mut index: usize) -> usize {
    while chars.get(index).is_some_and(|ch| ch.is_ascii_digit()) {
        index = index.saturating_add(1);
    }
    index
}

fn mask_numbers_in_text(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        out.push_str(&mask_numbers(line));
        out.push('\n');
    }
    out
}

fn compare_texts(ours: &str, modeler: &str) -> TextDiff {
    let masked_ours = mask_numbers_in_text(ours);
    let masked_modeler = mask_numbers_in_text(modeler);
    let (structure_diff, structure) = diff_report(&masked_ours, &masked_modeler);
    let (exact_diff, exact) = diff_report(ours, modeler);
    TextDiff {
        structure_diff,
        structure,
        exact_diff,
        exact,
    }
}

fn summary_line(structure: usize, exact: usize) -> String {
    format!("structure: {structure} differing lines, exact: {exact} differing lines")
}

fn diff_report(ours: &str, modeler: &str) -> (String, usize) {
    let left: Vec<&str> = ours.lines().collect();
    let right: Vec<&str> = modeler.lines().collect();
    let edits = line_edits(&left, &right);
    let mut text = String::new();
    for edit in &edits {
        match edit {
            Edit::Ours(line) => {
                text.push_str("< ");
                text.push_str(line);
                text.push('\n');
            }
            Edit::Modeler(line) => {
                text.push_str("> ");
                text.push_str(line);
                text.push('\n');
            }
        }
    }
    (text, edits.len())
}

/// Longest common subsequence. Equal scores prefer a Modeler-only line so a
/// replacement is printed as `<` then `>`.
fn line_edits<'a>(ours: &[&'a str], modeler: &[&'a str]) -> Vec<Edit<'a>> {
    let rows = ours.len();
    let cols = modeler.len();
    let width = cols.saturating_add(1);
    let height = rows.saturating_add(1);
    let Some(cells) = height.checked_mul(width) else {
        return zip_edits(ours, modeler);
    };
    if cells > MAX_DIFF_CELLS {
        return zip_edits(ours, modeler);
    }
    let mut lcs = vec![0u32; cells];
    for row in 1..=rows {
        for col in 1..=cols {
            let value = if ours.get(row - 1) == modeler.get(col - 1) {
                lcs_at(&lcs, width, row - 1, col - 1).saturating_add(1)
            } else {
                lcs_at(&lcs, width, row - 1, col).max(lcs_at(&lcs, width, row, col - 1))
            };
            if let Some(slot) = lcs_slot(&mut lcs, width, row, col) {
                *slot = value;
            }
        }
    }
    let mut reversed = Vec::new();
    let mut row = rows;
    let mut col = cols;
    let mut guard = rows.saturating_add(cols).saturating_add(1);
    while (row > 0 || col > 0) && guard > 0 {
        guard = guard.saturating_sub(1);
        if row > 0 && col > 0 && ours.get(row - 1) == modeler.get(col - 1) {
            row -= 1;
            col -= 1;
            continue;
        }
        let left = if col > 0 {
            lcs_at(&lcs, width, row, col - 1)
        } else {
            0
        };
        let up = if row > 0 {
            lcs_at(&lcs, width, row - 1, col)
        } else {
            0
        };
        if col > 0 && (row == 0 || left >= up) {
            if let Some(line) = modeler.get(col - 1) {
                reversed.push(Edit::Modeler(line));
            }
            col -= 1;
        } else if row > 0 {
            if let Some(line) = ours.get(row - 1) {
                reversed.push(Edit::Ours(line));
            }
            row -= 1;
        }
    }
    reversed.reverse();
    reversed
}

fn lcs_at(table: &[u32], width: usize, row: usize, col: usize) -> u32 {
    row.checked_mul(width)
        .and_then(|start| start.checked_add(col))
        .and_then(|index| table.get(index).copied())
        .unwrap_or(0)
}

fn lcs_slot(table: &mut [u32], width: usize, row: usize, col: usize) -> Option<&mut u32> {
    let index = row.checked_mul(width)?.checked_add(col)?;
    table.get_mut(index)
}

fn zip_edits<'a>(ours: &[&'a str], modeler: &[&'a str]) -> Vec<Edit<'a>> {
    let mut edits = Vec::new();
    let mut index = 0;
    let len = ours.len().max(modeler.len());
    while index < len {
        match (ours.get(index), modeler.get(index)) {
            (Some(left), Some(right)) if left == right => {}
            (Some(left), Some(right)) => {
                edits.push(Edit::Ours(left));
                edits.push(Edit::Modeler(right));
            }
            (Some(left), None) => edits.push(Edit::Ours(left)),
            (None, Some(right)) => edits.push(Edit::Modeler(right)),
            (None, None) => {}
        }
        index = index.saturating_add(1);
    }
    edits
}

fn roots_of(model: &Model) -> Vec<&Node> {
    let mut roots = Vec::new();
    collect_roots(model, 0, &mut roots);
    roots
}

fn collect_roots<'a>(model: &'a Model, depth: usize, out: &mut Vec<&'a Node>) {
    if depth > 8 {
        return;
    }
    match model {
        Model::Tree(tree) => out.push(&tree.root),
        Model::Mining(mining) => {
            for segment in &mining.segments {
                collect_roots(&segment.model, depth.saturating_add(1), out);
            }
        }
        _ => {}
    }
}

fn model_kind(model: &Model) -> &'static str {
    match model {
        Model::Tree(_) => "tree",
        Model::RuleSet(_) => "ruleset",
        Model::Mining(_) => "mining",
        Model::Clustering(_) => "clustering",
        Model::GeneralRegression(_) => "generalRegression",
        Model::Regression(_) => "regression",
        Model::Unsupported(_) => "unsupported",
    }
}

fn load_case(label: &str, case_name: &str) -> Result<Loaded, String> {
    let root = support::repo_root();
    let bytes = std::fs::read(root.join(format!("corpus/{label}/{case_name}.str")))
        .map_err(|error| format!("cannot read corpus/{label}/{case_name}.str: {error}"))?;
    let archive = read_archive(&bytes, &ArchiveLimits::default())
        .map_err(|error| format!("cannot read stream: {error}"))?;
    let stream = Stream::from_document(&archive.document)
        .map_err(|error| format!("cannot read stream: {error}"))?;
    let uid = builder_uid(&stream)?;
    let context = ExecContext {
        files: FileResolver {
            data_dirs: vec![
                root.join(format!("corpus/{label}")),
                root.join("corpus/ibm-demo-data"),
            ],
        },
        attachments: Arc::new(
            archive
                .attachments
                .into_iter()
                .map(|attachment| (attachment.name, Arc::<[u8]>::from(attachment.bytes)))
                .collect(),
        ),
        stream_properties: stream.properties.clone().map(Arc::new),
        ..ExecContext::default()
    };
    let dir = root.join("oracle").join(label).join(case_name);
    let manifest = read_manifest(&dir)?;
    let builder_label = builder_label(&manifest, &uid)?;
    let file = model_file(&manifest, builder_label)?;
    let name = Path::new(file)
        .file_name()
        .and_then(|item| item.to_str())
        .ok_or_else(|| format!("manifest file has no name: {file}"))?;
    let path = dir.join("models").join(name);
    let xml = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let modeler =
        Pmml::parse(&xml).map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
    if roots_of(&modeler.model).is_empty() {
        return Err(format!(
            "Modeler export has no tree ({})",
            model_kind(&modeler.model)
        ));
    }
    Ok(Loaded {
        stream,
        context,
        uid,
        manifest,
        modeler,
    })
}

fn builder_uid(stream: &Stream) -> Result<String, String> {
    let mut found = Vec::new();
    for node in &stream.diagram.nodes {
        if node.type_identifier() == "BuildC5RuleNode" {
            found.push(node.uid.clone());
        }
    }
    let [uid] = found.as_slice() else {
        return Err(format!("expected one C5.0 builder, found {}", found.len()));
    };
    Ok(uid.clone())
}

fn read_manifest(dir: &Path) -> Result<Json, String> {
    let path = dir.join("manifest.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("cannot parse {}: {error}", path.display()))
}

fn builder_label<'a>(manifest: &'a Json, uid: &str) -> Result<&'a str, String> {
    let runs = manifest
        .get("runs")
        .and_then(Json::as_array)
        .ok_or("manifest missing runs")?;
    let run = runs
        .iter()
        .find(|entry| {
            entry
                .get("node")
                .and_then(|node| node.get("id"))
                .and_then(Json::as_str)
                == Some(uid)
        })
        .ok_or_else(|| format!("manifest has no run for builder {uid}"))?;
    run.get("node")
        .and_then(|node| node.get("label"))
        .and_then(Json::as_str)
        .ok_or_else(|| format!("builder run {uid} is missing a label"))
}

fn model_file<'a>(manifest: &'a Json, builder_label: &str) -> Result<&'a str, String> {
    let models = manifest
        .get("models")
        .and_then(Json::as_array)
        .ok_or("manifest missing models")?;
    let model = models
        .iter()
        .find(|entry| entry.get("label").and_then(Json::as_str) == Some(builder_label))
        .ok_or_else(|| format!("manifest has no model labeled {builder_label:?}"))?;
    model
        .get("file")
        .and_then(Json::as_str)
        .ok_or_else(|| "model entry is missing file".to_string())
}

fn train(case: &Loaded) -> Result<Pmml, String> {
    match std::env::var("C50_WEIGHTS") {
        Err(std::env::VarError::NotPresent) => train_stream(case),
        Err(error) => Err(format!("C50_WEIGHTS: {error}")),
        Ok(mode) => train_weighted(case, &mode),
    }
}

fn train_stream(case: &Loaded) -> Result<Pmml, String> {
    let results = Plan::new(&case.stream).evaluate(&[&case.uid], &case.context);
    match results.get(&case.uid) {
        Some(Ok(NodeOutput::Model(model))) => Ok((**model).clone()),
        Some(Ok(_)) => Err("C5.0 builder did not produce a model".into()),
        Some(Err(error)) => Err(error.to_string()),
        None => Err("C5.0 builder was not evaluated".into()),
    }
}

fn train_weighted(case: &Loaded, mode: &str) -> Result<Pmml, String> {
    let (table, weight_field) = prepare_weighted_table(case, mode)?;
    let (target, inputs) = dataset_columns(table.as_ref())?;
    let settings = weighted_settings(weight_field)?;
    train_tree(
        &Dataset {
            table: table.as_ref(),
            target,
            inputs: &inputs,
            frequency: None,
        },
        &settings,
    )
    .map_err(|error| error.to_string())
}

fn prepare_weighted_table(
    case: &Loaded,
    mode: &str,
) -> Result<(Arc<Table>, Option<usize>), String> {
    let table = input_table(case)?;
    match mode {
        "field" => {
            let position = table
                .model()
                .position("weight")
                .ok_or_else(|| "table has no weight field".to_string())?;
            Ok((table, Some(position)))
        }
        "unit" => Ok((table, None)),
        path => {
            let weights = read_weights(path)?;
            let replaced = replace_weights(table.as_ref(), &weights)?;
            let position = replaced
                .model()
                .position("weight")
                .ok_or_else(|| "table has no weight field".to_string())?;
            Ok((Arc::new(replaced), Some(position)))
        }
    }
}

fn input_table(case: &Loaded) -> Result<Arc<Table>, String> {
    let source = type_source(&case.manifest, &case.uid)?;
    let results = Plan::new(&case.stream).evaluate(&[source.as_str()], &case.context);
    match results.get(&source) {
        Some(Ok(NodeOutput::Data(table))) => Ok(Arc::clone(table)),
        Some(Ok(_)) => Err(format!("Type node {source} did not produce a table")),
        Some(Err(error)) => Err(format!("Type node {source}: {error}")),
        None => Err(format!("Type node {source} was not evaluated")),
    }
}

fn type_source(manifest: &Json, builder: &str) -> Result<String, String> {
    let data = manifest
        .get("data")
        .and_then(Json::as_array)
        .ok_or("manifest missing data")?;
    let entry = data
        .iter()
        .find(|item| {
            item.get("type").and_then(Json::as_str) == Some("type")
                && item
                    .get("terminal")
                    .and_then(|terminal| terminal.get("id"))
                    .and_then(Json::as_str)
                    == Some(builder)
        })
        .ok_or("manifest has no Type export feeding the C5.0 builder")?;
    entry
        .get("id")
        .and_then(Json::as_str)
        .map(str::to_string)
        .ok_or_else(|| "Type export is missing an id".to_string())
}

fn dataset_columns(table: &Table) -> Result<(usize, Vec<usize>), String> {
    let mut targets = Vec::new();
    let mut inputs = Vec::new();
    for (index, field) in table.model().fields().iter().enumerate() {
        match field.role {
            Role::Target => targets.push(index),
            Role::Input => inputs.push(index),
            _ => {}
        }
    }
    let [target] = targets.as_slice() else {
        return Err(format!(
            "C5.0 needs exactly one target, found {}",
            targets.len()
        ));
    };
    Ok((*target, inputs))
}

fn weighted_settings(weight_field: Option<usize>) -> Result<C50Settings, String> {
    Ok(C50Settings {
        weight_field,
        pruning_severity: env_f64("C50_SEVERITY", 75.0)?,
        global_pruning: env_flag("C50_GLOBAL", true)?,
        min_child_records: env_usize("C50_MIN_CHILD", 2)?,
        group_symbolics: env_flag("C50_GROUP", false)?,
        ..C50Settings::default()
    })
}

fn env_f64(name: &str, default: f64) -> Result<f64, String> {
    match std::env::var(name) {
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(format!("{name}: {error}")),
        Ok(text) => {
            let value = text
                .parse::<f64>()
                .map_err(|error| format!("{name}={text:?}: {error}"))?;
            if !value.is_finite() {
                return Err(format!("{name}={text:?} is not finite"));
            }
            Ok(value)
        }
    }
}

fn env_usize(name: &str, default: usize) -> Result<usize, String> {
    match std::env::var(name) {
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(format!("{name}: {error}")),
        Ok(text) => text
            .parse::<usize>()
            .map_err(|error| format!("{name}={text:?}: {error}")),
    }
}

fn env_flag(name: &str, default: bool) -> Result<bool, String> {
    match std::env::var(name) {
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(format!("{name}: {error}")),
        Ok(text) => match text.as_str() {
            "0" => Ok(false),
            "1" => Ok(true),
            _ => Err(format!("{name} must be 0 or 1, got {text:?}")),
        },
    }
}

fn read_weights(path: &str) -> Result<Vec<f64>, String> {
    let meta =
        std::fs::metadata(path).map_err(|error| format!("cannot read weights {path}: {error}"))?;
    if meta.len() > MAX_WEIGHT_BYTES {
        return Err(format!(
            "weight file {path} exceeds {MAX_WEIGHT_BYTES} bytes"
        ));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read weights {path}: {error}"))?;
    let mut weights = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            return Err(format!(
                "weight file {path} has a blank line at {}",
                index.saturating_add(1)
            ));
        }
        let weight = line.trim().parse::<f64>().map_err(|error| {
            format!(
                "weight file {path} line {}: {error}",
                index.saturating_add(1)
            )
        })?;
        weights.push(weight);
        if weights.len() > MAX_WEIGHTS {
            return Err(format!("weight file {path} exceeds {MAX_WEIGHTS} values"));
        }
    }
    Ok(weights)
}

fn replace_weights(table: &Table, weights: &[f64]) -> Result<Table, String> {
    let position = table
        .model()
        .position("weight")
        .ok_or_else(|| "table has no weight field".to_string())?;
    if weights.len() != table.row_count() {
        return Err(format!(
            "weight file has {} values, training table has {} rows",
            weights.len(),
            table.row_count()
        ));
    }
    let mut rows = table.rows().to_vec();
    for (index, weight) in weights.iter().enumerate() {
        let Some(row) = rows.get_mut(index) else {
            return Err(format!("missing training row {index}"));
        };
        let Some(cell) = row.get_mut(position) else {
            return Err(format!("weight field is outside row {index}"));
        };
        *cell = Value::real(*weight);
    }
    Table::new(table.model().clone(), rows).map_err(|error| error.to_string())
}

fn ours_text(case: &Loaded) -> (String, usize) {
    match train(case) {
        Ok(pmml) => {
            let roots = roots_of(&pmml.model);
            let count = roots.len();
            if count == 0 {
                println!(
                    "training error: model has no tree ({})",
                    model_kind(&pmml.model)
                );
                (String::new(), 0)
            } else {
                (format_forest(&roots), count)
            }
        }
        Err(error) => {
            println!("training error: {error}");
            (String::new(), 0)
        }
    }
}

#[test]
#[ignore = "diagnostic print of our C5.0 tree against Modeler; needs C50_LABEL and C50_CASE"]
fn c50_tree_diff() {
    let label = match std::env::var("C50_LABEL") {
        Ok(value) if !value.is_empty() => value,
        _ => panic!("set C50_LABEL"),
    };
    let case_name = match std::env::var("C50_CASE") {
        Ok(value) if !value.is_empty() => value,
        _ => panic!("set C50_CASE"),
    };
    let loaded = match load_case(&label, &case_name) {
        Ok(loaded) => loaded,
        Err(error) => panic!("{error}"),
    };
    println!("case: {label}/{case_name}");
    let (ours, ours_count) = ours_text(&loaded);
    let modeler_roots = roots_of(&loaded.modeler.model);
    let modeler = format_forest(&modeler_roots);
    println!("ours trees: {ours_count}");
    println!("modeler trees: {}", modeler_roots.len());
    println!("--- ours");
    print!("{ours}");
    println!("--- modeler");
    print!("{modeler}");
    let compared = compare_texts(&ours, &modeler);
    println!("diff: '<' ours, '>' Modeler");
    println!("--- structure");
    print!("{}", compared.structure_diff);
    println!("--- exact");
    print!("{}", compared.exact_diff);
    println!("{}", summary_line(compared.structure, compared.exact));
}

#[cfg(test)]
mod tests {
    use super::{
        compare_texts, diff_report, format_forest, format_node_line, format_number,
        format_predicate, mask_numbers, summary_line, zip_edits,
    };
    use dms_pmml::{Node, Operator, Predicate, ScoreDistribution};

    fn leaf(predicate: Predicate, score: &str, count: f64, dist: &[(&str, f64)]) -> Node {
        Node {
            id: None,
            score: Some(score.to_string()),
            record_count: Some(count),
            default_child: None,
            predicate,
            distribution: dist
                .iter()
                .map(|(value, record_count)| ScoreDistribution {
                    value: (*value).to_string(),
                    record_count: *record_count,
                    probability: None,
                    confidence: None,
                })
                .collect(),
            children: Vec::new(),
        }
    }

    #[test]
    fn predicate_text_uses_the_compact_form() {
        assert_eq!(format_predicate(&Predicate::True), "root");
        assert_eq!(
            format_predicate(&Predicate::Simple {
                field: "x".into(),
                operator: Operator::LessOrEqual,
                value: Some("3".into()),
            }),
            "x <= 3"
        );
        assert_eq!(
            format_predicate(&Predicate::Simple {
                field: "x".into(),
                operator: Operator::GreaterThan,
                value: Some("3.0".into()),
            }),
            "x > 3"
        );
        assert_eq!(
            format_predicate(&Predicate::Simple {
                field: "cat".into(),
                operator: Operator::Equal,
                value: Some("red".into()),
            }),
            "cat = red"
        );
        assert_eq!(
            format_predicate(&Predicate::SimpleSet {
                field: "cat".into(),
                is_in: true,
                values: vec!["a".into(), "b".into()],
            }),
            "cat in \"a\" \"b\""
        );
        assert_eq!(
            format_predicate(&Predicate::Simple {
                field: "x".into(),
                operator: Operator::IsMissing,
                value: None,
            }),
            "x isMissing"
        );
    }

    #[test]
    fn numbers_print_with_display() {
        assert_eq!(format_number(10.0), format!("{}", 10.0_f64));
        assert_eq!(format_number(0.0), "0");
        assert_eq!(format_number(3.5), "3.5");
        assert_eq!(format_number(-1.5), "-1.5");
        assert_eq!(format_number(384.0), "384");
    }

    #[test]
    fn node_line_keeps_counts_and_two_space_indent() {
        let mut root = leaf(Predicate::True, "A", 10.0, &[("A", 10.0)]);
        root.children.push(leaf(
            Predicate::Simple {
                field: "x".into(),
                operator: Operator::LessOrEqual,
                value: Some("3".into()),
            },
            "A",
            4.0,
            &[("A", 4.0)],
        ));
        assert_eq!(format_node_line(&root), "root -> A [10] A10");
        assert_eq!(
            format_forest(&[&root]),
            "root -> A [10] A10\n  x <= 3 -> A [4] A4\n"
        );
    }

    #[test]
    fn numbers_in_a_line_become_n() {
        assert_eq!(mask_numbers("x <= 3"), "x <= N");
        assert_eq!(mask_numbers("x > 3"), "x > N");
        assert_eq!(mask_numbers("cat = red"), "cat = red");
        assert_eq!(mask_numbers("cat in \"a\" \"b\""), "cat in \"a\" \"b\"");
        assert_eq!(
            mask_numbers("root -> A [384] A183 B96 C105"),
            "root -> A [N] AN BN CN"
        );
        assert_eq!(mask_numbers("x <= -1.25e-3"), "x <= N");
        assert_eq!(mask_numbers("A117.5032"), "AN");
    }

    #[test]
    fn line_diff_follows_the_common_subsequence() {
        let (text, count) = diff_report("a\nb\nc\n", "b\nc\nd\n");
        assert_eq!(count, 2);
        assert_eq!(text, "< a\n> d\n");
        let (text, count) = diff_report("a\nb\n", "a\nx\n");
        assert_eq!(count, 2);
        assert_eq!(text, "< b\n> x\n");
        let (text, count) = diff_report("a\nb\n", "a\n");
        assert_eq!(count, 1);
        assert_eq!(text, "< b\n");
        let (text, count) = diff_report("a\n", "a\nb\n");
        assert_eq!(count, 1);
        assert_eq!(text, "> b\n");
        let (text, count) = diff_report("a\n", "a\n");
        assert_eq!(count, 0);
        assert_eq!(text, "");
    }

    #[test]
    fn zip_fallback_pairs_lines_by_index() {
        let edits = zip_edits(&["a", "b", "c"], &["b", "c", "d"]);
        assert_eq!(edits.len(), 6);
    }

    #[test]
    fn masking_numbers_drops_them_from_the_structure_count() {
        let ours = "root -> A [10] A10\n  x <= 3 -> A [4] A4\n";
        let modeler = "root -> A [10] A10\n  x <= 3.5 -> A [4] A4\n";
        let compared = compare_texts(ours, modeler);
        assert_eq!(compared.structure, 0);
        assert_eq!(compared.exact, 2);
        assert_eq!(
            summary_line(compared.structure, compared.exact),
            "structure: 0 differing lines, exact: 2 differing lines"
        );
        let shifted = compare_texts(
            "root -> A [10] A10\n  x <= 3 -> A [4] A4\n",
            "root -> A [11] A11\n  cat = red -> A [4] A4\n",
        );
        assert_eq!(shifted.structure, 2);
        assert_eq!(shifted.exact, 4);
    }
}
