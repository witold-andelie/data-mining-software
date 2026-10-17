//! RFM Analysis, Modeler 19.0 (public Scripting and Automation Guide, table 92).
//!
//! Settings: `rfmAnalysis{RecencyField,FrequencyField,MonetaryField,RecencyNumBins,
//! FrequencyNumBins,MonetaryNumBins,RecencyWeight,FrequencyWeight,MonetaryWeight,
//! RecencyScoreOutputFieldName,FrequencyScoreOutputFieldName,MonetaryScoreOutputFieldName,
//! RFMScoreOutputFieldName,AddOutliersToEndBins,BinValuesField}`, `recalculateBins`
//! (`always`, `ifnecessary`), `tiedValuesMethod` (`low` Current, `high` Next).
//! `BinValuesField` is only the editor's selected dimension. Display/repository metadata is
//! ignored; unknown settings, invalid counts/weights, random ties and malformed caches fail.
//!
//! Rules from `oracle/misc/`:
//! - `misc_rf_default`, `misc_rf_reverse`: three independent equal-count partitions; order
//!   and source fields survive. Numeric recency sorts descending, frequency/monetary ascending.
//!   Negative numeric recency participates in cut calculation but receives a null score.
//!   Each dimension omits its own nulls; any missing component makes the weighted score null.
//! - `misc_rf_default`, `misc_rf_next`: Current keeps a whole tie group in the current bin;
//!   Next moves a crossing group to the next bin when the current bin is nonempty. A new bin
//!   starts at most once per distinct value; cumulative targets are n/k, 2n/k, ... .
//! - `misc_rf_mixed_bins`, `misc_rf_nine_bins`: 2..9 bins independently; ties can leave fewer
//!   occupied bins. `misc_rf_constant`, `misc_rf_singleton`: a constant dimension scores 1.
//! - `misc_rf_date`, `misc_rf_timestamp`: temporal recency sorts ascending, preserving sub-day
//!   timestamp distinctions. Future dates remain valid. `misc_rf_empty`: empty output schema.
//! - `misc_rf_weights`: RFM Score is a real weighted sum, including fractional/zero weights.
//! - `misc_rf_always_limits`: Always recalculates; originally supplied thresholds were replaced
//!   in the saved stream. `misc_rf_ifnecessary`, `misc_rf_limits_null`, `misc_rf_limits_clamp`:
//!   IfNecessary reuses BinningOutput/IntervalTables, including their open/closed bounds and
//!   actual bin IDs. Numeric recency's cache omits its final bin: with outliers enabled,
//!   values at/below its lowest open bound clamp to the last *saved* bin (4 rather than 5).
//!   Other out-of-range values clamp to end bins or become null. Thus Always and IfNecessary
//!   intentionally differ at the extrema. These probes do not retain their original manual
//!   thresholds, so they cannot establish manually edited thresholds on a fresh stream.
//!
//! GAP: cacheless IfNecessary cannot distinguish a new node from discarded BinningOutput in
//! older stream readers and is rejected. User-missing values, negative weights, and replacing
//! existing score fields need probes; score-name collisions are rejected. Saving newly computed
//! bins back into a stream is outside the read-only execution API. Explicit user-missing
//! definitions are rejected until their effect on bin populations is measured.

use super::field_util::{Column, position, with_columns};
use super::rfm_settings::{reject_unknown, required_text};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::calendar::{MICROS_PER_DAY, days_from_civil};
use dms_types::{Field, Measurement, Storage, Table, Value};
use std::sync::Arc;

const SETTINGS: &[&str] = &[
    "rfmAnalysisRecencyField",
    "rfmAnalysisFrequencyField",
    "rfmAnalysisMonetaryField",
    "rfmAnalysisRecencyNumBins",
    "rfmAnalysisFrequencyNumBins",
    "rfmAnalysisMonetaryNumBins",
    "rfmAnalysisRecencyWeight",
    "rfmAnalysisFrequencyWeight",
    "rfmAnalysisMonetaryWeight",
    "rfmAnalysisRecencyScoreOutputFieldName",
    "rfmAnalysisFrequencyScoreOutputFieldName",
    "rfmAnalysisMonetaryScoreOutputFieldName",
    "rfmAnalysisRFMScoreOutputFieldName",
    "rfmAnalysisAddOutliersToEndBins",
    "rfmAnalysisBinValuesField",
    "recalculateBins",
    "tiedValuesMethod",
    "BinningOutput",
];

struct Dimension {
    field: String,
    name: String,
    bins: i64,
    weight: f64,
}

