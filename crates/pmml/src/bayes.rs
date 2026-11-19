//! Modeler's Bayes Net nugget: the `BayesNetModel` Modeler writes inside
//! `<Extension extender="spss.com" name="SPSS Models">`, and its scoring.
//!
//! The model is a discrete Bayesian network that holds one conditional probability table per
//! node: the parents' names, the node's categories and one row per parent configuration that
//! occurs in the training data. Scoring is exact inference: the tables are reduced by the
//! evidence the record gives, every other variable is summed out, and what remains over the
//! target is normalised. A value that is missing, or a category the tables do not list, gives no
//! evidence for that variable.
//!
//! A parent configuration with no row has probability 0, as in the maximum-likelihood estimate of
//! a configuration that never occurred. When every target category then has probability 0 the
//! record has no prediction (`Value::Missing`): Modeler's behaviour for that case is not known.

use crate::parse::{parse_model_header, parse_value_str, req_attr};
use crate::{ModelHeader, PmmlError, Prediction, Usage, Value};
use std::collections::HashSet;

/// Largest table (cells) inference may build.
const MAX_FACTOR_CELLS: usize = 1 << 22;
/// Largest number of variables and of rows in one table.
const MAX_NODES: usize = 4_096;
const MAX_ROWS: usize = 4_000_000;

/// A Bayes Net model: its header and one table per node.
#[derive(Debug, Clone, PartialEq)]
pub struct BayesNetModel {
    pub header: ModelHeader,
    /// The predicted field (the mining field with usage target).
    pub target: String,
    pub nodes: Vec<BayesNode>,
}

/// The conditional probability table of one node.
#[derive(Debug, Clone, PartialEq)]
pub struct BayesNode {
    pub name: String,
    /// The parents, in the order the rows list their values.
    pub parents: Vec<String>,
    /// The categories of the node, in the order of the row probabilities.
    pub categories: Vec<String>,
    pub rows: Vec<BayesRow>,
}

/// One parent configuration of a table.
#[derive(Debug, Clone, PartialEq)]
pub struct BayesRow {
    pub parent_values: Vec<String>,
    /// The number of training records with this configuration, when the table records it.
    pub total: Option<f64>,
    /// `P(category | parents)` for each of the node's categories.
    pub probabilities: Vec<f64>,
}

pub(crate) fn parse(node: roxmltree::Node<'_, '_>) -> Result<BayesNetModel, PmmlError> {
    let header = parse_model_header(node)?;
    let target = header
        .mining_schema
        .iter()
        .find(|field| field.usage == Usage::Target)
        .map(|field| field.name.clone())
        .ok_or_else(|| PmmlError::Parse("BayesNetModel has no predicted field".into()))?;
    let tables = node
        .children()
        .find(|child| child.has_tag_name("ConditionalTables"))
        .ok_or_else(|| PmmlError::Parse("BayesNetModel is missing ConditionalTables".into()))?;
    let mut nodes = Vec::new();
    for table in tables
        .children()
        .filter(|child| child.has_tag_name("ConditionalTable"))
    {
        if nodes.len() >= MAX_NODES {
            return Err(PmmlError::Parse("BayesNetModel has too many nodes".into()));
        }
        nodes.push(parse_table(table)?);
    }
    if !nodes.iter().any(|n| n.name == target) {
        return Err(PmmlError::Parse(format!(
            "BayesNetModel has no table for the target {target:?}"
        )));
    }
    let model = BayesNetModel {
        header,
        target,
        nodes,
    };
    model.validate()?;
    Ok(model)
}

