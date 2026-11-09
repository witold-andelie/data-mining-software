//! Readable, presentation-independent summaries of PMML models.
//!
//! File strings are limited to 200 Unicode scalar values, including the ellipsis.
//! Composite expressions have a separate size/work budget. Trees are walked iteratively;
//! indentation stops growing at 512 levels, predicates at 64, and ensembles at four.

use crate::{
    Activation, AssociationModel, BooleanOperator, ClusteringModel, GeneralModelType,
    GeneralRegressionModel, MiningFunction, MiningModel, Model, ModelHeader, NeuralNetworkModel,
    Node, Operator, Pmml, Predicate, RegressionModel, RuleSetModel, SequenceItemset, SequenceModel,
    TreeModel, Usage,
};

const MAX_TEXT: usize = 200;
const MAX_EXPRESSION: usize = 16_384;
const MAX_TERMS: usize = 1_000;
const MAX_PREDICATE_DEPTH: usize = 64;
const MAX_ENSEMBLE_DEPTH: usize = 4;
const MAX_TREE_LINES: usize = 2_000;
const MAX_COEFFICIENTS: usize = 1_000;

#[derive(Debug, Clone, PartialEq)]
pub struct ModelSummary {
    /// Plain-English model kind, followed by the algorithm name when available.
    pub kind: String,
    /// Model name from the header.
    pub name: Option<String>,
    /// Mining function from the header, in readable form.
    pub function: Option<String>,
    /// Predicted/target fields in mining-schema order.
    pub targets: Vec<String>,
    /// Active fields in mining-schema order.
    pub inputs: Vec<String>,
    pub sections: Vec<SummarySection>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SummarySection {
    pub heading: String,
    pub body: SectionBody,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SectionBody {
    /// Label/value pairs.
    Facts(Vec<(String, String)>),
    /// Preformatted lines, for example an indented tree.
    Lines(Vec<String>),
    Table {
        header: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    /// Number of items displayed and total number available in the preceding section.
    Truncated { shown: usize, total: usize },
}

/// Summarize a model without scoring it, accessing files, or changing the document.
pub fn summarize(pmml: &Pmml) -> ModelSummary {
    let header = pmml.model.header();
    let mut sections = Vec::new();
    model_sections(&pmml.model, 0, &mut sections);
    ModelSummary {
        kind: model_kind(&pmml.model),
        name: header.and_then(|h| h.name.as_deref().map(file_text)),
        function: header.map(|h| function_text(&h.function)),
        targets: schema_names(header, Usage::Target),
        inputs: schema_names(header, Usage::Active),
        sections,
    }
}

fn schema_names(header: Option<&ModelHeader>, usage: Usage) -> Vec<String> {
    header
        .into_iter()
        .flat_map(|h| &h.mining_schema)
        .filter(|field| field.usage == usage)
        .map(|field| file_text(&field.name))
        .collect()
}

fn file_text(value: &str) -> String {
    let mut chars = value.chars();
    let mut out: String = chars.by_ref().take(MAX_TEXT).collect();
    if chars.next().is_some() {
        out.pop();
        out.push('…');
    }
    out
}

fn function_text(function: &MiningFunction) -> String {
    match function {
        MiningFunction::Classification => "classification".into(),
        MiningFunction::Regression => "regression".into(),
        MiningFunction::Clustering => "clustering".into(),
        MiningFunction::Other(name) => match name.as_str() {
            "associationRules" => "association rules".into(),
            "sequences" => "sequence rules".into(),
            "timeSeries" => "time series".into(),
            _ => file_text(name),
        },
    }
}

fn model_kind(model: &Model) -> String {
    let kind = match model {
        Model::Tree(_) => "Decision tree",
        Model::RuleSet(_) => "Rule set",
        Model::Regression(m) => {
            if m.header.function == MiningFunction::Classification {
                "Logistic regression"
            } else {
                "Linear regression"
            }
        }
        Model::GeneralRegression(m) => match m.model_type {
            GeneralModelType::Regression | GeneralModelType::GeneralLinear => "Linear regression",
            GeneralModelType::MultinomialLogistic | GeneralModelType::OrdinalMultinomial => {
                "Logistic regression"
            }
            _ => "Generalized linear model",
        },
        Model::Clustering(_) => "Cluster model",
        Model::Neural(_) => "Neural network",
        Model::Mining(_) => "Ensemble",
        Model::Association(_) => "Association rules",
        Model::Sequence(_) => "Sequence rules",
        Model::TimeSeries { .. } => "Time series model",
        Model::Unsupported(element) => {
            return format!("Unsupported model ({})", file_text(element));
        }
    };
    match model.header().and_then(|h| h.algorithm.as_deref()) {
        Some(algorithm) => format!("{kind} ({})", file_text(algorithm)),
        None => kind.into(),
    }
}

/// Format a display number with up to six fractional decimal places (also in the
/// scientific mantissa). Negative zero is displayed as zero.
pub fn number_text(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-∞"
        } else {
            "∞"
        }
        .into();
    }
    if value == 0.0 {
        return "0".into();
    }
    if value.abs() >= 1e9 || value.abs() < 1e-4 {
        let scientific = format!("{value:.6e}");
        if let Some((mantissa, exponent)) = scientific.split_once('e') {
            return format!("{}e{exponent}", trim_decimals(mantissa));
        }
        return scientific;
    }
    trim_decimals(&format!("{value:.6}"))
}

fn trim_decimals(value: &str) -> String {
    if value.contains('.') {
        value.trim_end_matches('0').trim_end_matches('.').into()
    } else {
        value.into()
    }
}

/// Render a predicate, limiting nesting to 64 levels and work to 1,000 terms.
/// Textual values are quoted; numeric spellings from the document are preserved.
pub fn predicate_text(predicate: &Predicate) -> String {
    let mut remaining = MAX_TERMS;
    predicate_at(predicate, 0, &mut remaining)
}

fn literal(value: &str) -> String {
    let text = file_text(value);
    if value.parse::<f64>().is_ok() {
        text
    } else {
        format!("\"{text}\"")
    }
}

// Bound both iteration and allocation when composing large sets, interactions, or rules.
fn join_text(parts: impl IntoIterator<Item = String>, separator: &str) -> String {
    let mut out = String::new();
    for (index, part) in parts.into_iter().enumerate() {
        if index >= MAX_TERMS
            || out
                .len()
                .saturating_add(part.len())
                .saturating_add(separator.len())
                > MAX_EXPRESSION
        {
            out.push('…');
            break;
        }
        if index > 0 {
            out.push_str(separator);
        }
        out.push_str(&part);
    }
    out
}

fn predicate_at(predicate: &Predicate, depth: usize, remaining: &mut usize) -> String {
    if depth >= MAX_PREDICATE_DEPTH || *remaining == 0 {
        return "…".into();
    }
    *remaining = remaining.saturating_sub(1);
    match predicate {
        Predicate::True => "true".into(),
        Predicate::False => "false".into(),
        Predicate::Simple {
            field,
            operator,
            value,
        } => {
            let field = file_text(field);
            let operator = match operator {
                Operator::Equal => "=",
                Operator::NotEqual => "!=",
                Operator::LessThan => "<",
                Operator::LessOrEqual => "<=",
                Operator::GreaterThan => ">",
                Operator::GreaterOrEqual => ">=",
                Operator::IsMissing => return format!("{field} is missing"),
                Operator::IsNotMissing => return format!("{field} is not missing"),
            };
            format!(
                "{field} {operator} {}",
                value.as_deref().map_or_else(String::new, literal)
            )
        }
        Predicate::SimpleSet {
            field,
            is_in,
            values,
        } => {
            let values = join_text(values.iter().map(|value| literal(value)), ", ");
            let operator = if *is_in { "in" } else { "not in" };
            format!("{} {operator} {{{values}}}", file_text(field))
        }
        Predicate::Compound {
            operator,
            predicates,
        } => {
            let separator = match operator {
                BooleanOperator::And => " and ",
                BooleanOperator::Or => " or ",
                BooleanOperator::Xor => " xor ",
                BooleanOperator::Surrogate => " surrogate ",
            };
            let mut parts = Vec::new();
            for (index, child) in predicates
                .iter()
                .take(MAX_TERMS.saturating_add(1))
                .enumerate()
            {
                parts.push(predicate_at(child, depth.saturating_add(1), remaining));
                if *remaining == 0 {
                    if index.saturating_add(1) < predicates.len() {
                        parts.push("…".into());
                    }
                    break;
                }
            }
            let text = join_text(parts, separator);
            if depth > 0 { format!("({text})") } else { text }
        }
    }
}

fn facts(sections: &mut Vec<SummarySection>, heading: &str, values: Vec<(&str, String)>) {
    if !values.is_empty() {
        sections.push(SummarySection {
            heading: heading.into(),
            body: SectionBody::Facts(values.into_iter().map(|(k, v)| (k.into(), v)).collect()),
        });
    }
}

fn table(
    sections: &mut Vec<SummarySection>,
    heading: &str,
    header: Vec<String>,
    rows: Vec<Vec<String>>,
) {
    if !rows.is_empty() {
        sections.push(SummarySection {
            heading: heading.into(),
            body: SectionBody::Table { header, rows },
        });
    }
}

fn truncated(sections: &mut Vec<SummarySection>, heading: &str, shown: usize, total: usize) {
    if shown < total {
        sections.push(SummarySection {
            heading: heading.into(),
            body: SectionBody::Truncated { shown, total },
        });
    }
}

fn columns(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).into()).collect()
}

