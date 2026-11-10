//! Decision List, with an empirically selected two-stage beam search.
//! Sources: IBM SPSS Modeler 19.0 Algorithms Guide, PDF pp.109–115 (the same chapter
//! as 18.0 printed pp.105–111; sea-level split pp.108–110 in that edition),
//! PMML 4.3 RuleSet, and differential oracle cases listed below. No IBM implementation used.
//!
//! - `train_next/dl_default`: equal-frequency edges average sorted values at positions
//!   floor(k*N/binCount) and the next (1-based); distinct edges below the maximum.
//!   Equal-width edges are min+k*(max-min)/binCount (`dl_numeric`). Bins are (lo,hi].
//! - Categorical values sort ascending by string (`models_next2/dl_default`).
//! - Minimum size is max(minGroupSize, 5% of training records), strictly exceeded:
//!   `train_next/dl_default` rejects 60-record segments; `dl_min_group` raises the floor.
//! - Sea-level split starts with response-rate peaks, takes the best segment stably,
//!   skips all categories and rates <= the covered rate. Acceptance requires size >
//!   max(minimum, segmentGrowth * each contained result size), lower Clopper–Pearson
//!   bound > the LOCAL covered population's upper bound, and both neighbours lower.
//!   It removes containing results of size <= growth*size, then extends toward the
//!   higher adjacent rate, larger adjacent count, or right on ties (`dl_default`).
//! - Rule search stably selects the best response-rate extensions at each depth and
//!   returns distinct terminal rules plus the last depth's selected rules. Lists grow
//!   on their own remaining records. Canonical list identity sorts conditions within
//!   each rule, preserving rule order and the first generated representation.
//! - The list beam stably ranks the arithmetic mean of its rules' first-hit response
//!   rates. Completed lists compete with extensions at subsequent depths. Final
//!   selection uses union response rate, fewer covered records, fewer rules, descending
//!   first-hit rate vector, then field-name predicate order. A disabled True remainder
//!   follows. Widths, depths and all other saved settings still apply as below.
//! - The mean beam score is an EMPIRICAL FIT, departing from the guide's union estimate.
//!   Phase 3 `estimates-01822` and phase 4 fit 20/21 frozen cases semantically; the Rust
//!   builder is checked for full RuleSet IR equality in `exec/tests/decision_list.rs`.
//!   `dl_discriminate/dl05_carry` remains a counterexample: first rule b=v0 & c=v0
//!   (118/140), whereas the oracle chooses a=v1 & c=v0 (116/160). Both cover the same
//!   final union, but their ordered first-hit partitions differ. See
//!   `docs/decision-list-validation.md` and local `target/dl6/REPORT.md` for evidence.
//! - Non-expert mode ignores confidence, rule width/depth, list width, binning and growth:
//!   defaults 95%, 5, 5, 5, equalFreq/10, 2 (`dl_confidence80` equals `dl_default`).
//!   List depth and minimum group size always apply. Expert settings: `dl_search`, `dl_numeric`.
//! - Open: dl05's partition, the rationale/generalization of the empirical beam and final
//!   tie order, and completed lists tying on every numeric final key with different
//!   first-hit supports (`models_next2/dl_default`). The lexical key does not settle that
//!   remaining ambiguity; such ties, or full-key ties with different counts, are refused.
//!   PMML emission intersects repeated field restrictions, retains the first condition's
//!   field and orders subsequent fields by name. This empirical serialization convention
//!   supplies exact predicates/schema without changing search identities or beam order.
//!   No oracle for searchDirection down, reuseFields=false, weights/frequencies, missing
//!   inputs; these are rejected. This is a strict oracle improvement, not a claim that
//!   the proprietary search has been recovered.

use crate::stats::f_quantile;
use crate::{AlgoError, Dataset};
use dms_pmml::Pmml;
use dms_types::{Measurement, Role, Storage, ValueSpec};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct DecisionListSettings {
    pub min_group_size: f64,
    pub segment_growth: f64,
    pub confidence: f64,
    pub rule_width: usize,
    pub rule_depth: usize,
    pub list_width: usize,
    pub list_depth: usize,
    pub bin_count: usize,
    pub equal_width: bool,
    /// None uses the flag's true value; other targets require an explicit response value.
    pub response: Option<String>,
}

impl Default for DecisionListSettings {
    fn default() -> Self {
        Self {
            min_group_size: 50.0,
            segment_growth: 2.0,
            confidence: 0.95,
            rule_width: 5,
            rule_depth: 5,
            list_width: 5,
            list_depth: 5,
            bin_count: 10,
            equal_width: false,
            response: None,
        }
    }
}

struct Attribute {
    name: String,
    data_type: &'static str,
    optype: &'static str,
    codes: Vec<usize>,
    labels: Vec<String>,
    edges: Option<Vec<f64>>,
}

impl Attribute {
    fn len(&self) -> usize {
        self.edges
            .as_ref()
            .map_or(self.labels.len(), |e| e.len() + 1)
    }
}