struct RfmAnalysis {
    dimensions: [Dimension; 3],
    score_name: String,
    next: bool,
    clamp: bool,
    cached: Option<[Vec<Interval>; 3]>,
}

#[derive(Clone)]
struct Interval {
    lower: f64,
    upper: f64,
    lower_closed: bool,
    upper_closed: bool,
    id: i64,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    reject_unknown(s, "", SETTINGS)?;
    let dimension = |name, default_weight| -> Result<Dimension, OperatorError> {
        let bins = settings::integer(s, &format!("rfmAnalysis{name}NumBins"))?.unwrap_or(5);
        if !(2..=9).contains(&bins) {
            return Err(bad(format!("{name}NumBins must be between 2 and 9")));
        }
        let weight =
            settings::real(s, &format!("rfmAnalysis{name}Weight"))?.unwrap_or(default_weight);
        if weight < 0.0 {
            return Err(bad("unsupported negative weight"));
        }
        Ok(Dimension {
            field: required_text(s, &format!("rfmAnalysis{name}Field"))?,
            name: settings::text(s, &format!("rfmAnalysis{name}ScoreOutputFieldName"))
                .unwrap_or_else(|| format!("{name} Score")),
            bins,
            weight,
        })
    };
    let dimensions = [
        dimension("Recency", 100.0)?,
        dimension("Frequency", 10.0)?,
        dimension("Monetary", 1.0)?,
    ];
    let score_name = settings::text(s, "rfmAnalysisRFMScoreOutputFieldName")
        .unwrap_or_else(|| "RFM Score".into());
    let mut names: Vec<&str> = dimensions.iter().map(|d| d.name.as_str()).collect();
    names.push(&score_name);
    if names.iter().any(|n| n.trim().is_empty()) {
        return Err(bad("empty score field name"));
    }
    names.sort_unstable();
    if names.windows(2).any(|pair| pair.first() == pair.get(1)) {
        return Err(bad("duplicate score field names"));
    }
    let next = match settings::text(s, "tiedValuesMethod").as_deref() {
        None | Some("low") => false,
        Some("high") => true,
        Some(other) => return Err(bad(format!("unsupported tiedValuesMethod {other:?}"))),
    };
    if let Some(field) = settings::text(s, "rfmAnalysisBinValuesField")
        && !["Recency", "Frequency", "Monetary"].contains(&field.as_str())
    {
        return Err(bad(format!("unsupported BinValuesField {field:?}")));
    }
    let cached = match settings::text(s, "recalculateBins").as_deref() {
        None | Some("always") => None,
        Some("ifnecessary") => Some(read_cache(s, &dimensions)?),
        Some(other) => return Err(bad(format!("unsupported recalculateBins {other:?}"))),
    };
    Ok(Box::new(RfmAnalysis {
        dimensions,
        score_name,
        next,
        clamp: settings::flag(s, "rfmAnalysisAddOutliersToEndBins")?.unwrap_or(true),
        cached,
    }))
}

impl Operator for RfmAnalysis {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("RFM Analysis needs one input".into()))?;
        if self
            .dimensions
            .iter()
            .any(|d| input.model().position(&d.name).is_some())
            || input.model().position(&self.score_name).is_some()
        {
            return Err(bad("unsupported score field name collision"));
        }
        let mut columns = Vec::new();
        for (index, dimension) in self.dimensions.iter().enumerate() {
            let p = position(input.model(), &dimension.field)?;
            if input
                .model()
                .fields()
                .get(p)
                .is_some_and(|f| !f.missing.values.is_empty() || !f.missing.ranges.is_empty())
            {
                return Err(bad("unsupported user-missing definitions"));
            }
            let storage = input.model().fields().get(p).map(|f| f.storage);
            let temporal =
                index == 0 && matches!(storage, Some(Storage::Date | Storage::Timestamp));
            if !temporal && !matches!(storage, Some(Storage::Integer | Storage::Real)) {
                return Err(bad(format!(
                    "unsupported storage for {:?}",
                    dimension.field
                )));
            }
            let values: Vec<&Value> = input
                .rows()
                .iter()
                .map(|row| {
                    let value = row.get(p).unwrap_or(&Value::Null);
                    number(value, temporal)?;
                    Ok(value)
                })
                .collect::<Result<_, _>>()?;
            let descending = index == 0 && !temporal;
            let scores = if let Some(cached) = &self.cached {
                let intervals = cached
                    .get(index)
                    .ok_or_else(|| bad("missing cached dimension"))?;
                values
                    .iter()
                    .map(|v| {
                        Ok(number(v, temporal)?
                            .and_then(|v| cached_score(v, intervals, self.clamp)))
                    })
                    .collect::<Result<_, OperatorError>>()?
            } else {
                tiles(&values, dimension.bins, descending, self.next)
            };
            let mut field = Field::new(&dimension.name, Storage::Integer);
            field.measurement = Measurement::Ordinal;
            columns.push(Column {
                field,
                values: scores
                    .into_iter()
                    .map(|v| v.map_or(Value::Null, Value::Integer))
                    .collect(),
            });
        }
        let mut combined = Vec::with_capacity(input.rows().len());
        for row in 0..input.rows().len() {
            let mut score = Some(0.0);
            for (column, dimension) in columns.iter().zip(&self.dimensions) {
                score = score
                    .zip(column.values.get(row).and_then(Value::as_f64))
                    .map(|(sum, value)| sum + value * dimension.weight);
            }
            if score.is_some_and(|v| !v.is_finite()) {
                return Err(OperatorError::Data(
                    "RFM Analysis: weighted score overflow".into(),
                ));
            }
            combined.push(score.map_or(Value::Null, Value::Real));
        }
        let mut field = Field::new(&self.score_name, Storage::Real);
        field.measurement = Measurement::Continuous;
        columns.push(Column {
            field,
            values: combined,
        });
        with_columns(input, columns)
    }
}