fn optional_number(value: Option<f64>) -> String {
    value.map_or_else(String::new, number_text)
}

fn model_sections(model: &Model, depth: usize, sections: &mut Vec<SummarySection>) {
    let start = sections.len();
    match model {
        Model::Tree(m) => tree_sections(m, sections),
        Model::RuleSet(m) => rule_sections(m, sections),
        Model::Regression(m) => regression_sections(m, sections),
        Model::GeneralRegression(m) => general_sections(m, sections),
        Model::Clustering(m) => cluster_sections(m, sections),
        Model::Neural(m) => neural_sections(m, sections),
        Model::Mining(m) => mining_sections(m, depth, sections),
        Model::Association(m) => association_sections(m, sections),
        Model::Sequence(m) => sequence_sections(m, sections),
        Model::TimeSeries { header, .. } => {
            let mut values = vec![("Function", function_text(&header.function))];
            if let Some(name) = &header.name {
                values.push(("Name", file_text(name)));
            }
            if let Some(algorithm) = &header.algorithm {
                values.push(("Algorithm", file_text(algorithm)));
            }
            facts(sections, "Time series", values);
        }
        Model::Unsupported(element) => {
            facts(sections, "Model", vec![("Element", file_text(element))])
        }
    }
    // Empty but supported models still have a useful, nonempty summary (including in ensembles).
    if sections.len() == start {
        facts(sections, "Model", vec![("Kind", model_kind(model))]);
    }
}

fn tree_sections(model: &TreeModel, sections: &mut Vec<SummarySection>) {
    let mut stack = vec![(std::slice::from_ref(&model.root).iter(), 0usize)];
    let (mut nodes, mut leaves, mut max_depth) = (0usize, 0usize, 0usize);
    let mut lines = Vec::new();
    while let Some((siblings, depth)) = stack.last_mut() {
        let Some(node) = siblings.next() else {
            stack.pop();
            continue;
        };
        let depth = *depth;
        nodes = nodes.saturating_add(1);
        max_depth = max_depth.max(depth);
        if node.children.is_empty() {
            leaves = leaves.saturating_add(1);
        }
        if lines.len() < MAX_TREE_LINES {
            let indent = "  ".repeat(depth.min(512));
            let continuation = if depth > 512 { "… " } else { "" };
            let mut details = Vec::new();
            if let Some(count) = node.record_count {
                details.push(format!("n = {}", number_text(count)));
            }
            if let Some(probability) = score_probability(node) {
                details.push(format!("{:.1}%", probability * 100.0));
            }
            let details = if details.is_empty() {
                String::new()
            } else {
                format!(" ({})", details.join(", "))
            };
            lines.push(format!(
                "{indent}{continuation}{} → {}{details}",
                predicate_text(&node.predicate),
                node.score.as_deref().map_or_else(String::new, file_text)
            ));
        }
        if !node.children.is_empty() {
            stack.push((node.children.iter(), depth.saturating_add(1)));
        }
    }
    facts(
        sections,
        "Tree",
        vec![
            ("Nodes", nodes.to_string()),
            ("Leaves", leaves.to_string()),
            ("Depth", max_depth.to_string()),
        ],
    );
    sections.push(SummarySection {
        heading: "Tree".into(),
        body: SectionBody::Lines(lines),
    });
    truncated(sections, "Tree", nodes.min(MAX_TREE_LINES), nodes);
}