type Rule = Vec<(usize, Vec<usize>)>;
type RuleList = Vec<Rule>;
struct RankedList {
    rules: RuleList,
    /// Per-rule first-hit (records, responses), excluding the disabled remainder.
    signature: Vec<(usize, usize)>,
    remaining: Vec<usize>,
    mean: f64,
    union_rate: f64,
    covered: usize,
    completed: bool,
    /// Parent beam index; remaining rows are materialized only after width pruning.
    parent: usize,
}

impl RankedList {
    fn identity(&self) -> RuleList {
        self.rules
            .iter()
            .map(|rule| {
                let mut key = rule.clone();
                key.sort();
                key
            })
            .collect()
    }

    fn rank_cmp(&self, other: &Self) -> Ordering {
        other
            .union_rate
            .total_cmp(&self.union_rate)
            .then(self.covered.cmp(&other.covered))
            .then(self.rules.len().cmp(&other.rules.len()))
            .then_with(|| {
                self.signature
                    .iter()
                    .map(|&(n, y)| response_rate(y, n))
                    .zip(&other.signature)
                    .map(|(a, &(n, y))| response_rate(y, n).total_cmp(&a))
                    .find(|order| *order != Ordering::Equal)
                    .unwrap_or(Ordering::Equal)
            })
    }

    fn final_cmp(&self, other: &Self, attrs: &[Attribute]) -> Ordering {
        self.rank_cmp(other).then_with(|| {
            self.rules
                .iter()
                .map(|rule| predicate_key(rule, attrs))
                .cmp(other.rules.iter().map(|rule| predicate_key(rule, attrs)))
        })
    }
}

fn response_rate(y: usize, n: usize) -> f64 {
    if n == 0 { 0.0 } else { y as f64 / n as f64 }
}

struct Search<'a> {
    y: Vec<bool>,
    attrs: Vec<Attribute>,
    settings: &'a DecisionListSettings,
    minimum: f64,
    intervals: HashMap<(usize, usize), (f64, f64)>,
    work: usize,
}