fn parse_table(table: roxmltree::Node<'_, '_>) -> Result<BayesNode, PmmlError> {
    let name = req_attr(table, "fieldName")?.to_string();
    let parent_count: usize = req_attr(table, "parentsNumber")?
        .trim()
        .parse()
        .map_err(|_| PmmlError::Parse(format!("invalid parentsNumber in the table of {name:?}")))?;
    let names = table
        .children()
        .find(|child| child.has_tag_name("names"))
        .and_then(|n| n.text())
        .ok_or_else(|| PmmlError::Parse(format!("the table of {name:?} has no names")))?;
    let header: Vec<&str> = names.split(';').map(str::trim).collect();
    if parent_count >= header.len() {
        return Err(PmmlError::Parse(format!(
            "the table of {name:?} lists {parent_count} parents but only {} names",
            header.len()
        )));
    }
    let parents: Vec<String> = header[..parent_count]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let categories: Vec<String> = header[parent_count..]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let mut rows = Vec::new();
    for row in table.children().filter(|child| child.has_tag_name("row")) {
        if rows.len() >= MAX_ROWS {
            return Err(PmmlError::Parse(format!(
                "the table of {name:?} has too many rows"
            )));
        }
        let cells: Vec<&str> = row.text().unwrap_or("").split(';').map(str::trim).collect();
        if cells.len() != header.len() {
            return Err(PmmlError::Parse(format!(
                "a row of the table of {name:?} has {} values, expected {}",
                cells.len(),
                header.len()
            )));
        }
        let probabilities = cells[parent_count..]
            .iter()
            .map(|cell| {
                cell.parse::<f64>()
                    .ok()
                    .filter(|p| p.is_finite() && *p >= 0.0)
                    .ok_or_else(|| {
                        PmmlError::Parse(format!(
                            "invalid probability {cell:?} in the table of {name:?}"
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        rows.push(BayesRow {
            parent_values: cells[..parent_count]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            total: row.attribute("totalN").and_then(|t| t.trim().parse().ok()),
            probabilities,
        });
    }
    Ok(BayesNode {
        name,
        parents,
        categories,
        rows,
    })
}

impl BayesNetModel {
    /// Checks what inference relies on: unique nodes, parents that are nodes, and a graph
    /// without cycles.
    fn validate(&self) -> Result<(), PmmlError> {
        let mut seen = HashSet::new();
        for node in &self.nodes {
            if !seen.insert(node.name.as_str()) {
                return Err(PmmlError::Parse(format!(
                    "BayesNetModel has two tables for {:?}",
                    node.name
                )));
            }
        }
        for node in &self.nodes {
            for parent in &node.parents {
                if !seen.contains(parent.as_str()) {
                    return Err(PmmlError::Parse(format!(
                        "{:?} has the parent {parent:?} that has no table",
                        node.name
                    )));
                }
            }
        }
        // Kahn's algorithm over the parent relation.
        let mut resolved: HashSet<&str> = HashSet::new();
        while resolved.len() < self.nodes.len() {
            let before = resolved.len();
            for node in &self.nodes {
                if !resolved.contains(node.name.as_str())
                    && node.parents.iter().all(|p| resolved.contains(p.as_str()))
                {
                    resolved.insert(node.name.as_str());
                }
            }
            if resolved.len() == before {
                return Err(PmmlError::Parse("BayesNetModel has a cycle".into()));
            }
        }
        Ok(())
    }

    /// The names of the variables the network reads: every node but the target.
    pub(crate) fn evidence_fields(&self) -> impl Iterator<Item = &str> {
        self.nodes
            .iter()
            .map(|n| n.name.as_str())
            .filter(|name| *name != self.target)
    }

    /// The categories of the target, in table order.
    pub fn target_categories(&self) -> &[String] {
        self.nodes
            .iter()
            .find(|n| n.name == self.target)
            .map_or(&[], |n| n.categories.as_slice())
    }

    /// `P(target | evidence)` for evidence given per node (the target's own entry is ignored):
    /// the category index of each observed variable, `None` for no evidence. `None` as a result
    /// when every target category has probability 0.
    pub fn posterior(&self, evidence: &[Option<usize>]) -> Result<Option<Vec<f64>>, PmmlError> {
        if evidence.len() != self.nodes.len() {
            return Err(PmmlError::Score(
                "evidence does not match the network's nodes".into(),
            ));
        }
        let target = self
            .nodes
            .iter()
            .position(|n| n.name == self.target)
            .ok_or_else(|| PmmlError::Score("the target has no table".into()))?;
        let index_of = |name: &str| self.nodes.iter().position(|n| n.name == name);

        let mut factors = Vec::with_capacity(self.nodes.len());
        for (i, node) in self.nodes.iter().enumerate() {
            let mut scope: Vec<usize> = node
                .parents
                .iter()
                .map(|p| index_of(p).ok_or_else(|| PmmlError::Score("unknown parent".into())))
                .collect::<Result<_, _>>()?;
            scope.push(i);
            let cards: Vec<usize> = scope
                .iter()
                .map(|&v| self.nodes.get(v).map_or(0, |n| n.categories.len()))
                .collect();
            let size = cards
                .iter()
                .try_fold(1usize, |acc, &c| acc.checked_mul(c))
                .filter(|&s| s <= MAX_FACTOR_CELLS)
                .ok_or_else(|| {
                    PmmlError::Unsupported(format!("the table of {:?} is too large", node.name))
                })?;
            let mut values = vec![0.0; size];
            for row in &node.rows {
                let mut offset = 0usize;
                let mut valid = true;
                for (value, &parent) in row.parent_values.iter().zip(&scope) {
                    let parent_node = self.nodes.get(parent);
                    let at = parent_node
                        .and_then(|p| p.categories.iter().position(|c| same_label(c, value)));
                    match at {
                        Some(at) => {
                            offset = offset * parent_node.map_or(1, |p| p.categories.len()) + at;
                        }
                        None => valid = false,
                    }
                }
                if !valid {
                    continue;
                }
                offset *= node.categories.len();
                for (k, p) in row.probabilities.iter().enumerate() {
                    if let Some(slot) = values.get_mut(offset + k) {
                        *slot = *p;
                    }
                }
            }
            factors.push(Factor {
                vars: scope,
                cards,
                values,
            });
        }

        // Reduce by the evidence.
        let factors: Vec<Factor> = factors
            .into_iter()
            .map(|f| {
                f.reduce(|v| {
                    if v == target {
                        None
                    } else {
                        evidence.get(v).copied().flatten()
                    }
                })
            })
            .collect();

        let result = eliminate(factors, target)?;
        let k = self.nodes.get(target).map_or(0, |n| n.categories.len());
        let mut p = vec![1.0; k];
        for factor in &result {
            // every remaining factor is over the target alone (or a constant)
            for (cat, slot) in p.iter_mut().enumerate() {
                *slot *= if factor.vars.is_empty() {
                    factor.values.first().copied().unwrap_or(1.0)
                } else {
                    factor.values.get(cat).copied().unwrap_or(0.0)
                };
            }
        }
        let total: f64 = p.iter().sum();
        if !(total.is_finite() && total > 0.0) {
            return Ok(None);
        }
        Ok(Some(p.into_iter().map(|x| x / total).collect()))
    }
}

pub(crate) fn score(
    pmml: &crate::Pmml,
    model: &BayesNetModel,
    env: &crate::score::Env<'_>,
) -> Result<Prediction, PmmlError> {
    let _ = pmml;
    let evidence: Vec<Option<usize>> = model
        .nodes
        .iter()
        .map(|node| {
            if node.name == model.target {
                return None;
            }
            match env.get(&node.name) {
                None | Some(Value::Missing) => None,
                Some(value) => node
                    .categories
                    .iter()
                    .position(|c| crate::expr::value_matches_str(value, c)),
            }
        })
        .collect();
    let Some(p) = model.posterior(&evidence)? else {
        return Ok(Prediction {
            value: Value::Missing,
            confidence: None,
            probabilities: Vec::new(),
            entity_id: None,
            affinity: None,
        });
    };
    let categories = model.target_categories();
    let mut best = 0usize;
    for (i, x) in p.iter().enumerate() {
        if *x > p.get(best).copied().unwrap_or(0.0) {
            best = i;
        }
    }
    Ok(Prediction {
        value: categories
            .get(best)
            .map_or(Value::Missing, |c| parse_value_str(c)),
        confidence: p.get(best).copied(),
        probabilities: categories.iter().cloned().zip(p.iter().copied()).collect(),
        entity_id: None,
        affinity: None,
    })
}

/// Labels of categories and of row values are compared as text, numerically when both are numbers.
fn same_label(a: &str, b: &str) -> bool {
    a.trim() == b.trim()
        || matches!((a.trim().parse::<f64>(), b.trim().parse::<f64>()), (Ok(x), Ok(y)) if x == y)
}

/// A table over a set of variables, row-major in `vars` order.
#[derive(Debug, Clone)]
struct Factor {
    vars: Vec<usize>,
    cards: Vec<usize>,
    values: Vec<f64>,
}

impl Factor {
    /// Fixes the variables `observed` gives a category for.
    fn reduce(self, observed: impl Fn(usize) -> Option<usize>) -> Factor {
        let fixed: Vec<Option<usize>> = self.vars.iter().map(|&v| observed(v)).collect();
        if fixed.iter().all(Option::is_none) {
            return self;
        }
        let keep: Vec<usize> = (0..self.vars.len())
            .filter(|&i| fixed.get(i).copied().flatten().is_none())
            .collect();
        let vars: Vec<usize> = keep
            .iter()
            .filter_map(|&i| self.vars.get(i))
            .copied()
            .collect();
        let cards: Vec<usize> = keep
            .iter()
            .filter_map(|&i| self.cards.get(i))
            .copied()
            .collect();
        let size: usize = cards.iter().product();
        let mut values = vec![0.0; size];
        let n = self.cards.len();
        // strides of the original layout
        let mut strides = vec![1usize; n];
        for i in (0..n.saturating_sub(1)).rev() {
            strides[i] = strides[i + 1] * self.cards.get(i + 1).copied().unwrap_or(1);
        }
        let base: usize = fixed
            .iter()
            .enumerate()
            .filter_map(|(i, f)| f.map(|c| c * strides[i]))
            .sum();
        for (out, slot) in values.iter_mut().enumerate() {
            let mut rest = out;
            let mut offset = base;
            for (pos, &i) in keep.iter().enumerate().rev() {
                let card = cards.get(pos).copied().unwrap_or(1).max(1);
                offset += (rest % card) * strides[i];
                rest /= card;
            }
            *slot = self.values.get(offset).copied().unwrap_or(0.0);
        }
        Factor {
            vars,
            cards,
            values,
        }
    }

    fn card_of(&self, var: usize) -> Option<usize> {
        self.vars
            .iter()
            .position(|&v| v == var)
            .and_then(|i| self.cards.get(i).copied())
    }
}

/// The product of `factors`, with `sum_out` summed out when given.
fn product(factors: &[Factor], sum_out: Option<usize>) -> Result<Factor, PmmlError> {
    let mut scope: Vec<usize> = factors
        .iter()
        .flat_map(|f| f.vars.iter().copied())
        .collect();
    scope.sort_unstable();
    scope.dedup();
    let cards: Vec<usize> = scope
        .iter()
        .map(|&v| factors.iter().find_map(|f| f.card_of(v)).unwrap_or(1))
        .collect();
    let size = cards
        .iter()
        .try_fold(1usize, |acc, &c| acc.checked_mul(c))
        .filter(|&s| s <= MAX_FACTOR_CELLS)
        .ok_or_else(|| PmmlError::Unsupported("the network is too connected to score".into()))?;
    // for each factor, the stride of each scope variable in its layout (0 when absent)
    let strides: Vec<Vec<usize>> = factors
        .iter()
        .map(|f| {
            let n = f.vars.len();
            let mut own = vec![1usize; n];
            for i in (0..n.saturating_sub(1)).rev() {
                own[i] = own[i + 1] * f.cards.get(i + 1).copied().unwrap_or(1);
            }
            scope
                .iter()
                .map(|v| f.vars.iter().position(|x| x == v).map_or(0, |i| own[i]))
                .collect()
        })
        .collect();
    let out_vars: Vec<usize> = scope
        .iter()
        .copied()
        .filter(|&v| Some(v) != sum_out)
        .collect();
    let out_cards: Vec<usize> = scope
        .iter()
        .zip(&cards)
        .filter(|(v, _)| Some(**v) != sum_out)
        .map(|(_, &c)| c)
        .collect();
    let out_size: usize = out_cards.iter().product();
    let mut values = vec![0.0; out_size.max(1)];
    let mut digits = vec![0usize; scope.len()];
    let mut offsets = vec![0usize; factors.len()];
    for _ in 0..size {
        let mut p = 1.0;
        for (f, offset) in factors.iter().zip(&offsets) {
            p *= f.values.get(*offset).copied().unwrap_or(0.0);
        }
        // the output cell of this assignment
        let mut out_index = 0usize;
        for (i, &v) in scope.iter().enumerate() {
            if Some(v) != sum_out {
                out_index = out_index * cards[i] + digits[i];
            }
        }
        if let Some(slot) = values.get_mut(out_index) {
            *slot += p;
        }
        // advance the mixed-radix counter, last variable fastest
        for i in (0..scope.len()).rev() {
            digits[i] += 1;
            for (j, offset) in offsets.iter_mut().enumerate() {
                *offset += strides[j][i];
            }
            if digits[i] < cards[i] {
                break;
            }
            for (j, offset) in offsets.iter_mut().enumerate() {
                *offset -= strides[j][i] * cards[i];
            }
            digits[i] = 0;
        }
    }
    Ok(Factor {
        vars: out_vars,
        cards: out_cards,
        values,
    })
}

/// Sums out every variable but `keep`, smallest resulting table first.
fn eliminate(mut factors: Vec<Factor>, keep: usize) -> Result<Vec<Factor>, PmmlError> {
    loop {
        let mut candidates: Vec<usize> = factors
            .iter()
            .flat_map(|f| f.vars.iter().copied())
            .filter(|&v| v != keep)
            .collect();
        candidates.sort_unstable();
        candidates.dedup();
        let Some(&var) = candidates.iter().min_by_key(|&&v| {
            let mut scope: Vec<(usize, usize)> = Vec::new();
            for f in factors.iter().filter(|f| f.vars.contains(&v)) {
                for (&u, &c) in f.vars.iter().zip(&f.cards) {
                    if !scope.iter().any(|(x, _)| *x == u) {
                        scope.push((u, c));
                    }
                }
            }
            scope.iter().fold(1usize, |a, (_, c)| a.saturating_mul(*c))
        }) else {
            return Ok(factors);
        };
        let (involved, rest): (Vec<Factor>, Vec<Factor>) =
            factors.into_iter().partition(|f| f.vars.contains(&var));
        let summed = product(&involved, Some(var))?;
        factors = rest;
        factors.push(summed);
    }
}