fn number(value: &Value, temporal: bool) -> Result<Option<f64>, OperatorError> {
    let number = match value {
        Value::Null => return Ok(None),
        Value::Date(days) if temporal => {
            let epoch = days_from_civil(1600, 1, 1).ok_or_else(|| bad("invalid temporal epoch"))?;
            days.checked_sub(epoch).map(|days| days as f64 * 86400.0)
        }
        Value::Timestamp(micros) if temporal => {
            let epoch = days_from_civil(1600, 1, 1)
                .and_then(|d| d.checked_mul(MICROS_PER_DAY))
                .ok_or_else(|| bad("invalid temporal epoch"))?;
            // Subtract as f64 to avoid overflowing i64 for malicious timestamp values.
            Some(*micros as f64 / 1_000_000.0 - epoch as f64 / 1_000_000.0)
        }
        value if !temporal => value.as_f64(),
        _ => None,
    };
    number
        .filter(|v| v.is_finite())
        .map(Some)
        .ok_or_else(|| OperatorError::Data("RFM Analysis: nonfinite or invalid input value".into()))
}

/// Cumulative equal-count targets, with whole tie groups. The same rule is used by Binning.
fn tiles(values: &[&Value], count: i64, descending: bool, next: bool) -> Vec<Option<i64>> {
    // Keep i64 integers and timestamp microseconds exact while sorting. Converting to f64
    // would merge adjacent large integers or sub-microsecond legacy-epoch representations.
    let mut ordered: Vec<(usize, &Value)> = values
        .iter()
        .enumerate()
        .filter(|(_, v)| !v.is_null())
        .map(|(i, v)| (i, *v))
        .collect();
    ordered.sort_by(|a, b| {
        if descending {
            b.1.total_cmp(a.1)
        } else {
            a.1.total_cmp(b.1)
        }
    });
    let mut scores = vec![None; values.len()];
    let n = i128::try_from(ordered.len()).unwrap_or(i128::MAX);
    let mut index = 0_usize;
    let mut bin = 1_i64;
    let mut in_bin = false;
    while let Some((_, value)) = ordered.get(index).copied() {
        let mut end = index.saturating_add(1);
        while ordered
            .get(end)
            .is_some_and(|(_, v)| v.total_cmp(value).is_eq())
        {
            end = end.saturating_add(1);
        }
        let placed = i128::try_from(index)
            .unwrap_or(i128::MAX)
            .saturating_mul(i128::from(count));
        let through = i128::try_from(end)
            .unwrap_or(i128::MAX)
            .saturating_mul(i128::from(count));
        let target = i128::from(bin).saturating_mul(n);
        if bin < count && (placed > target || (next && in_bin && through > target)) {
            bin = bin.saturating_add(1);
        }
        for &(source, value) in ordered.get(index..end).into_iter().flatten() {
            if let Some(slot) = scores.get_mut(source) {
                *slot = (!(descending && value.as_f64().is_some_and(|v| v < 0.0))).then_some(bin);
            }
        }
        in_bin = true;
        index = end;
    }
    scores
}