impl Search<'_> {
    fn interval(&mut self, x: usize, n: usize) -> (f64, f64) {
        *self
            .intervals
            .entry((x, n))
            .or_insert_with(|| ci(x, n, self.settings.confidence))
    }

    fn covered(&mut self, records: &[usize], rule: &Rule) -> Result<Vec<usize>, AlgoError> {
        self.work = self
            .work
            .saturating_add(records.len().saturating_mul(rule.len().max(1)));
        if self.work > 200_000_000 {
            return Err(AlgoError::Unsupported(
                "GAP: Decision List search work limit (200000000)".into(),
            ));
        }
        Ok(records
            .iter()
            .copied()
            .filter(|&i| {
                rule.iter().all(|(f, cats)| {
                    self.attrs
                        .get(*f)
                        .and_then(|a| a.codes.get(i))
                        .is_some_and(|c| cats.contains(c))
                })
            })
            .collect())
    }

    fn positives(&self, records: &[usize]) -> usize {
        records
            .iter()
            .filter(|&&i| self.y.get(i) == Some(&true))
            .count()
    }

    fn rate(&self, records: &[usize]) -> f64 {
        if records.is_empty() {
            0.0
        } else {
            self.positives(records) as f64 / records.len() as f64
        }
    }

    fn split(&mut self, records: &[usize], field: usize, pop_hi: f64) -> Vec<Vec<usize>> {
        let Some(attr) = self.attrs.get(field) else {
            return Vec::new();
        };
        let mut n = vec![0usize; attr.len()];
        let mut x = vec![0usize; attr.len()];
        for &i in records {
            if let Some(&code) = attr.codes.get(i) {
                if let Some(count) = n.get_mut(code) {
                    *count += 1;
                }
                if self.y.get(i) == Some(&true)
                    && let Some(count) = x.get_mut(code)
                {
                    *count += 1;
                }
            }
        }
        let idx: Vec<usize> = (0..n.len())
            .filter(|&c| n.get(c).copied().unwrap_or(0) > 0)
            .collect();
        let m = idx.len();
        if m == 0 {
            return Vec::new();
        }
        let mut counts = vec![0usize];
        let mut successes = vec![0usize];
        for &c in &idx {
            counts.push(counts.last().copied().unwrap_or(0) + n.get(c).copied().unwrap_or(0));
            successes.push(successes.last().copied().unwrap_or(0) + x.get(c).copied().unwrap_or(0));
        }
        let stats = |l: usize, r: usize| {
            (
                counts
                    .get(r + 1)
                    .copied()
                    .unwrap_or(0)
                    .saturating_sub(counts.get(l).copied().unwrap_or(0)),
                successes
                    .get(r + 1)
                    .copied()
                    .unwrap_or(0)
                    .saturating_sub(successes.get(l).copied().unwrap_or(0)),
            )
        };
        let prob = |l, r| {
            let (n, x) = stats(l, r);
            if n == 0 { 0.0 } else { x as f64 / n as f64 }
        };
        let mut segments = Vec::new();
        let mut i = 0;
        while i < m {
            if prob(i, i) > if i == 0 { 0.0 } else { prob(i - 1, i - 1) } {
                let mut j = i;
                while j + 1 < m && prob(j + 1, j + 1) == prob(i, i) {
                    j += 1;
                }
                if prob(j, j) > if j + 1 == m { 0.0 } else { prob(j + 1, j + 1) } {
                    segments.push((i, j));
                }
                i = j + 1;
            } else {
                i += 1;
            }
        }
        let all_rate = prob(0, m - 1);
        let mut result: Vec<(usize, usize)> = Vec::new();
        let mut seen = HashSet::new();
        while !segments.is_empty() {
            segments.sort_by(|&(l1, r1), &(l2, r2)| prob(l2, r2).total_cmp(&prob(l1, r1)));
            let (mut l, mut r) = segments.remove(0);
            if !seen.insert((l, r)) || r - l + 1 == m || prob(l, r) <= all_rate {
                continue;
            }
            let (nn, xx) = stats(l, r);
            let need = result
                .iter()
                .filter(|&&(a, b)| l <= a && b <= r)
                .map(|&(a, b)| self.settings.segment_growth * stats(a, b).0 as f64)
                .fold(self.minimum, f64::max);
            let lower = self.interval(xx, nn).0;
            if nn as f64 > need
                && lower > pop_hi
                && (l == 0 || prob(l - 1, r) < prob(l, r))
                && (r + 1 == m || prob(l, r + 1) < prob(l, r))
            {
                result.retain(|&(a, b)| {
                    !(a <= l
                        && r <= b
                        && stats(a, b).0 as f64 <= self.settings.segment_growth * nn as f64)
                });
                result.push((l, r));
            }
            if l == 0 {
                r += 1;
            } else if r + 1 == m {
                l -= 1;
            } else {
                let left = prob(l - 1, l - 1);
                let right = prob(r + 1, r + 1);
                if left > right || (left == right && stats(l - 1, l - 1).0 > stats(r + 1, r + 1).0)
                {
                    l -= 1;
                } else {
                    r += 1;
                }
            }
            segments.push((l, r));
        }
        result
            .into_iter()
            .map(|(l, r)| idx.iter().skip(l).take(r - l + 1).copied().collect())
            .collect()
    }

    fn rules(&mut self, records: &[usize]) -> Result<Vec<Rule>, AlgoError> {
        let mut beam = vec![Vec::new()];
        let mut result = Vec::new();
        for depth in 0..self.settings.rule_depth {
            crate::check_cancelled()?;
            let mut candidates = Vec::new();
            for rule in beam {
                let xr = self.covered(records, &rule)?;
                let hi = self.interval(self.positives(&xr), xr.len()).1;
                let mut splits = Vec::new();
                for f in 0..self.attrs.len() {
                    for seg in self.split(&xr, f, hi) {
                        splits.push((f, seg));
                    }
                }
                if splits.is_empty() && !rule.is_empty() {
                    result.push(rule.clone());
                }
                for split in splits {
                    let mut next = rule.clone();
                    next.push(split);
                    let cov = self.covered(records, &next)?;
                    candidates.push((next, self.rate(&cov)));
                    candidate_limit(candidates.len())?;
                }
            }
            if candidates.is_empty() {
                break;
            }
            candidates.sort_by(|a, b| b.1.total_cmp(&a.1));
            beam = candidates
                .into_iter()
                .take(self.settings.rule_width)
                .map(|(r, _)| r)
                .collect();
            if depth + 1 == self.settings.rule_depth {
                result.extend(beam.iter().cloned());
            }
        }
        // A terminal may also have been selected at the depth limit. Preserve its first
        // representation, without merging distinct conjunctions that cover the same rows.
        let mut seen = HashSet::new();
        result.retain(|rule| seen.insert(rule.clone()));
        Ok(result)
    }

    fn lists(&mut self) -> Result<RuleList, AlgoError> {
        let all: Vec<usize> = (0..self.y.len()).collect();
        let mut kept = vec![RankedList {
            rules: Vec::new(),
            signature: Vec::new(),
            remaining: all,
            mean: 0.0,
            union_rate: 0.0,
            covered: 0,
            completed: false,
            parent: 0,
        }];
        let mut last_level = Vec::new();
        for _ in 0..self.settings.list_depth {
            crate::check_cancelled()?;
            let mut candidates = Vec::new();
            let mut seen = HashSet::new();
            for (parent, list) in kept.iter().enumerate() {
                let rules = if list.completed {
                    Vec::new()
                } else {
                    self.rules(&list.remaining)?
                };
                if rules.is_empty() && !list.rules.is_empty() {
                    if seen.insert(list.identity()) {
                        candidates.push(RankedList {
                            rules: list.rules.clone(),
                            signature: list.signature.clone(),
                            remaining: Vec::new(),
                            mean: list.mean,
                            union_rate: list.union_rate,
                            covered: list.covered,
                            completed: true,
                            parent,
                        });
                        candidate_limit(candidates.len())?;
                    }
                    continue;
                }
                for rule in rules {
                    let cov = self.covered(&list.remaining, &rule)?;
                    if cov.is_empty() {
                        continue;
                    }
                    let mut next_rules = list.rules.clone();
                    next_rules.push(rule);
                    let mut signature = list.signature.clone();
                    signature.push((cov.len(), self.positives(&cov)));
                    let covered = list.covered.saturating_add(cov.len());
                    let responses = signature.iter().map(|(_, y)| y).sum();
                    let mean = signature
                        .iter()
                        .map(|&(n, y)| response_rate(y, n))
                        .sum::<f64>()
                        / signature.len() as f64;
                    let next = RankedList {
                        rules: next_rules,
                        signature,
                        remaining: Vec::new(),
                        mean,
                        union_rate: response_rate(responses, covered),
                        covered,
                        completed: false,
                        parent,
                    };
                    if seen.insert(next.identity()) {
                        candidates.push(next);
                    }
                    candidate_limit(candidates.len())?;
                }
            }
            if candidates.is_empty() {
                break;
            }
            // Only beam pruning uses the empirical mean. Keep generation order on ties.
            candidates.sort_by(|a, b| b.mean.total_cmp(&a.mean));
            // The final selection includes the entire final candidate level, before pruning.
            // Move it out rather than cloning candidates and their remaining populations.
            last_level = candidates.split_off(self.settings.list_width.min(candidates.len()));
            // Candidate count can be much larger than beam width. Retaining every
            // candidate's row population would allocate candidates * rows before the work
            // limit is reached. Only selected beam members need populations at all.
            for candidate in &mut candidates {
                let parent = kept
                    .get(candidate.parent)
                    .ok_or_else(|| AlgoError::Data("Decision List parent beam index".into()))?;
                candidate.remaining = if candidate.completed {
                    parent.remaining.clone()
                } else {
                    let rule = candidate.rules.last().ok_or_else(|| {
                        AlgoError::Data("Decision List extension has no rule".into())
                    })?;
                    let cov: HashSet<usize> =
                        self.covered(&parent.remaining, rule)?.into_iter().collect();
                    parent
                        .remaining
                        .iter()
                        .copied()
                        .filter(|i| !cov.contains(i))
                        .collect()
                };
            }
            kept = candidates;
        }
        last_level.extend(kept);
        last_level.sort_by(|a, b| a.final_cmp(b, &self.attrs));
        // Names settle capped-list ties (`dl10_tie_sizes`), but completed lists with equal
        // numeric ranks and different first-hit supports remain unverified
        // (`models_next2/dl_default`). Equal union rates alone no longer trigger a GAP.
        if let Some(top) = last_level.first()
            && last_level.iter().skip(1).any(|list| {
                top.signature != list.signature
                    && (top.final_cmp(list, &self.attrs) == Ordering::Equal
                        || (top.completed
                            && list.completed
                            && top.rank_cmp(list) == Ordering::Equal))
            })
        {
            return Err(AlgoError::Unsupported(
                "GAP: Decision List unresolved final list tie with different first-hit counts"
                    .into(),
            ));
        }
        Ok(last_level
            .into_iter()
            .next()
            .map(|l| l.rules)
            .unwrap_or_default())
    }
}

