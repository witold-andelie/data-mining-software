//! Decision List, a faithful port of the verified `dl-lab/dl.py` prototype.
//! Sources: IBM SPSS Modeler 19 Algorithms Guide pp.105–111 (split pp.108–110),
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
//! - Rule beam returns selected rules from all levels, plus terminal rules; list beam
//!   ranks union coverage rates on the remaining records and returns the best list of
//!   the deepest available level (`dl_max_rules2`, `dl_max_rules4`, `dl_search`). Both
//!   sorts are stable, preserving input-field order. A disabled True remainder follows.
//! - Non-expert mode ignores confidence, rule width/depth, list width, binning and growth:
//!   defaults 95%, 5, 5, 5, equalFreq/10, 2 (`dl_confidence80` equals `dl_default`).
//!   List depth and minimum group size always apply. Expert settings: `dl_search`, `dl_numeric`.
//! - Lists of equal rate prefer the one covering fewer records (`train_next/dl_max_rules2`:
//!   126 of 180 over 140 of 200); equivalent rules covering the same records keep the
//!   stable order (`train_next/dl_default`: `region = east` before `spend > 40`).
//! - GAP: lists tying on rate and coverage with different per-rule splits (permutations of
//!   pure segments) are refused, so the nugget keeps its saved model:
//!   `models_next2/dl_default` (Modeler: north, web, gold) and `dl_max_rules` (web, gold).
//!   No oracle for searchDirection down, reuseFields=false, weights/frequencies, missing
//!   inputs; these are rejected.

use crate::stats::f_quantile;
use crate::{AlgoError, Dataset};
use dms_pmml::Pmml;
use dms_types::{Measurement, Role, Storage, ValueSpec};
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
/// A candidate list, its response rate, the records it covers and, per rule, the records and
/// responses it claims.
type RankedList = (RuleList, f64, usize, Vec<(usize, usize)>);

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
        for _ in 0..self.settings.rule_depth {
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
            result.extend(beam.iter().cloned());
        }
        Ok(result)
    }

    fn lists(&mut self) -> Result<RuleList, AlgoError> {
        let all: Vec<usize> = (0..self.y.len()).collect();
        let mut kept: Vec<RankedList> = vec![(Vec::new(), 0.0, 0, Vec::new())];
        let mut last_level: Vec<RankedList> = Vec::new();
        for _ in 0..self.settings.list_depth {
            let beam: Vec<RuleList> = kept.iter().map(|(l, ..)| l.clone()).collect();
            let mut candidates = Vec::new();
            for list in &beam {
                let mut rest = all.clone();
                for rule in list {
                    let cov: HashSet<usize> = self.covered(&rest, rule)?.into_iter().collect();
                    rest.retain(|i| !cov.contains(i));
                }
                for rule in self.rules(&rest)? {
                    let mut next = list.clone();
                    next.push(rule);
                    let mut cov = HashSet::new();
                    for r in &next {
                        cov.extend(self.covered(&all, r)?);
                    }
                    let records: Vec<usize> = cov.into_iter().collect();
                    candidates.push((next, self.rate(&records)));
                    candidate_limit(candidates.len())?;
                }
            }
            if candidates.is_empty() {
                break;
            }
            // Equal rates prefer the list covering fewer records (`train_next/dl_max_rules2`:
            // 126 of 180 over 140 of 200, both 0.7); the sort is stable otherwise.
            let mut keyed = Vec::with_capacity(candidates.len());
            for (list, rate) in candidates {
                let signature = self.signature(&all, &list)?;
                let total: usize = signature.iter().map(|(n, _)| n).sum();
                keyed.push((list, rate, total, signature));
            }
            keyed.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.2.cmp(&b.2)));
            last_level = keyed.clone();
            keyed.truncate(self.settings.list_width);
            kept = keyed;
        }
        // GAP: lists that tie on rate and coverage but split the records differently rule by
        // rule (permutations of pure segments, `models_next2/dl_default` and `dl_max_rules`:
        // Modeler keeps north, web, gold and web, gold) are not settled. Equivalent rules
        // that cover the same records keep the stable order (`train_next/dl_default`:
        // `region = east` before `spend > 40`).
        if let Some((_, top, total, signature)) = last_level.first()
            && last_level
                .iter()
                .skip(1)
                .any(|(_, rate, t, sig)| rate == top && t == total && sig != signature)
        {
            return Err(AlgoError::Unsupported(
                "GAP: Decision List lists tie on rate and coverage in different orders".into(),
            ));
        }
        Ok(kept.into_iter().next().map(|(l, ..)| l).unwrap_or_default())
    }

    /// Records and responses each rule of a list claims, in list order.
    fn signature(
        &mut self,
        all: &[usize],
        list: &RuleList,
    ) -> Result<Vec<(usize, usize)>, AlgoError> {
        let mut rest = all.to_vec();
        let mut out = Vec::with_capacity(list.len());
        for rule in list {
            let cov: HashSet<usize> = self.covered(&rest, rule)?.into_iter().collect();
            let responses = cov
                .iter()
                .filter(|&&i| self.y.get(i) == Some(&true))
                .count();
            out.push((cov.len(), responses));
            rest.retain(|i| !cov.contains(i));
        }
        Ok(out)
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
    let list = search.lists()?;
    let target = dataset
        .table
        .model()
        .fields()
        .get(dataset.target)
        .ok_or_else(|| AlgoError::Data("Decision List target index".into()))?;
    let mut used = Vec::new();
    for rule in &list {
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
        xml.push_str(&format!("<Value value=\"{}\"/>", escape(value)));
    }
    xml.push_str("</DataField>");
    for f in dictionary {
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
        if let Some(a) = search.attrs.get(f) {
            xml.push_str(&format!("<MiningField name=\"{}\"/>", escape(&a.name)));
        }
    }
    xml.push_str(&format!("<MiningField name=\"{}\" usageType=\"predicted\"/></MiningSchema><RuleSet><RuleSelectionMethod criterion=\"firstHit\"/>",escape(&target.name)));
    let mut rest: Vec<usize> = (0..search.y.len()).collect();
    for (i, rule) in list.iter().chain(std::iter::once(&Vec::new())).enumerate() {
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