fn score_probability(node: &Node) -> Option<f64> {
    let score = node.score.as_deref()?;
    let distribution = node.distribution.iter().find(|d| d.value == score)?;
    let valid = |value: &f64| (0.0..=1.0).contains(value);
    if let Some(value) = distribution
        .confidence
        .filter(valid)
        .or(distribution.probability.filter(valid))
    {
        return Some(value);
    }
    let total: f64 = node.distribution.iter().map(|d| d.record_count).sum();
    if total.is_finite() && total > 0.0 {
        let value = distribution.record_count / total;
        return Some(value).filter(valid);
    }
    None
}

fn rule_sections(model: &RuleSetModel, sections: &mut Vec<SummarySection>) {
    if let Some(score) = &model.default_score {
        let mut values = vec![("Score", file_text(score))];
        if let Some(confidence) = model.default_confidence {
            values.push(("Confidence", number_text(confidence)));
        }
        facts(sections, "Default", values);
    }
    let rows = model
        .rules
        .iter()
        .take(1_000)
        .enumerate()
        .map(|(index, rule)| {
            let score = file_text(&rule.score);
            vec![
                index.saturating_add(1).to_string(),
                predicate_text(&rule.predicate),
                if rule.disabled {
                    format!("{score} (disabled)")
                } else {
                    score
                },
                number_text(rule.confidence),
                optional_number(rule.record_count),
            ]
        })
        .collect();
    table(
        sections,
        "Rules",
        columns(&["#", "If", "Then", "Confidence", "Records"]),
        rows,
    );
    truncated(
        sections,
        "Rules",
        model.rules.len().min(1_000),
        model.rules.len(),
    );
}

fn coefficient_heading(category: Option<&str>) -> String {
    category.map_or_else(
        || "Coefficients".into(),
        |c| format!("Coefficients: {}", file_text(c)),
    )
}

fn regression_sections(model: &RegressionModel, sections: &mut Vec<SummarySection>) {
    for coefficients in &model.tables {
        let heading = coefficient_heading(coefficients.target_category.as_deref());
        let mut rows = vec![vec![
            "Intercept".into(),
            number_text(coefficients.intercept),
            String::new(),
        ]];
        for (name, exponent, coefficient) in coefficients
            .numeric
            .iter()
            .take(MAX_COEFFICIENTS.saturating_sub(1))
        {
            let name = file_text(name);
            rows.push(vec![
                if *exponent == 1 {
                    name
                } else {
                    format!("{name}^{exponent}")
                },
                number_text(*coefficient),
                String::new(),
            ]);
        }
        for (name, category, coefficient) in coefficients
            .categorical
            .iter()
            .take(MAX_COEFFICIENTS.saturating_sub(rows.len()))
        {
            rows.push(vec![
                file_text(name),
                number_text(*coefficient),
                file_text(category),
            ]);
        }
        let total = 1usize
            .saturating_add(coefficients.numeric.len())
            .saturating_add(coefficients.categorical.len());
        let shown = rows.len();
        table(
            sections,
            &heading,
            columns(&["Predictor", "Coefficient", "Category"]),
            rows,
        );
        truncated(sections, &heading, shown, total);
    }
}

fn general_sections(model: &GeneralRegressionModel, sections: &mut Vec<SummarySection>) {
    // Borrow keys: do not clone unbounded parameter strings or matrix entries.
    let mut mappings = std::collections::BTreeMap::<&str, Vec<&crate::PPCell>>::new();
    for cell in &model.pp_matrix {
        let cells = mappings.entry(cell.parameter.as_str()).or_default();
        if cells.len() <= MAX_TERMS {
            cells.push(cell);
        }
    }
    let mut categories = Vec::new();
    let mut groups = std::collections::BTreeMap::<Option<&str>, (usize, Vec<&crate::PCell>)>::new();
    for cell in &model.param_matrix {
        let category = cell.target_category.as_deref();
        let (total, rows) = groups.entry(category).or_insert_with(|| {
            categories.push(category);
            (0usize, Vec::new())
        });
        *total = total.saturating_add(1);
        // Keep intercepts first even if ParamMatrix lists them after other coefficients.
        let intercept = !mappings.contains_key(cell.parameter.as_str());
        if intercept {
            if rows.len() == MAX_COEFFICIENTS {
                rows.pop();
            }
            rows.insert(0, cell);
        } else if rows.len() < MAX_COEFFICIENTS {
            rows.push(cell);
        }
    }
    for category in categories {
        let Some((total, cells)) = groups.remove(&category) else {
            continue;
        };
        let rows: Vec<Vec<String>> = cells
            .into_iter()
            .map(|cell| {
                let mapping = mappings.get(cell.parameter.as_str());
                let (predictor, categories) = match mapping {
                    None => ("Intercept".into(), String::new()),
                    Some(mapping) => {
                        let predictor = join_text(
                            mapping.iter().map(|p| {
                                let name = file_text(&p.predictor);
                                if model.covariates.contains(&p.predictor) && p.value != "1" {
                                    format!("{name}^{}", file_text(&p.value))
                                } else {
                                    name
                                }
                            }),
                            " * ",
                        );
                        let categories = join_text(
                            mapping
                                .iter()
                                .filter(|p| model.factors.contains(&p.predictor))
                                .map(|p| file_text(&p.value)),
                            ", ",
                        );
                        (predictor, categories)
                    }
                };
                vec![predictor, number_text(cell.beta), categories]
            })
            .collect();
        let shown = rows.len();
        let heading = coefficient_heading(category);
        table(
            sections,
            &heading,
            columns(&["Parameter / predictor", "Coefficient", "Category"]),
            rows,
        );
        truncated(sections, &heading, shown, total);
    }
}

fn cluster_sections(model: &ClusteringModel, sections: &mut Vec<SummarySection>) {
    let has_size = model.clusters.iter().take(200).any(|c| c.size.is_some());
    let mut header = columns(&["Cluster"]);
    if has_size {
        header.push("Size".into());
    }
    header.extend(model.fields.iter().take(50).map(|f| file_text(&f.field)));
    let rows = model
        .clusters
        .iter()
        .take(200)
        .enumerate()
        .map(|(index, cluster)| {
            let name = cluster
                .name
                .as_deref()
                .or(cluster.id.as_deref())
                .map_or_else(|| index.saturating_add(1).to_string(), file_text);
            let mut row = vec![name];
            if has_size {
                row.push(optional_number(cluster.size));
            }
            let mut center = cluster.center.iter();
            for field in model.fields.iter().take(50) {
                // Arrays contain center fields only; non-center fields must not shift coordinates.
                row.push(if field.is_center_field {
                    center.next().map_or_else(String::new, |v| number_text(*v))
                } else {
                    String::new()
                });
            }
            row
        })
        .collect();
    table(sections, "Clusters", header, rows);
    truncated(
        sections,
        "Clusters",
        model.clusters.len().min(200),
        model.clusters.len(),
    );
    truncated(
        sections,
        "Clustering fields",
        model.fields.len().min(50),
        model.fields.len(),
    );
}