fn candidate_limit(n: usize) -> Result<(), AlgoError> {
    if n > 10_000 {
        Err(AlgoError::Unsupported(
            "GAP: Decision List candidate limit (10000)".into(),
        ))
    } else {
        Ok(())
    }
}

fn ci(x: usize, n: usize, confidence: f64) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let p = 1.0 - (1.0 - confidence) * 0.5;
    let (x, n) = (x as f64, n as f64);
    let lo = if x == 0.0 {
        0.0
    } else {
        x / (x + (n - x + 1.0) * f_quantile(p, 2.0 * (n - x + 1.0), 2.0 * x))
    };
    let hi = if x == n {
        1.0
    } else {
        let f = f_quantile(p, 2.0 * (x + 1.0), 2.0 * (n - x));
        (x + 1.0) * f / (n - x + (x + 1.0) * f)
    };
    (lo, hi)
}

fn prepare<'a>(
    dataset: &Dataset<'_>,
    s: &'a DecisionListSettings,
) -> Result<(Search<'a>, String), AlgoError> {
    if !s.min_group_size.is_finite()
        || s.min_group_size < 0.0
        || !s.segment_growth.is_finite()
        || s.segment_growth < 1.0
        || !s.confidence.is_finite()
        || !(0.0..1.0).contains(&s.confidence)
        || s.confidence == 0.0
        || 1.0 - (1.0 - s.confidence) * 0.5 == 1.0
        || [s.rule_width, s.list_width]
            .iter()
            .any(|n| !(1..=64).contains(n))
        || [s.rule_depth, s.list_depth]
            .iter()
            .any(|n| !(1..=32).contains(n))
        || !(2..=256).contains(&s.bin_count)
    {
        return Err(AlgoError::Settings("Decision List: invalid size, growth, confidence, beam (1..64), depth (1..32) or bins (2..256)".into()));
    }
    let table = dataset.table;
    if table.row_count() == 0
        || table.row_count() > 1_000_000
        || dataset.inputs.len() > 128
        || table.row_count().saturating_mul(dataset.inputs.len()) > 8_000_000
    {
        return Err(AlgoError::Data(
            "Decision List requires 1..1000000 rows, <=128 inputs, <=8000000 input cells".into(),
        ));
    }
    if dataset.frequency.is_some()
        || table
            .model()
            .fields()
            .iter()
            .any(|f| matches!(f.role, Role::Frequency | Role::Split))
    {
        return Err(AlgoError::Unsupported(
            "GAP: Decision List frequency, weight or split fields".into(),
        ));
    }
    let target = table
        .model()
        .fields()
        .get(dataset.target)
        .ok_or_else(|| AlgoError::Data("Decision List target index".into()))?;
    let response = s
        .response
        .clone()
        .or_else(|| match &target.values {
            ValueSpec::Flag { true_value, .. } => Some(true_value.to_string()),
            _ => None,
        })
        .ok_or_else(|| {
            AlgoError::Settings(
                "Decision List requires a flag target or explicit response value".into(),
            )
        })?;
    if target.measurement == Measurement::Continuous {
        return Err(AlgoError::Settings(
            "Decision List requires a categorical target".into(),
        ));
    }
    let mut text_bytes = target.name.len().saturating_add(response.len());
    let mut y = Vec::new();
    for row in table.rows() {
        crate::check_cancelled()?;
        let v = row
            .get(dataset.target)
            .filter(|v| !target.is_missing(v))
            .ok_or_else(|| AlgoError::Unsupported("GAP: Decision List missing target".into()))?;
        y.push(v.to_string() == response);
    }
    if !y.contains(&true) {
        return Err(AlgoError::Data(
            "Decision List response value absent".into(),
        ));
    }
    let mut attrs = Vec::new();
    for &position in dataset.inputs {
        crate::check_cancelled()?;
        if position == dataset.target {
            return Err(AlgoError::Settings(
                "Decision List target is also an input".into(),
            ));
        }
        let field = table
            .model()
            .fields()
            .get(position)
            .ok_or_else(|| AlgoError::Data("Decision List input index".into()))?;
        let input_data_type = data_type(field.storage)?;
        text_bytes = text_bytes.saturating_add(field.name.len());
        let values = table
            .rows()
            .iter()
            .map(|r| {
                r.get(position)
                    .filter(|v| !field.is_missing(v))
                    .ok_or_else(|| {
                        AlgoError::Unsupported(format!(
                            "GAP: Decision List missing input {}",
                            field.name
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        for value in &values {
            text_bytes = text_bytes.saturating_add(value.as_str().map_or(32, str::len));
        }
        if text_bytes > 32 * 1024 * 1024 {
            return Err(AlgoError::Unsupported(
                "GAP: Decision List input text limit (32 MiB)".into(),
            ));
        }
        let (codes, labels, edges) = if field.measurement == Measurement::Continuous {
            let vals = values
                .iter()
                .map(|v| {
                    v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| {
                        AlgoError::Data(format!(
                            "Decision List nonfinite/nonnumeric input {}",
                            field.name
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let mut sorted = vals.clone();
            sorted.sort_by(f64::total_cmp);
            let min = sorted.first().copied().unwrap_or(0.0);
            let max = sorted.last().copied().unwrap_or(0.0);
            let mut edges = Vec::new();
            for k in 1..s.bin_count {
                let edge = if s.equal_width {
                    min + (max - min) / s.bin_count as f64 * k as f64
                } else {
                    let pos = k.saturating_mul(sorted.len()) / s.bin_count;
                    let (Some(a), Some(b)) = (
                        pos.checked_sub(1).and_then(|i| sorted.get(i)),
                        sorted.get(pos),
                    ) else {
                        continue;
                    };
                    a * 0.5 + b * 0.5
                };
                if !edge.is_finite() {
                    return Err(AlgoError::Data("Decision List bin edge overflow".into()));
                }
                if edge >= min && edge < max {
                    edges.push(edge);
                }
            }
            edges.sort_by(f64::total_cmp);
            edges.dedup();
            let codes = vals
                .iter()
                .map(|v| edges.partition_point(|e| v > e))
                .collect();
            (codes, Vec::new(), Some(edges))
        } else {
            let texts: Vec<String> = values.iter().map(|v| v.to_string()).collect();
            let mut labels = texts.clone();
            labels.sort();
            labels.dedup();
            if labels.len() > 256 {
                return Err(AlgoError::Unsupported(
                    "GAP: Decision List category limit (256)".into(),
                ));
            }
            let codes = texts
                .iter()
                .map(|v| labels.binary_search(v).unwrap_or(0))
                .collect();
            (codes, labels, None)
        };
        attrs.push(Attribute {
            name: field.name.clone(),
            data_type: input_data_type,
            optype: match field.measurement {
                Measurement::Continuous => "continuous",
                Measurement::Ordinal => "ordinal",
                _ => "categorical",
            },
            codes,
            labels,
            edges,
        });
    }
    Ok((
        Search {
            y,
            attrs,
            settings: s,
            minimum: s.min_group_size.max(0.05 * table.row_count() as f64),
            intervals: HashMap::new(),
            work: 0,
        },
        response,
    ))
}

fn data_type(storage: Storage) -> Result<&'static str, AlgoError> {
    match storage {
        Storage::String => Ok("string"),
        Storage::Integer => Ok("integer"),
        Storage::Real => Ok("double"),
        Storage::Date => Ok("date"),
        Storage::Time => Ok("time"),
        Storage::Timestamp => Ok("dateTime"),
        _ => Err(AlgoError::Unsupported(
            "GAP: Decision List non-scalar/unknown storage".into(),
        )),
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn compound(parts: &[String], op: &str) -> String {
    match parts {
        [] => "<True/>".into(),
        [one] => one.clone(),
        _ => format!(
            "<CompoundPredicate booleanOperator=\"{op}\">{}</CompoundPredicate>",
            parts.concat()
        ),
    }
}

/// Name-based ordered predicate syntax, including `and`/`or` structure and repeated
/// restrictions, as in the phase-3 harness. Field-index or XML-tag order is different.
fn predicate_key(rule: &Rule, attrs: &[Attribute]) -> String {
    fn compound_key(parts: &[String], op: &str) -> String {
        match parts {
            [] => "()".into(),
            [one] => one.clone(),
            _ => format!("({op:?}, ({}))", parts.join(", ")),
        }
    }
    let mut parts = Vec::new();
    for (f, cats) in rule {
        let Some(attr) = attrs.get(*f) else { continue };
        if let Some(edges) = &attr.edges {
            if let Some(lo) = cats
                .first()
                .and_then(|c| c.checked_sub(1))
                .and_then(|c| edges.get(c))
            {
                parts.push(format!("({:?}, \"greaterThan\", {lo:?})", attr.name));
            }
            if let Some(hi) = cats.last().and_then(|c| edges.get(*c)) {
                parts.push(format!("({:?}, \"lessOrEqual\", {hi:?})", attr.name));
            }
        } else {
            let alternatives: Vec<_> = cats
                .iter()
                .filter_map(|c| attr.labels.get(*c))
                .map(|value| format!("({:?}, \"equal\", {value:?})", attr.name))
                .collect();
            parts.push(compound_key(&alternatives, "or"));
        }
    }
    compound_key(&parts, "and")
}

/// Emit an effective conjunction once per field, preserving the seed field. Search retains
/// its ordered, unmerged conditions: normalizing before beam selection changes the search.
fn emission_rule(rule: &Rule, attrs: &[Attribute]) -> Result<Rule, AlgoError> {
    let mut out: Rule = Vec::new();
    for (f, cats) in rule {
        crate::check_cancelled()?;
        let attr = attrs
            .get(*f)
            .ok_or_else(|| AlgoError::Data("Decision List predicate field index".into()))?;
        if let Some((_, previous)) = out.iter_mut().find(|(field, _)| field == f) {
            if attr.edges.is_some() {
                let lo = previous.first().copied().zip(cats.first().copied());
                let hi = previous.last().copied().zip(cats.last().copied());
                if let (Some((a, b)), Some((c, d))) = (lo, hi) {
                    *previous = (a.max(b)..=c.min(d)).collect();
                } else {
                    previous.clear();
                }
            } else {
                previous.retain(|c| cats.contains(c));
            }
            if previous.is_empty() {
                return Err(AlgoError::Data(
                    "Decision List empty predicate intersection".into(),
                ));
            }
        } else {
            out.push((*f, cats.clone()));
        }
    }
    if let Some(tail) = out.get_mut(1..) {
        tail.sort_by_key(|(f, _)| attrs.get(*f).map(|a| a.name.as_str()));
    }
    Ok(out)
}

fn predicate(rule: &Rule, attrs: &[Attribute]) -> String {
    let mut parts = Vec::new();
    for (f, cats) in rule {
        let Some(a) = attrs.get(*f) else { continue };
        let simple = |op: &str, value: &str| {
            format!(
                "<SimplePredicate field=\"{}\" operator=\"{op}\" value=\"{}\"/>",
                escape(&a.name),
                escape(value)
            )
        };
        if let Some(edges) = &a.edges {
            if let Some(lo) = cats
                .first()
                .and_then(|c| c.checked_sub(1))
                .and_then(|c| edges.get(c))
            {
                parts.push(simple("greaterThan", &format!("{lo:.6}")));
            }
            if let Some(hi) = cats.last().and_then(|c| edges.get(*c)) {
                parts.push(simple("lessOrEqual", &format!("{hi:.6}")));
            }
        } else {
            let alternatives: Vec<String> = cats
                .iter()
                .filter_map(|c| a.labels.get(*c))
                .map(|v| simple("equal", v))
                .collect();
            parts.push(compound(&alternatives, "or"));
        }
    }
    compound(&parts, "and")
}

/// Train and emit PMML, including extensions not represented by the shared scoring IR.
/// Modeler calls the PMML model `RuleSet0`, independently of the nugget's display name.
pub fn train_xml(
    dataset: &Dataset<'_>,
    settings: &DecisionListSettings,
) -> Result<String, AlgoError> {
    let (mut search, response) = prepare(dataset, settings)?;
    let list = search
        .lists()?
        .iter()
        .map(|rule| emission_rule(rule, &search.attrs))
        .collect::<Result<RuleList, _>>()?;
    let target = dataset
        .table
        .model()
        .fields()
        .get(dataset.target)
        .ok_or_else(|| AlgoError::Data("Decision List target index".into()))?;
    let mut used = Vec::new();
    for rule in &list {
        crate::check_cancelled()?;
        for (f, _) in rule {
            if !used.contains(f) {
                used.push(*f);
            }
        }
    }
    let mut xml = String::from("<PMML version=\"4.3\"><DataDictionary>");
    let mut dictionary: Vec<usize> = (0..search.attrs.len()).collect();
    dictionary.sort_by_key(|f| search.attrs.get(*f).map(|a| a.name.as_str()));
    xml.push_str(&format!(
        "<DataField name=\"{}\" dataType=\"{}\" optype=\"categorical\"><Value value=\"{}\"/>",
        escape(&target.name),
        data_type(target.storage)?,
        escape(&response)
    ));
    for value in crate::categories(dataset, dataset.target)
        .iter()
        .filter(|v| **v != response)
    {
        crate::check_cancelled()?;
        xml.push_str(&format!("<Value value=\"{}\"/>", escape(value)));
    }
    xml.push_str("</DataField>");
    for f in dictionary {
        crate::check_cancelled()?;
        if let Some(a) = search.attrs.get(f) {
            xml.push_str(&format!(
                "<DataField name=\"{}\" dataType=\"{}\" optype=\"{}\">",
                escape(&a.name),
                a.data_type,
                a.optype
            ));
            for label in &a.labels {
                xml.push_str(&format!("<Value value=\"{}\"/>", escape(label)));
            }
            xml.push_str("</DataField>");
        }
    }
    xml.push_str("</DataDictionary><RuleSetModel algorithmName=\"DecisionList\" functionName=\"classification\" modelName=\"RuleSet0\"><MiningSchema>");
    for f in used {
        crate::check_cancelled()?;
        if let Some(a) = search.attrs.get(f) {
            xml.push_str(&format!("<MiningField name=\"{}\"/>", escape(&a.name)));
        }
    }
    xml.push_str(&format!("<MiningField name=\"{}\" usageType=\"predicted\"/></MiningSchema><RuleSet><RuleSelectionMethod criterion=\"firstHit\"/>",escape(&target.name)));
    let mut rest: Vec<usize> = (0..search.y.len()).collect();
    for (i, rule) in list.iter().chain(std::iter::once(&Vec::new())).enumerate() {
        crate::check_cancelled()?;
        let remainder = i == list.len();
        let cov = search.covered(&rest, rule)?;
        xml.push_str(&format!(
            "<SimpleRule id=\"{}\" nbCorrect=\"{}\" recordCount=\"{}\" score=\"{}\">",
            i + 1,
            search.positives(&cov),
            cov.len(),
            escape(&response)
        ));
        if remainder {
            xml.push_str("<Extension extender=\"spss.com\" name=\"label\" value=\"remainder\"/>");
        }
        xml.push_str(&format!("<Extension extender=\"spss.com\" name=\"isDisabled\" value=\"{remainder}\"/><Extension extender=\"spss.com\" name=\"searchDirection\" value=\"up\"/>{}</SimpleRule>",predicate(rule,&search.attrs)));
        let cov: HashSet<usize> = cov.into_iter().collect();
        rest.retain(|i| !cov.contains(i));
    }
    xml.push_str("</RuleSet></RuleSetModel></PMML>");
    Ok(xml)
}

pub fn train(dataset: &Dataset<'_>, settings: &DecisionListSettings) -> Result<Pmml, AlgoError> {
    crate::check_cancelled()?;
    Pmml::parse(&train_xml(dataset, settings)?)
        .map_err(|e| AlgoError::Data(format!("Decision List PMML: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dms_types::{DataModel, Field, Table, Value};

    fn sample(numeric: bool) -> Table {
        let mut x = Field::new(
            "segment<&\"",
            if numeric {
                Storage::Integer
            } else {
                Storage::String
            },
        );
        x.measurement = if numeric {
            Measurement::Continuous
        } else {
            Measurement::Nominal
        };
        let mut y = Field::new("respond", Storage::String);
        y.measurement = Measurement::Flag;
        y.values = ValueSpec::Flag {
            true_value: Value::string("Y"),
            false_value: Value::string("N"),
        };
        Table::new(
            DataModel::new(vec![x, y]).unwrap(),
            (0..200)
                .map(|i| {
                    vec![
                        if numeric {
                            Value::Integer(i)
                        } else {
                            Value::string(if i >= 140 { "z>" } else { "a<&" })
                        },
                        Value::string(if i >= 140 { "Y" } else { "N" }),
                    ]
                })
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn strict_size_and_xml_extensions_round_trip() {
        let table = sample(false);
        let ds = Dataset {
            table: &table,
            target: 1,
            inputs: &[0],
            frequency: None,
        };
        let settings = DecisionListSettings {
            min_group_size: 59.0,
            ..DecisionListSettings::default()
        };
        let xml = train_xml(&ds, &settings).unwrap();
        assert!(xml.contains("name=\"searchDirection\" value=\"up\""));
        assert!(xml.contains("name=\"label\" value=\"remainder\""));
        assert!(xml.contains("name=\"isDisabled\" value=\"true\""));
        assert!(xml.contains("field=\"segment&lt;&amp;&quot;\""));
        let model = Pmml::parse(&xml).unwrap();
        let dms_pmml::Model::RuleSet(rules) = model.model else {
            panic!("expected RuleSet")
        };
        assert_eq!(rules.rules.len(), 2);
        assert_eq!(rules.rules[0].record_count, Some(60.0));
        assert_eq!(rules.rules[0].correct, Some(60.0));
        assert!(rules.rules[1].disabled);
        let strict = DecisionListSettings {
            min_group_size: 60.0,
            ..settings
        };
        let dms_pmml::Model::RuleSet(rules) = train(&ds, &strict).unwrap().model else {
            panic!("expected RuleSet")
        };
        assert_eq!(rules.rules.len(), 1);
        assert_eq!(rules.rules[0].record_count, Some(200.0));
    }

    #[test]
    fn bin_edges_boundaries_and_numeric_pmml() {
        let table = sample(true);
        let ds = Dataset {
            table: &table,
            target: 1,
            inputs: &[0],
            frequency: None,
        };
        let settings = DecisionListSettings {
            bin_count: 4,
            ..DecisionListSettings::default()
        };
        let (search, _) = prepare(&ds, &settings).unwrap();
        assert_eq!(search.attrs[0].edges, Some(vec![49.5, 99.5, 149.5]));
        assert_eq!(search.attrs[0].codes[49], 0);
        assert_eq!(search.attrs[0].codes[50], 1);
        let xml = train_xml(&ds, &settings).unwrap();
        assert!(xml.contains("value=\"99.500000\""));
        assert!(xml.contains("dataType=\"integer\""));
        let width = DecisionListSettings {
            equal_width: true,
            ..settings
        };
        let (search, _) = prepare(&ds, &width).unwrap();
        assert_eq!(search.attrs[0].edges, Some(vec![49.75, 99.5, 149.25]));
    }

    #[test]
    fn invalid_settings_indices_and_missing_values_return_errors() {
        let table = sample(false);
        let ds = Dataset {
            table: &table,
            target: 1,
            inputs: &[0],
            frequency: None,
        };
        for settings in [
            DecisionListSettings {
                bin_count: usize::MAX,
                ..DecisionListSettings::default()
            },
            DecisionListSettings {
                rule_depth: usize::MAX,
                ..DecisionListSettings::default()
            },
            DecisionListSettings {
                list_width: 0,
                ..DecisionListSettings::default()
            },
            DecisionListSettings {
                confidence: f64::NAN,
                ..DecisionListSettings::default()
            },
            DecisionListSettings {
                confidence: 1.0 - f64::EPSILON * 0.5,
                ..DecisionListSettings::default()
            },
            DecisionListSettings {
                min_group_size: f64::INFINITY,
                ..DecisionListSettings::default()
            },
        ] {
            assert!(train(&ds, &settings).is_err());
        }
        let s = DecisionListSettings::default();
        assert!(
            train(
                &Dataset {
                    target: usize::MAX,
                    ..ds
                },
                &s
            )
            .is_err()
        );
        assert!(
            train(
                &Dataset {
                    inputs: &[usize::MAX],
                    ..ds
                },
                &s
            )
            .is_err()
        );
        assert!(
            train(
                &Dataset {
                    frequency: Some(0),
                    ..ds
                },
                &s
            )
            .is_err()
        );
        let (model, mut rows) = table.clone().into_parts();
        rows[0][0] = Value::Null;
        let table = Table::new(model, rows).unwrap();
        assert!(matches!(
            train(
                &Dataset {
                    table: &table,
                    ..ds
                },
                &s
            ),
            Err(AlgoError::Unsupported(_))
        ));
    }

    #[test]
    fn scipy_f_quantiles_and_clopper_pearson() {
        // Computed now with Python 3.13 / scipy.stats.f.ppf and beta.ppf, 2026-10-18.
        for (p, d1, d2, expected) in [
            (0.975, 2.0, 2.0, 38.999999999999964),
            (0.975, 34.0, 168.0, 1.6189604225419605),
            (0.975, 202.0, 2.0, 39.49293991698355),
            (0.9, 20.0, 40.0, 1.6051514663755269),
            (0.995, 1000.0, 1200.0, 1.1683696944561464),
        ] {
            assert!(
                (f_quantile(p, d1, d2) - expected).abs() < 1e-12 * expected.abs().max(1.0),
                "F({p},{d1},{d2})={} vs {expected}",
                f_quantile(p, d1, d2)
            );
        }
        for (x, n, c, lo, hi) in [
            (0, 100, 0.95, 0.0, 0.03621669264517641),
            (100, 100, 0.95, 0.9637833073548235, 1.0),
            (84, 100, 0.95, 0.7532124025891376, 0.9056897100260537),
            (298, 1200, 0.95, 0.22411282248816003, 0.2737935487680534),
            (42, 80, 0.8, 0.4474140338867472, 0.601614899379602),
            (1, 10000, 0.99, 5.01254056726572e-7, 0.000742774112396069),
            (
                500000,
                1000000,
                0.95,
                0.49901951919531184,
                0.5009804808046882,
            ),
            (
                123456,
                1000000,
                0.95,
                0.12281184162962974,
                0.1241023402768027,
            ),
            (
                2,
                1000000,
                0.95,
                2.4220937031600305e-7,
                7.2246687943652175e-6,
            ),
        ] {
            let actual = ci(x, n, c);
            assert!(
                (actual.0 - lo).abs() < 1e-12 && (actual.1 - hi).abs() < 1e-12,
                "{actual:?}"
            );
        }
    }
}