fn cached_score(value: f64, intervals: &[Interval], clamp: bool) -> Option<i64> {
    if let Some(bin) = intervals.iter().find(|bin| {
        (value > bin.lower || (bin.lower_closed && value == bin.lower))
            && (value < bin.upper || (bin.upper_closed && value == bin.upper))
    }) {
        return Some(bin.id);
    }
    if !clamp {
        return None;
    }
    let low = intervals
        .iter()
        .min_by(|a, b| a.lower.total_cmp(&b.lower))?;
    let high = intervals
        .iter()
        .max_by(|a, b| a.upper.total_cmp(&b.upper))?;
    if value <= low.lower {
        return Some(low.id);
    }
    if value >= high.upper {
        return Some(high.id);
    }
    // Never fill an internal gap in a malformed interval table.
    None
}

fn read_cache(
    s: Option<&Element>,
    dimensions: &[Dimension; 3],
) -> Result<[Vec<Interval>; 3], OperatorError> {
    let output = settings::element(s, "BinningOutput")
        .ok_or_else(|| bad("unsupported IfNecessary without retained BinningOutput"))?;
    let names = settings::children(Some(output), "BinningFieldNames", "binningFieldName");
    if !names.is_empty() {
        if names.len() != dimensions.len() {
            return Err(bad("invalid cached field mapping"));
        }
        for (name, dimension) in names.iter().zip(dimensions) {
            if name.attribute("fieldName") != Some(dimension.field.as_str())
                || name
                    .attribute("binningId")
                    .and_then(|id| id.parse::<i64>().ok())
                    != Some(dimension.bins)
            {
                return Err(bad("cached field mapping differs from current settings"));
            }
        }
    }
    let mut tables: [Option<Vec<Interval>>; 3] = std::array::from_fn(|_| None);
    for table in settings::children(Some(output), "IntervalTables", "IntervalTable") {
        let row = table
            .attribute("Row")
            .and_then(|v| v.parse::<usize>().ok())
            .ok_or_else(|| bad("invalid cached table Row"))?;
        if table.attribute("Col") != Some("0") {
            return Err(bad("unsupported cached table Col"));
        }
        let dimension = dimensions
            .get(row)
            .ok_or_else(|| bad("cached table Row out of range"))?;
        let target = tables
            .get_mut(row)
            .ok_or_else(|| bad("cached table Row out of range"))?;
        if target.is_some() {
            return Err(bad("duplicate cached table Row"));
        }
        let mut intervals = Vec::new();
        for interval in settings::children(Some(table), "rows", "row") {
            if intervals.len() >= 9 {
                return Err(bad("too many cached bins"));
            }
            let lower = settings::real(Some(interval), "lower")?
                .ok_or_else(|| bad("missing cached lower bound"))?;
            let upper = settings::real(Some(interval), "upper")?
                .ok_or_else(|| bad("missing cached upper bound"))?;
            let id = settings::integer(Some(interval), "binID")?
                .ok_or_else(|| bad("missing cached binID"))?;
            if lower > upper
                || !(1..=dimension.bins).contains(&id)
                || intervals.iter().any(|bin: &Interval| bin.id == id)
            {
                return Err(bad("invalid cached bounds or binID"));
            }
            let bin = Interval {
                lower,
                upper,
                id,
                lower_closed: settings::flag(Some(interval), "lowerClosed")?
                    .ok_or_else(|| bad("missing lowerClosed"))?,
                upper_closed: settings::flag(Some(interval), "upperClosed")?
                    .ok_or_else(|| bad("missing upperClosed"))?,
            };
            if intervals.iter().any(|other| {
                let lower = bin.lower.max(other.lower);
                let upper = bin.upper.min(other.upper);
                lower < upper
                    || (lower == upper
                        && (lower > bin.lower || bin.lower_closed)
                        && (lower < bin.upper || bin.upper_closed)
                        && (lower > other.lower || other.lower_closed)
                        && (lower < other.upper || other.upper_closed))
            }) {
                return Err(bad("overlapping cached intervals"));
            }
            intervals.push(bin);
        }
        *target = Some(intervals);
    }
    let [r, f, m] = tables;
    Ok([
        r.ok_or_else(|| bad("missing recency cache"))?,
        f.ok_or_else(|| bad("missing frequency cache"))?,
        m.ok_or_else(|| bad("missing monetary cache"))?,
    ])
}

fn bad(message: impl Into<String>) -> OperatorError {
    OperatorError::Settings(format!("RFM Analysis: {}", message.into()))
}