fn neural_sections(model: &NeuralNetworkModel, sections: &mut Vec<SummarySection>) {
    let hidden = join_text(
        model
            .layers
            .iter()
            .take(model.layers.len().saturating_sub(1))
            .map(|layer| layer.neurons.len().to_string()),
        ", ",
    );
    let activation = match model.activation {
        Activation::Threshold => "threshold",
        Activation::Logistic => "logistic",
        Activation::Tanh => "tanh",
        Activation::Identity => "identity",
        Activation::Exponential => "exponential",
        Activation::Reciprocal => "reciprocal",
        Activation::Square => "square",
        Activation::Gauss => "Gauss",
        Activation::Sine => "sine",
        Activation::Cosine => "cosine",
        Activation::Elliott => "Elliott",
        Activation::Arctan => "arctan",
        Activation::RadialBasis => "radialBasis",
    };
    facts(
        sections,
        "Neural network",
        vec![
            ("Inputs", model.inputs.len().to_string()),
            ("Hidden layers", hidden),
            ("Outputs", model.outputs.len().to_string()),
            ("Activation function", activation.into()),
        ],
    );
}

fn mining_sections(model: &MiningModel, depth: usize, sections: &mut Vec<SummarySection>) {
    facts(
        sections,
        "Ensemble",
        vec![
            ("Method", file_text(&model.method)),
            ("Segments", model.segments.len().to_string()),
        ],
    );
    if depth >= MAX_ENSEMBLE_DEPTH {
        truncated(sections, "Segments", 0, model.segments.len());
        return;
    }
    for (index, segment) in model.segments.iter().take(20).enumerate() {
        let start = sections.len();
        model_sections(&segment.model, depth.saturating_add(1), sections);
        let prefix = if segment.predicate == Predicate::True {
            format!("Segment {}: ", index.saturating_add(1))
        } else {
            format!(
                "Segment {}: {} — ",
                index.saturating_add(1),
                predicate_text(&segment.predicate)
            )
        };
        for section in sections.iter_mut().skip(start) {
            section.heading = format!("{prefix}{}", section.heading);
        }
    }
    truncated(
        sections,
        "Segments",
        model.segments.len().min(20),
        model.segments.len(),
    );
}

fn association_sections(model: &AssociationModel, sections: &mut Vec<SummarySection>) {
    facts(
        sections,
        "Association rules",
        vec![
            ("Rules", model.rules.len().to_string()),
            ("Items", model.items.len().to_string()),
            ("Transactions", model.number_of_transactions.to_string()),
            ("Minimum support", number_text(model.minimum_support)),
            ("Minimum confidence", number_text(model.minimum_confidence)),
        ],
    );
    let items: std::collections::BTreeMap<&str, &str> = model
        .items
        .iter()
        .map(|item| (item.id.as_str(), item.value.as_str()))
        .collect();
    let sets: std::collections::BTreeMap<&str, &crate::AssociationItemset> = model
        .itemsets
        .iter()
        .map(|set| (set.id.as_str(), set))
        .collect();
    let itemset = |id: &str| {
        sets.get(id).map_or_else(
            || file_text(id),
            |set| {
                join_text(
                    set.items.iter().map(|id| {
                        file_text(items.get(id.as_str()).copied().unwrap_or(id.as_str()))
                    }),
                    ", ",
                )
            },
        )
    };
    let rows = model
        .rules
        .iter()
        .take(500)
        .map(|rule| {
            vec![
                itemset(&rule.antecedent),
                itemset(&rule.consequent),
                number_text(rule.support),
                number_text(rule.confidence),
                number_text(rule.lift),
            ]
        })
        .collect();
    table(
        sections,
        "Rules",
        columns(&["Antecedent", "Consequent", "Support", "Confidence", "Lift"]),
        rows,
    );
    truncated(
        sections,
        "Rules",
        model.rules.len().min(500),
        model.rules.len(),
    );
}

fn sequence_text(sets: &[SequenceItemset]) -> String {
    join_text(
        sets.iter().map(|set| {
            format!(
                "{{{}}}",
                join_text(
                    set.iter().map(|(field, value)| format!(
                        "{} = {}",
                        file_text(field),
                        literal(value)
                    )),
                    ", "
                )
            )
        }),
        " → ",
    )
}

fn sequence_sections(model: &SequenceModel, sections: &mut Vec<SummarySection>) {
    let items: std::collections::BTreeSet<(&str, &str)> = model
        .rules
        .iter()
        .flat_map(|rule| rule.antecedent.iter().chain(&rule.consequent))
        .flatten()
        .map(|(field, value)| (field.as_str(), value.as_str()))
        .collect();
    facts(
        sections,
        "Sequence rules",
        vec![
            ("Rules", model.rules.len().to_string()),
            ("Items", items.len().to_string()),
            ("Transactions", model.number_of_transactions.to_string()),
        ],
    );
    let rows = model
        .rules
        .iter()
        .take(500)
        .map(|rule| {
            vec![
                sequence_text(&rule.antecedent),
                sequence_text(&rule.consequent),
                number_text(rule.support),
                number_text(rule.confidence),
            ]
        })
        .collect();
    table(
        sections,
        "Rules",
        columns(&["Antecedent", "Consequent", "Support", "Confidence"]),
        rows,
    );
    truncated(
        sections,
        "Rules",
        model.rules.len().min(500),
        model.rules.len(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEAF: &str = r#"<TreeModel functionName="classification"><MiningSchema/>
        <Node score="yes"><True/></Node></TreeModel>"#;

    fn parse(model: &str) -> Pmml {
        Pmml::parse(&format!(
            "<PMML version=\"4.3\"><DataDictionary/>{model}</PMML>"
        ))
        .unwrap()
    }

    fn body<'a>(summary: &'a ModelSummary, heading: &str) -> &'a SectionBody {
        &summary
            .sections
            .iter()
            .find(|s| s.heading == heading)
            .unwrap()
            .body
    }

    fn fact(summary: &ModelSummary, heading: &str, label: &str) -> String {
        let SectionBody::Facts(values) = body(summary, heading) else {
            panic!("expected facts")
        };
        values
            .iter()
            .find(|(key, _)| key == label)
            .unwrap()
            .1
            .clone()
    }

    fn rows<'a>(summary: &'a ModelSummary, heading: &str) -> &'a Vec<Vec<String>> {
        let SectionBody::Table { rows, .. } = body(summary, heading) else {
            panic!("expected table")
        };
        rows
    }

    fn lines(summary: &ModelSummary) -> &Vec<String> {
        let SectionBody::Lines(lines) = &summary
            .sections
            .iter()
            .find(|s| matches!(s.body, SectionBody::Lines(_)))
            .unwrap()
            .body
        else {
            panic!("expected lines")
        };
        lines
    }

    fn has_truncation(summary: &ModelSummary, heading: &str, shown: usize, total: usize) -> bool {
        summary
            .sections
            .iter()
            .any(|s| s.heading == heading && s.body == SectionBody::Truncated { shown, total })
    }

    #[test]
    fn summary_tree_fields_facts_and_depth_first_lines() {
        let pmml = parse(
            r#"<TreeModel functionName="classification" algorithmName="C5.0" modelName="A tree">
          <MiningSchema><MiningField name="Age"/><MiningField name="Sex"/>
          <MiningField name="ignored" usageType="supplementary"/>
          <MiningField name="response" usageType="predicted"/><MiningField name="other" usageType="target"/></MiningSchema>
          <Node score="no" recordCount="200"><True/>
            <Node score="yes" recordCount="120"><SimplePredicate field="Age" operator="lessOrEqual" value="30"/>
              <ScoreDistribution value="yes" recordCount="102" confidence="0.85"/>
              <ScoreDistribution value="no" recordCount="18"/>
              <Node score="yes"><SimplePredicate field="Sex" operator="equal" value="F"/>
                <ScoreDistribution value="yes" recordCount="9" probability="0.9"/>
              </Node>
            </Node>
            <Node score="no"><SimplePredicate field="Age" operator="greaterThan" value="30"/>
              <ScoreDistribution value="yes" recordCount="2"/>
              <ScoreDistribution value="no" recordCount="8"/>
            </Node>
          </Node></TreeModel>"#,
        );
        let summary = summarize(&pmml);
        assert_eq!(summary.kind, "Decision tree (C5.0)");
        assert_eq!(summary.name.as_deref(), Some("A tree"));
        assert_eq!(summary.function.as_deref(), Some("classification"));
        assert_eq!(summary.inputs, ["Age", "Sex"]);
        assert_eq!(summary.targets, ["response", "other"]);
        assert_eq!(fact(&summary, "Tree", "Nodes"), "4");
        assert_eq!(fact(&summary, "Tree", "Leaves"), "2");
        assert_eq!(fact(&summary, "Tree", "Depth"), "2");
        assert_eq!(
            lines(&summary),
            &[
                "true → no (n = 200)",
                "  Age <= 30 → yes (n = 120, 85.0%)",
                "    Sex = \"F\" → yes (90.0%)",
                "  Age > 30 → no (80.0%)"
            ]
        );
        assert!(!summary.sections.iter().any(|s| s.heading == "Fields"));
        assert_eq!(summary, summarize(&pmml));
    }

    #[test]
    fn summary_rule_set_default_disabled_and_unknown_records() {
        let summary = summarize(&parse(
            r#"<RuleSetModel functionName="classification"><MiningSchema/>
          <RuleSet defaultScore="no" defaultConfidence="0.6"><RuleSelectionMethod criterion="firstHit"/>
          <SimpleRule score="yes" confidence="0.9" recordCount="12"><True/></SimpleRule>
          <SimpleRule score="no" confidence="0.7"><Extension name="isDisabled" value="true"/><False/></SimpleRule>
          </RuleSet></RuleSetModel>"#,
        ));
        assert_eq!(summary.kind, "Rule set");
        assert_eq!(fact(&summary, "Default", "Score"), "no");
        assert_eq!(fact(&summary, "Default", "Confidence"), "0.6");
        assert_eq!(
            rows(&summary, "Rules"),
            &[
                vec!["1", "true", "yes", "0.9", "12"],
                vec!["2", "false", "no (disabled)", "0.7", ""]
            ]
        );
    }

    #[test]
    fn summary_regression_intercept_categories_and_exponents() {
        let summary = summarize(&parse(
            r#"<RegressionModel functionName="classification" normalizationMethod="logit">
          <MiningSchema/><RegressionTable intercept="-1.5" targetCategory="yes">
          <NumericPredictor name="Age" exponent="2" coefficient="0.25"/>
          <CategoricalPredictor name="Sex" value="F" coefficient="2"/></RegressionTable>
          <RegressionTable intercept="0" targetCategory="no"/></RegressionModel>"#,
        ));
        assert_eq!(summary.kind, "Logistic regression");
        assert_eq!(
            rows(&summary, "Coefficients: yes"),
            &[
                vec!["Intercept", "-1.5", ""],
                vec!["Age^2", "0.25", ""],
                vec!["Sex", "2", "F"]
            ]
        );
        assert_eq!(
            rows(&summary, "Coefficients: no"),
            &[vec!["Intercept", "0", ""]]
        );
        let linear = summarize(&parse(
            r#"<RegressionModel functionName="regression"><MiningSchema/>
            <RegressionTable intercept="3"/></RegressionModel>"#,
        ));
        assert_eq!(linear.kind, "Linear regression");
        assert_eq!(rows(&linear, "Coefficients")[0][0], "Intercept");
    }

    #[test]
    fn summary_general_regression_resolves_parameters_and_intercept_first() {
        let model = r#"<GeneralRegressionModel functionName="regression" modelType="generalizedLinear">
          <MiningSchema/><ParameterList><Parameter name="p0"/><Parameter name="p1"/><Parameter name="p2"/></ParameterList>
          <FactorList><Predictor name="group"/></FactorList><CovariateList><Predictor name="x"/></CovariateList>
          <PPMatrix><PPCell parameterName="p1" predictorName="group" value="A"/>
          <PPCell parameterName="p2" predictorName="x" value="2"/></PPMatrix>
          <ParamMatrix><PCell parameterName="p1" beta="2" targetCategory="yes"/>
          <PCell parameterName="p2" beta="0.25" targetCategory="yes"/>
          <PCell parameterName="p0" beta="1.5" targetCategory="yes"/>
          <PCell parameterName="p0" beta="0" targetCategory="no"/></ParamMatrix></GeneralRegressionModel>"#;
        let summary = summarize(&parse(model));
        assert_eq!(summary.kind, "Generalized linear model");
        assert_eq!(
            rows(&summary, "Coefficients: yes"),
            &[
                vec!["Intercept", "1.5", ""],
                vec!["group", "2", "A"],
                vec!["x^2", "0.25", ""]
            ]
        );
        assert_eq!(
            rows(&summary, "Coefficients: no"),
            &[vec!["Intercept", "0", ""]]
        );
        assert_eq!(
            summarize(&parse(
                &model.replace("generalizedLinear", "multinomialLogistic")
            ))
            .kind,
            "Logistic regression"
        );
        assert_eq!(
            summarize(&parse(&model.replace("generalizedLinear", "generalLinear"))).kind,
            "Linear regression"
        );
    }

    const CLUSTERS: &str = r#"<ClusteringModel functionName="clustering" modelClass="centerBased">
        <MiningSchema/><ComparisonMeasure kind="distance"><euclidean/></ComparisonMeasure>
        <ClusteringField field="raw" isCenterField="false"/>
        <ClusteringField field="x" isCenterField="true"/><ClusteringField field="y" isCenterField="true"/>
        <Cluster name="First" id="a" size="20"><Array type="real">1.25 2</Array></Cluster>
        <Cluster id="b"><Array type="real">3 4</Array></Cluster></ClusteringModel>"#;

    #[test]
    fn summary_cluster_centres_and_optional_sizes() {
        let summary = summarize(&parse(CLUSTERS));
        assert_eq!(summary.kind, "Cluster model");
        let SectionBody::Table { header, .. } = body(&summary, "Clusters") else {
            panic!("expected table")
        };
        assert_eq!(header, &["Cluster", "Size", "raw", "x", "y"]);
        assert_eq!(
            rows(&summary, "Clusters"),
            &[
                vec!["First", "20", "", "1.25", "2"],
                vec!["b", "", "", "3", "4"]
            ]
        );
        let summary = summarize(&parse(&CLUSTERS.replace(" size=\"20\"", "")));
        assert_eq!(rows(&summary, "Clusters")[0], ["First", "", "1.25", "2"]);
    }

    #[test]
    fn summary_neural_hidden_layers_exclude_output_layer() {
        let summary = summarize(&parse(
            r#"<NeuralNetwork functionName="classification" activationFunction="tanh">
          <MiningSchema/><NeuralInputs><NeuralInput id="in"><DerivedField optype="continuous" dataType="double">
          <FieldRef field="x"/></DerivedField></NeuralInput></NeuralInputs>
          <NeuralLayer><Neuron id="h1"/><Neuron id="h2"/></NeuralLayer>
          <NeuralLayer><Neuron id="h3"/></NeuralLayer><NeuralLayer><Neuron id="out"/></NeuralLayer>
          <NeuralOutputs><NeuralOutput outputNeuron="out"><DerivedField optype="categorical" dataType="string">
          <NormDiscrete field="y" value="yes"/></DerivedField></NeuralOutput></NeuralOutputs></NeuralNetwork>"#,
        ));
        assert_eq!(summary.kind, "Neural network");
        assert_eq!(fact(&summary, "Neural network", "Inputs"), "1");
        assert_eq!(fact(&summary, "Neural network", "Hidden layers"), "2, 1");
        assert_eq!(fact(&summary, "Neural network", "Outputs"), "1");
        assert_eq!(
            fact(&summary, "Neural network", "Activation function"),
            "tanh"
        );
    }

    fn ensemble(inner: &str, count: usize) -> String {
        let mut xml = String::from(
            r#"<MiningModel functionName="regression"><MiningSchema/><Segmentation multipleModelMethod="average">"#,
        );
        for index in 0..count {
            let predicate = if index == 0 {
                "<True/>"
            } else {
                r#"<SimplePredicate field="x" operator="lessThan" value="3"/>"#
            };
            xml.push_str(&format!("<Segment>{predicate}{inner}</Segment>"));
        }
        xml.push_str("</Segmentation></MiningModel>");
        xml
    }

    #[test]
    fn summary_mining_two_segments_and_predicate_prefix() {
        let summary = summarize(&parse(&ensemble(LEAF, 2)));
        assert_eq!(summary.kind, "Ensemble");
        assert_eq!(fact(&summary, "Ensemble", "Method"), "average");
        assert_eq!(fact(&summary, "Ensemble", "Segments"), "2");
        assert_eq!(fact(&summary, "Segment 1: Tree", "Nodes"), "1");
        assert_eq!(fact(&summary, "Segment 2: x < 3 — Tree", "Leaves"), "1");
        assert_eq!(lines(&summary), &["true → yes"]);
    }

    #[test]
    fn summary_ensemble_segment_and_recursion_limits() {
        let summary = summarize(&parse(&ensemble(LEAF, 21)));
        assert!(has_truncation(&summary, "Segments", 20, 21));
        assert!(
            !summary
                .sections
                .iter()
                .any(|s| s.heading.starts_with("Segment 21:"))
        );
        let mut model = LEAF.to_string();
        for _ in 0..6 {
            model = ensemble(&model, 1);
        }
        let summary = summarize(&parse(&model));
        assert_eq!(summary.sections.len(), 6);
        assert!(
            summary
                .sections
                .iter()
                .any(|s| s.body == SectionBody::Truncated { shown: 0, total: 1 })
        );
        assert!(
            !summary
                .sections
                .iter()
                .any(|s| matches!(s.body, SectionBody::Lines(_)))
        );
    }

    const ASSOCIATION: &str = r#"<AssociationModel functionName="associationRules" minimumSupport="0.1"
        minimumConfidence="0.5" numberOfTransactions="10"><MiningSchema/>
        <Item id="a" value="bread"/><Item id="b" value="milk"/>
        <Itemset id="s1" support="0.4"><ItemRef itemRef="a"/></Itemset>
        <Itemset id="s2" support="0.5"><ItemRef itemRef="b"/></Itemset>
        <AssociationRule id="r1" antecedent="s1" consequent="s2" support="0.3" confidence="0.75" lift="1.5"/>
        </AssociationModel>"#;

    #[test]
    fn summary_association_resolves_itemsets() {
        let summary = summarize(&parse(ASSOCIATION));
        assert_eq!(summary.kind, "Association rules");
        assert_eq!(summary.function.as_deref(), Some("association rules"));
        assert_eq!(fact(&summary, "Association rules", "Rules"), "1");
        assert_eq!(fact(&summary, "Association rules", "Items"), "2");
        assert_eq!(
            fact(&summary, "Association rules", "Minimum support"),
            "0.1"
        );
        assert_eq!(
            fact(&summary, "Association rules", "Minimum confidence"),
            "0.5"
        );
        assert_eq!(
            rows(&summary, "Rules"),
            &[vec!["bread", "milk", "0.3", "0.75", "1.5"]]
        );
    }

    #[test]
    fn summary_sequence_shows_ordered_itemsets() {
        let summary = summarize(&parse(
            r#"<SequenceModel functionName="sequences" numberOfTransactions="10">
          <MiningSchema><MiningField name="basket" usageType="group"/><MiningField name="item"/></MiningSchema>
          <Item id="a" value="bread"/><Item id="b" value="milk"/>
          <Itemset id="s1"><ItemRef itemRef="a"/></Itemset><Itemset id="s2"><ItemRef itemRef="b"/></Itemset>
          <Sequence id="q1"><SetReference setId="s1"/><SetReference setId="s2"/></Sequence>
          <Sequence id="q2"><SetReference setId="s2"/></Sequence>
          <SequenceRule id="r" confidence="0.75" support="0.4" occurrence="3">
          <AntecedentSequence><SequenceReference seqId="q1"/></AntecedentSequence>
          <ConsequentSequence><SequenceReference seqId="q2"/></ConsequentSequence></SequenceRule></SequenceModel>"#,
        ));
        assert_eq!(summary.kind, "Sequence rules");
        assert_eq!(fact(&summary, "Sequence rules", "Items"), "2");
        assert_eq!(
            rows(&summary, "Rules"),
            &[vec![
                "{item = \"bread\"} → {item = \"milk\"}",
                "{item = \"milk\"}",
                "0.4",
                "0.75"
            ]]
        );
    }

    #[test]
    fn summary_time_series_uses_header_only() {
        let summary = summarize(&parse(
            r#"<TimeSeriesModel functionName="timeSeries" modelName="Sales" algorithmName="ARIMA">
          <MiningSchema><MiningField name="sales" usageType="target"/></MiningSchema></TimeSeriesModel>"#,
        ));
        assert_eq!(summary.kind, "Time series model (ARIMA)");
        assert_eq!(fact(&summary, "Time series", "Name"), "Sales");
        assert_eq!(fact(&summary, "Time series", "Function"), "time series");
        assert_eq!(fact(&summary, "Time series", "Algorithm"), "ARIMA");
        assert_eq!(summary.targets, ["sales"]);
    }

    #[test]
    fn summary_unsupported_element_and_empty_metadata() {
        let summary = summarize(&parse("<SupportVectorMachineModel/>"));
        assert_eq!(
            summary.kind,
            "Unsupported model (SupportVectorMachineModel)"
        );
        assert_eq!(summary.name, None);
        assert_eq!(summary.function, None);
        assert!(summary.inputs.is_empty() && summary.targets.is_empty());
        assert_eq!(
            fact(&summary, "Model", "Element"),
            "SupportVectorMachineModel"
        );
    }

    fn parsed_predicate(xml: &str) -> Predicate {
        let pmml = parse(&format!(
            r#"<TreeModel functionName="classification"><MiningSchema/><Node>{xml}</Node></TreeModel>"#
        ));
        let Model::Tree(tree) = pmml.model else {
            panic!("expected tree")
        };
        tree.root.predicate
    }

    #[test]
    fn summary_predicate_text_all_simple_operators_and_missing() {
        assert_eq!(predicate_text(&parsed_predicate("<True/>")), "true");
        assert_eq!(predicate_text(&parsed_predicate("<False/>")), "false");
        for (operator, expected) in [
            ("equal", "="),
            ("notEqual", "!="),
            ("lessThan", "<"),
            ("lessOrEqual", "<="),
            ("greaterThan", ">"),
            ("greaterOrEqual", ">="),
        ] {
            let predicate = parsed_predicate(&format!(
                r#"<SimplePredicate field="Age" operator="{operator}" value="30"/>"#
            ));
            assert_eq!(predicate_text(&predicate), format!("Age {expected} 30"));
        }
        assert_eq!(
            predicate_text(&parsed_predicate(
                r#"<SimplePredicate field="Sex" operator="equal" value="F"/>"#
            )),
            "Sex = \"F\""
        );
        for (operator, expected) in [
            ("isMissing", "is missing"),
            ("isNotMissing", "is not missing"),
        ] {
            assert_eq!(
                predicate_text(&parsed_predicate(&format!(
                    r#"<SimplePredicate field="Age" operator="{operator}"/>"#
                ))),
                format!("Age {expected}")
            );
        }
    }

    #[test]
    fn summary_predicate_sets_and_compounds() {
        for (operator, expected) in [("isIn", "in"), ("isNotIn", "not in")] {
            let predicate = parsed_predicate(&format!(
                r#"<SimpleSetPredicate field="Region" booleanOperator="{operator}"><Array type="string">"N" "S"</Array></SimpleSetPredicate>"#
            ));
            assert_eq!(
                predicate_text(&predicate),
                format!("Region {expected} {{\"N\", \"S\"}}")
            );
        }
        for operator in ["and", "or", "xor", "surrogate"] {
            let predicate = parsed_predicate(&format!(
                r#"<CompoundPredicate booleanOperator="{operator}"><True/><CompoundPredicate booleanOperator="and"><False/><True/></CompoundPredicate></CompoundPredicate>"#
            ));
            assert_eq!(
                predicate_text(&predicate),
                format!("true {operator} (false and true)")
            );
        }
    }

    #[test]
    fn summary_predicate_depth_and_width_are_bounded() {
        let mut predicate = parsed_predicate("<True/>");
        for _ in 0..100 {
            predicate = Predicate::Compound {
                operator: BooleanOperator::And,
                predicates: vec![predicate],
            };
        }
        let text = predicate_text(&predicate);
        assert!(text.contains('…'));
        assert!(!text.contains("true"));
        let predicate = Predicate::Compound {
            operator: BooleanOperator::Or,
            predicates: vec![parsed_predicate("<True/>"); 100_000],
        };
        let text = predicate_text(&predicate);
        assert!(text.ends_with('…'));
        assert!(text.len() <= MAX_EXPRESSION.saturating_add(3));
    }

    #[test]
    fn summary_number_text_edge_cases_and_rounding() {
        for (number, expected) in [
            (0.0, "0"),
            (-0.0, "0"),
            (42.0, "42"),
            (0.1 + 0.2, "0.3"),
            (1e9, "1e9"),
            (1e-5, "1e-5"),
            (-1e9, "-1e9"),
            (-1.23e-7, "-1.23e-7"),
            (f64::NAN, "NaN"),
            (f64::INFINITY, "∞"),
            (f64::NEG_INFINITY, "-∞"),
            (1.23456789, "1.234568"),
            (123.1234567, "123.123457"),
            (1e-4, "0.0001"),
            (999_999_999.0, "999999999"),
        ] {
            assert_eq!(number_text(number), expected);
        }
        assert_eq!(number_text(f64::MAX), "1.797693e308");
        assert_eq!(number_text(f64::from_bits(1)), "4.940656e-324");
    }

    #[test]
    fn summary_file_strings_preserve_unicode_and_limit_to_200_characters() {
        let long = "界🙂".repeat(150);
        let pmml = parse(&format!(
            r#"<TreeModel functionName="classification" algorithmName="{long}" modelName="{long}">
            <MiningSchema><MiningField name="{long}"/></MiningSchema>
            <Node score="{long}"><SimplePredicate field="{long}" operator="equal" value="{long}"/></Node></TreeModel>"#
        ));
        let summary = summarize(&pmml);
        let expected = format!("{}…", long.chars().take(199).collect::<String>());
        assert_eq!(summary.name, Some(expected.clone()));
        assert_eq!(summary.inputs, std::slice::from_ref(&expected));
        assert_eq!(summary.kind, format!("Decision tree ({expected})"));
        assert_eq!(
            lines(&summary)[0],
            format!("{expected} = \"{expected}\" → {expected}")
        );
        assert_eq!(file_text(&"é".repeat(200)), "é".repeat(200));
    }

    #[test]
    fn summary_wide_tree_truncates_after_2000_lines_and_counts_all_nodes() {
        let mut pmml = parse(LEAF);
        let Model::Tree(tree) = &mut pmml.model else {
            panic!("expected tree")
        };
        tree.root.children = vec![tree.root.clone(); 2_100];
        let summary = summarize(&pmml);
        assert_eq!(lines(&summary).len(), 2_000);
        assert_eq!(fact(&summary, "Tree", "Nodes"), "2101");
        assert_eq!(fact(&summary, "Tree", "Leaves"), "2100");
        assert!(has_truncation(&summary, "Tree", 2_000, 2_101));
    }

    #[test]
    fn summary_3000_level_tree_does_not_overflow_stack() {
        let mut pmml = parse(LEAF);
        let Model::Tree(tree) = &mut pmml.model else {
            panic!("expected tree")
        };
        let leaf = tree.root.clone();
        let mut root = leaf.clone();
        for _ in 0..3_000 {
            let mut parent = leaf.clone();
            parent.children.push(root);
            root = parent;
        }
        tree.root = root;
        let summary = summarize(&pmml);
        // Node's existing derived drop is recursive: dismantle test input iteratively before assertions.
        let Model::Tree(tree) = pmml.model else {
            panic!("expected tree")
        };
        let mut stack = vec![tree.root];
        while let Some(mut node) = stack.pop() {
            stack.append(&mut node.children);
        }
        assert_eq!(fact(&summary, "Tree", "Nodes"), "3001");
        assert_eq!(fact(&summary, "Tree", "Depth"), "3000");
        assert_eq!(fact(&summary, "Tree", "Leaves"), "1");
        assert_eq!(lines(&summary).len(), 2_000);
        assert!(has_truncation(&summary, "Tree", 2_000, 3_001));
    }

    #[test]
    fn summary_table_limits_report_exact_totals() {
        let mut pmml = parse(
            r#"<RuleSetModel functionName="classification"><MiningSchema/><RuleSet>
            <RuleSelectionMethod criterion="firstHit"/><SimpleRule score="yes"><True/></SimpleRule></RuleSet></RuleSetModel>"#,
        );
        let Model::RuleSet(model) = &mut pmml.model else {
            panic!("expected rules")
        };
        model.rules = vec![model.rules[0].clone(); 1_001];
        let summary = summarize(&pmml);
        assert_eq!(rows(&summary, "Rules").len(), 1_000);
        assert!(has_truncation(&summary, "Rules", 1_000, 1_001));

        let mut pmml = parse(
            r#"<RegressionModel functionName="regression"><MiningSchema/>
            <RegressionTable intercept="1"><NumericPredictor name="x" coefficient="2"/></RegressionTable></RegressionModel>"#,
        );
        let Model::Regression(model) = &mut pmml.model else {
            panic!("expected regression")
        };
        model.tables[0].numeric = vec![("x".into(), 1, 2.0); 1_001];
        let summary = summarize(&pmml);
        assert_eq!(rows(&summary, "Coefficients").len(), 1_000);
        assert!(has_truncation(&summary, "Coefficients", 1_000, 1_002));

        let mut pmml = parse(CLUSTERS);
        let Model::Clustering(model) = &mut pmml.model else {
            panic!("expected clustering")
        };
        model.clusters = vec![model.clusters[0].clone(); 201];
        model.fields = vec![model.fields[1].clone(); 51];
        let summary = summarize(&pmml);
        assert_eq!(rows(&summary, "Clusters").len(), 200);
        assert_eq!(rows(&summary, "Clusters")[0].len(), 52);
        assert!(has_truncation(&summary, "Clusters", 200, 201));
        assert!(has_truncation(&summary, "Clustering fields", 50, 51));

        let mut pmml = parse(ASSOCIATION);
        let Model::Association(model) = &mut pmml.model else {
            panic!("expected association")
        };
        model.rules = vec![model.rules[0].clone(); 501];
        let summary = summarize(&pmml);
        assert_eq!(rows(&summary, "Rules").len(), 500);
        assert!(has_truncation(&summary, "Rules", 500, 501));
    }

    #[test]
    fn summary_empty_supported_models_still_have_a_section() {
        for xml in [
            r#"<RuleSetModel functionName="classification"><MiningSchema/><RuleSet><RuleSelectionMethod criterion="firstHit"/></RuleSet></RuleSetModel>"#,
            r#"<GeneralRegressionModel functionName="regression" modelType="generalLinear"><MiningSchema/><ParameterList/><ParamMatrix/></GeneralRegressionModel>"#,
            r#"<ClusteringModel functionName="clustering" modelClass="centerBased"><MiningSchema/><ComparisonMeasure kind="distance"><euclidean/></ComparisonMeasure></ClusteringModel>"#,
        ] {
            assert!(!summarize(&parse(xml)).sections.is_empty());
        }
    }
}
