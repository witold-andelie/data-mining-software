//! Binning (Field Operations): appends a bin or rank field for each selected numeric field.
//! Nulls stay null and are left out of the cut points. Input fields are unchanged.
//!
//! Implemented settings (saved names; scripting names in parentheses):
//! - `binningFields` (`fields`): the fields to bin, in that order. Each field's new columns are
//!   appended after the previous field's.
//! - `binningMethod` (`method`): `fixedWidth`, `equalFreq`, `rank`, or `aroundMean`.
//!   `optimal` is rejected.
//! - `recalculateBins` (`recalculate_bins`): `always` or `ifnecessary`. There is no saved bin
//!   cache, so both compute the cuts from the input.
//! - Fixed width (`fixedWidth`): `fixedBinMethod` (`width` or `count`), `fixedBinWidth`,
//!   `fixedBinCount`, `fixedBinExtensionName` (default `_BIN`), `fixedBinExtensionPosition`
//!   (`suffix` or `prefix`). `fixedBinSameBins` true is rejected.
//! - Tiles (`equalFreq`): `binTile4`, `binTile5`, `binTile10` (default true), `binTile20`,
//!   `binTile100`, `binTileExtensionName` (default `_TILE`), `binTileExtensionPosition`,
//!   `tilingMethod` (`count`; `sum` is rejected), `tiedValuesMethod` (`high` = add ties to the
//!   next bin, `low` = keep them in the current bin; `random` is rejected). `useBinTileN` true
//!   is rejected. The new name is the extension plus the tile count: `x` + `_TILE` + `4`.
//! - Ranks (`rank`): `binByRank`, `binByFractionalRank`, `binByPcFractionalRank` (each default
//!   true), `binRankExtensionName` (`_RANK`), `binFrRankExtensionName` (`_F_RANK`),
//!   `binPcRankExtensionName` (`_P_RANK`), `binRankExtensionPositions`, `reverseRank`.
//! - Standard deviation (`aroundMean`): `binMeanStdDevCount` (1, 2, or 3),
//!   `binMeanStdDevExtensionName` (`_SDBIN`), `binMeanStdDevExtensionPosition`.
//!
//! Display settings are ignored: `annotation`, `keywords`, `label`, `autoLabel`, `toolTipText`,
//! `cacheEnabled`, `enabled`, `name`, `position`, `doReadValues`, `fieldMappingEssentialFields`.
//! Optimal-binning settings are ignored unless `binningMethod` is `optimal`, which is rejected.
//!
//! Rules read from the oracle (`corpus/fields`, input `fields_main.csv`):
//! - `bin_width_x`: width bins are centered on the minimum. The saved interval table starts at
//!   `min - width/2` and steps by `width`, lower edge closed, upper edge open. `x` from -10 to
//!   20 with width 5 gives cuts -12.5, -7.5, -2.5, 2.5, ... so -10 is bin 1 and -2.5 is bin 3.
//!   The name is `x_BIN` (suffix `_BIN`). Bin ids are integers. A null `x` stays null.
//! - `bin_count_xy`: `count` bins of equal width from min to max, lower closed and upper open,
//!   and the maximum in the last bin (its upper edge is closed). Width is `(max-min)/count`.
//!   `x` then `y`, names `x_BIN`, `y_BIN`.
//! - `bin_quartile_next_x`: quartile (`binTile4`, `tiedValuesMethod` `high`). Non-null values are
//!   sorted. Bin `b` would end at record `b * n / k`. A tie group that would cross that end, and
//!   that is not the first group in the bin, is placed entirely in the next bin. Name `x_TILE4`.
//! - `bin_decile_current_xy`: the same cuts (`binTile10`, `tiedValuesMethod` `low`), but a tie
//!   group that crosses the end stays in the current bin. Names `x_TILE10`, `y_TILE10`. Counts
//!   are records (`tilingMethod` `count`), not a sum of values.
//! - `bin_rank_x`: average of the 1-based positions of a tie group (`x_RANK`, real). Nulls are
//!   not ranked. -10 (positions 1 and 2) is 1.5.
//! - `bin_rank_fractional_x`: that rank divided by the number of non-null values (`x_F_RANK`).
//! - `bin_rank_pct_xy`: the fractional rank times 100 (`x_P_RANK`, `y_P_RANK`).
//! - `bin_sdev1_x` and `bin_sdev2_xy`: sample standard deviation (divisor n-1; the saved cuts of
//!   `bin_sdev2_xy` are `mean ± k * sd`). Bin 0 is the closed interval `[mean-sd, mean+sd]`.
//!   Bin `k > 0` is `(mean+k*sd, mean+(k+1)*sd]`, and the outer bin is everything above
//!   `mean+count*sd`. Bin `k < 0` is `[mean+k*sd, mean+(k+1)*sd)`, and the outer bin is
//!   everything below `mean-count*sd`. `count` 1 yields -1, 0, 1 (`x_SDBIN`). Names `x_SDBIN`,
//!   `y_SDBIN` when both fields are selected.
//!
//! Not shown by a probe: a fixed width whose minimum is not a multiple of the width (the first
//! bin is still centered on the minimum); `reverseRank` (descending average rank is
//! `n + 1 - rank`); a prefix extension; more than one tile or rank column for the same field
//! (field by field, and within a field tiles 4, 5, 10, 20, 100 then rank, fractional rank,
//! percentile). New bin columns are ordinal integers and rank columns are continuous reals;
//! the table export does not show measurement level.

use super::field_util::{Column, extended_name, field_list, position, with_columns};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::{Field, Measurement, Storage, Table, Value};
use std::sync::Arc;

const TILES: [(u32, &str, bool); 5] = [
    (4, "binTile4", false),
    (5, "binTile5", false),
    (10, "binTile10", true),
    (20, "binTile20", false),
    (100, "binTile100", false),
];

enum Method {
    Fixed(Fixed),
    Tiles(Tiles),
    Rank(Rank),
    Sdev(Sdev),
}

struct Fixed {
    by: FixedBy,
    extension: String,
    prefix: bool,
}

enum FixedBy {
    Width(f64),
    Count(u32),
}

struct Tiles {
    counts: Vec<u32>,
    ties_next: bool,
    extension: String,
    prefix: bool,
}

struct Rank {
    rank: bool,
    fractional: bool,
    percentile: bool,
    reverse: bool,
    rank_extension: String,
    fractional_extension: String,
    percentile_extension: String,
    prefix: bool,
}

struct Sdev {
    count: i64,
    extension: String,
    prefix: bool,
}

struct Binning {
    fields: Vec<String>,
    method: Method,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let settings_element = node.settings.as_ref();
    let recalculate = settings::text(settings_element, "recalculateBins").unwrap_or_default();
    match recalculate.trim() {
        "" | "always" | "ifnecessary" => {}
        other => {
            return Err(OperatorError::Settings(format!(
                "recalculateBins: unknown value {other:?}"
            )));
        }
    }
    let fields = field_list(settings_element, "binningFields");
    if fields.is_empty() {
        return Err(OperatorError::Settings(
            "binningFields: no field selected".into(),
        ));
    }
    if fields
        .iter()
        .enumerate()
        .any(|(index, name)| fields.iter().take(index).any(|earlier| earlier == name))
    {
        return Err(OperatorError::Settings(
            "binningFields: a field is listed more than once".into(),
        ));
    }
    let method = match settings::text(settings_element, "binningMethod")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("fixedWidth") => Method::Fixed(parse_fixed(settings_element)?),
        Some("equalFreq") => Method::Tiles(parse_tiles(settings_element)?),
        Some("rank") => Method::Rank(parse_rank(settings_element)?),
        Some("aroundMean") => Method::Sdev(parse_sdev(settings_element)?),
        Some("optimal") => {
            return Err(OperatorError::Settings(
                "binningMethod: optimal is not supported yet".into(),
            ));
        }
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "binningMethod: unknown method {other:?}"
            )));
        }
    };
    Ok(Box::new(Binning { fields, method }))
}

fn parse_fixed(settings_element: Option<&Element>) -> Result<Fixed, OperatorError> {
    if settings::flag(settings_element, "fixedBinSameBins")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "fixedBinSameBins: shared bin boundaries are not supported yet".into(),
        ));
    }
    let by = match settings::text(settings_element, "fixedBinMethod")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("width") => {
            let width = settings::real(settings_element, "fixedBinWidth")?.unwrap_or(10.0);
            if !(width.is_finite() && width > 0.0) {
                return Err(OperatorError::Settings(
                    "fixedBinWidth: the width must be a positive number".into(),
                ));
            }
            FixedBy::Width(width)
        }
        Some("count") => {
            let count = settings::integer(settings_element, "fixedBinCount")?.unwrap_or(10);
            let count = u32::try_from(count).map_err(|_| {
                OperatorError::Settings(
                    "fixedBinCount: the number of bins must be at least 1".into(),
                )
            })?;
            if count == 0 {
                return Err(OperatorError::Settings(
                    "fixedBinCount: the number of bins must be at least 1".into(),
                ));
            }
            FixedBy::Count(count)
        }
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "fixedBinMethod: unknown method {other:?}"
            )));
        }
    };
    Ok(Fixed {
        by,
        extension: extension(settings_element, "fixedBinExtensionName", "_BIN"),
        prefix: prefix(settings_element, "fixedBinExtensionPosition")?,
    })
}

fn parse_tiles(settings_element: Option<&Element>) -> Result<Tiles, OperatorError> {
    if settings::flag(settings_element, "useBinTileN")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "useBinTileN: custom tiles are not supported yet".into(),
        ));
    }
    match settings::text(settings_element, "tilingMethod")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("count") => {}
        Some("sum") => {
            return Err(OperatorError::Settings(
                "tilingMethod: sum is not supported yet".into(),
            ));
        }
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "tilingMethod: unknown method {other:?}"
            )));
        }
    }
    let ties_next = match settings::text(settings_element, "tiedValuesMethod")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("high") => true,
        Some("low") => false,
        Some("random") => {
            return Err(OperatorError::Settings(
                "tiedValuesMethod: random is not supported yet".into(),
            ));
        }
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "tiedValuesMethod: unknown method {other:?}"
            )));
        }
    };
    let mut counts = Vec::new();
    for (count, name, default) in TILES {
        if settings::flag(settings_element, name)?.unwrap_or(default) {
            counts.push(count);
        }
    }
    if counts.is_empty() {
        return Err(OperatorError::Settings(
            "equalFreq: no tile is selected".into(),
        ));
    }
    Ok(Tiles {
        counts,
        ties_next,
        extension: extension(settings_element, "binTileExtensionName", "_TILE"),
        prefix: prefix(settings_element, "binTileExtensionPosition")?,
    })
}

fn parse_rank(settings_element: Option<&Element>) -> Result<Rank, OperatorError> {
    let rank = Rank {
        rank: settings::flag(settings_element, "binByRank")?.unwrap_or(true),
        fractional: settings::flag(settings_element, "binByFractionalRank")?.unwrap_or(true),
        percentile: settings::flag(settings_element, "binByPcFractionalRank")?.unwrap_or(true),
        reverse: settings::flag(settings_element, "reverseRank")?.unwrap_or(false),
        rank_extension: extension(settings_element, "binRankExtensionName", "_RANK"),
        fractional_extension: extension(settings_element, "binFrRankExtensionName", "_F_RANK"),
        percentile_extension: extension(settings_element, "binPcRankExtensionName", "_P_RANK"),
        prefix: prefix(settings_element, "binRankExtensionPositions")?,
    };
    if !rank.rank && !rank.fractional && !rank.percentile {
        return Err(OperatorError::Settings(
            "rank: no rank column is selected".into(),
        ));
    }
    Ok(rank)
}

fn parse_sdev(settings_element: Option<&Element>) -> Result<Sdev, OperatorError> {
    let count = settings::integer(settings_element, "binMeanStdDevCount")?.unwrap_or(1);
    if !(1..=3).contains(&count) {
        return Err(OperatorError::Settings(
            "binMeanStdDevCount: expected 1, 2, or 3".into(),
        ));
    }
    Ok(Sdev {
        count,
        extension: extension(settings_element, "binMeanStdDevExtensionName", "_SDBIN"),
        prefix: prefix(settings_element, "binMeanStdDevExtensionPosition")?,
    })
}

fn extension(settings_element: Option<&Element>, path: &str, default: &str) -> String {
    settings::text(settings_element, path)
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn prefix(settings_element: Option<&Element>, path: &str) -> Result<bool, OperatorError> {
    match settings::text(settings_element, path)
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("suffix") => Ok(false),
        Some("prefix") => Ok(true),
        Some(other) => Err(OperatorError::Settings(format!(
            "{path}: expected prefix or suffix, found {other:?}"
        ))),
    }
}

impl Operator for Binning {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Binning needs one input".into()))?;
        let mut columns = Vec::new();
        for name in &self.fields {
            let values = numeric_column(input, name)?;
            match &self.method {
                Method::Fixed(fixed) => columns.push(fixed.column(name, &values)),
                Method::Tiles(tiles) => columns.extend(tiles.columns(name, &values)),
                Method::Rank(rank) => columns.extend(rank.columns(name, &values)),
                Method::Sdev(sdev) => columns.push(sdev.column(name, &values)),
            }
        }
        let mut seen = Vec::new();
        for column in &columns {
            if seen.iter().any(|name| name == &column.field.name) {
                return Err(OperatorError::Data(format!(
                    "two bin columns are both named {:?}",
                    column.field.name
                )));
            }
            seen.push(column.field.name.clone());
        }
        with_columns(input, columns)
    }
}

fn numeric_column(input: &Table, name: &str) -> Result<Vec<Option<f64>>, OperatorError> {
    let index = position(input.model(), name)?;
    let field = input
        .model()
        .fields()
        .get(index)
        .ok_or_else(|| OperatorError::Data(format!("input has no field {name:?}")))?;
    if !matches!(field.storage, Storage::Integer | Storage::Real) {
        return Err(OperatorError::Data(format!(
            "{name}: only a numeric field can be binned"
        )));
    }
    let mut values = Vec::with_capacity(input.row_count());
    for (row_index, row) in input.rows().iter().enumerate() {
        let value = row.get(index).unwrap_or(&Value::Null);
        values.push(number(name, value, row_index)?);
    }
    Ok(values)
}

fn number(field: &str, value: &Value, row_index: usize) -> Result<Option<f64>, OperatorError> {
    match value {
        Value::Null => Ok(None),
        Value::Integer(integer) => Ok(Some(*integer as f64)),
        Value::Real(real) if real.is_finite() => Ok(Some(*real)),
        Value::Real(_) => Ok(None),
        _ => Err(OperatorError::Data(format!(
            "{field}, record {}: only a numeric field can be binned",
            super::field_util::record_number(row_index)
        ))),
    }
}

fn present(values: &[Option<f64>]) -> Vec<f64> {
    values.iter().copied().flatten().collect()
}

fn ordinal(name: String, values: Vec<Value>) -> Column {
    let mut field = Field::new(name, Storage::Integer);
    field.measurement = Measurement::Ordinal;
    Column { field, values }
}

fn continuous(name: String, values: Vec<Value>) -> Column {
    let mut field = Field::new(name, Storage::Real);
    field.measurement = Measurement::Continuous;
    Column { field, values }
}

impl Fixed {
    fn column(&self, field: &str, values: &[Option<f64>]) -> Column {
        let numbers = present(values);
        let bins: Vec<Option<i64>> = match self.by {
            FixedBy::Width(width) => width_bins(&numbers, width),
            FixedBy::Count(count) => count_bins(&numbers, count),
        };
        let assigned = assign(values, &bins, Value::Integer);
        ordinal(extended_name(field, &self.extension, self.prefix), assigned)
    }
}

/// Bins of `width` centered on the minimum (`bin_width_x`).
fn width_bins(numbers: &[f64], width: f64) -> Vec<Option<i64>> {
    let Some(min) = numbers.iter().copied().reduce(f64::min) else {
        return Vec::new();
    };
    let offset = min - width / 2.0;
    numbers
        .iter()
        .map(|value| {
            let raw = ((*value - offset) / width).floor();
            Some(bounded_bin(raw).saturating_add(1).max(1))
        })
        .collect()
}

/// `count` equal-width bins from min to max (`bin_count_xy`). The maximum is in the last bin.
fn count_bins(numbers: &[f64], count: u32) -> Vec<Option<i64>> {
    let Some(min) = numbers.iter().copied().reduce(f64::min) else {
        return Vec::new();
    };
    let Some(max) = numbers.iter().copied().reduce(f64::max) else {
        return Vec::new();
    };
    let count_f = f64::from(count);
    let span = max - min;
    numbers
        .iter()
        .map(|value| {
            if !span.is_finite() || span == 0.0 {
                return Some(1);
            }
            let raw = ((*value - min) / (span / count_f)).floor();
            let bin = bounded_bin(raw).saturating_add(1);
            Some(bin.clamp(1, i64::from(count)))
        })
        .collect()
}

fn bounded_bin(raw: f64) -> i64 {
    if raw >= i64::MAX as f64 {
        i64::MAX
    } else if raw <= i64::MIN as f64 {
        i64::MIN
    } else {
        raw as i64
    }
}

impl Tiles {
    fn columns(&self, field: &str, values: &[Option<f64>]) -> Vec<Column> {
        let numbers = present(values);
        self.counts
            .iter()
            .map(|count| {
                let bins = tile_bins(&numbers, *count, self.ties_next);
                let assigned = assign(values, &bins, Value::Integer);
                let extension = format!("{}{count}", self.extension);
                ordinal(extended_name(field, &extension, self.prefix), assigned)
            })
            .collect()
    }
}

/// Equal-count tiles. `ties_next` is `tiedValuesMethod` `high` (`bin_quartile_next_x`);
/// otherwise ties stay in the current bin (`bin_decile_current_xy`).
fn tile_bins(numbers: &[f64], tiles: u32, ties_next: bool) -> Vec<Option<i64>> {
    let n = numbers.len();
    if n == 0 || tiles == 0 {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&left, &right| {
        numbers
            .get(left)
            .copied()
            .unwrap_or(0.0)
            .total_cmp(&numbers.get(right).copied().unwrap_or(0.0))
    });
    let mut sorted_bins = vec![1_i64; n];
    let mut bin_no: i64 = 1;
    let mut placed: i128 = 0;
    let mut in_bin: i128 = 0;
    let k = i128::from(tiles);
    let n_i = i128::try_from(n).unwrap_or(i128::MAX);
    let mut index = 0;
    while index < n {
        let Some(value) = order
            .get(index)
            .and_then(|&source| numbers.get(source))
            .copied()
        else {
            break;
        };
        let mut end = index.saturating_add(1);
        while end < n
            && order
                .get(end)
                .and_then(|&source| numbers.get(source))
                .copied()
                == Some(value)
        {
            end = end.saturating_add(1);
        }
        let size = i128::try_from(end.saturating_sub(index)).unwrap_or(i128::MAX);
        let past = placed.saturating_mul(k) > bin_no as i128 * n_i;
        let crosses = placed.saturating_add(size).saturating_mul(k) > bin_no as i128 * n_i;
        if (bin_no as u32) < tiles
            && (if ties_next {
                in_bin > 0 && (past || crosses)
            } else {
                past
            })
        {
            bin_no = bin_no.saturating_add(1);
            in_bin = 0;
        }
        for slot in sorted_bins.get_mut(index..end).into_iter().flatten() {
            *slot = bin_no;
        }
        placed = placed.saturating_add(size);
        in_bin = in_bin.saturating_add(size);
        index = end;
    }
    let mut bins = vec![None; n];
    for (sorted_index, source) in order.into_iter().enumerate() {
        if let Some(slot) = bins.get_mut(source) {
            *slot = sorted_bins.get(sorted_index).copied();
        }
    }
    bins
}

impl Rank {
    fn columns(&self, field: &str, values: &[Option<f64>]) -> Vec<Column> {
        let numbers = present(values);
        let ranks = average_ranks(&numbers, self.reverse);
        let n = numbers.len();
        let mut columns = Vec::new();
        if self.rank {
            columns.push(continuous(
                extended_name(field, &self.rank_extension, self.prefix),
                assign(values, &ranks, Value::Real),
            ));
        }
        if self.fractional {
            let scaled = scale(&ranks, |rank| rank / n as f64);
            columns.push(continuous(
                extended_name(field, &self.fractional_extension, self.prefix),
                assign(values, &scaled, Value::Real),
            ));
        }
        if self.percentile {
            let scaled = scale(&ranks, |rank| rank / n as f64 * 100.0);
            columns.push(continuous(
                extended_name(field, &self.percentile_extension, self.prefix),
                assign(values, &scaled, Value::Real),
            ));
        }
        columns
    }
}

/// Average 1-based rank of each tie group (`bin_rank_x`). `reverse` is `n + 1 - rank`.
fn average_ranks(numbers: &[f64], reverse: bool) -> Vec<Option<f64>> {
    let n = numbers.len();
    if n == 0 {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&left, &right| {
        numbers
            .get(left)
            .copied()
            .unwrap_or(0.0)
            .total_cmp(&numbers.get(right).copied().unwrap_or(0.0))
    });
    let mut ranks = vec![None; n];
    let n_f = n as f64;
    let mut index = 0;
    while index < n {
        let Some(value) = order
            .get(index)
            .and_then(|&source| numbers.get(source))
            .copied()
        else {
            break;
        };
        let mut end = index.saturating_add(1);
        while end < n
            && order
                .get(end)
                .and_then(|&source| numbers.get(source))
                .copied()
                == Some(value)
        {
            end = end.saturating_add(1);
        }
        let first = index as f64 + 1.0;
        let last = end as f64;
        let mut rank = (first + last) / 2.0;
        if reverse {
            rank = n_f + 1.0 - rank;
        }
        for source in order.get(index..end).into_iter().flatten() {
            if let Some(slot) = ranks.get_mut(*source) {
                *slot = Some(rank);
            }
        }
        index = end;
    }
    ranks
}

fn scale(ranks: &[Option<f64>], scale: impl Fn(f64) -> f64) -> Vec<Option<f64>> {
    ranks.iter().map(|rank| rank.map(&scale)).collect()
}

impl Sdev {
    fn column(&self, field: &str, values: &[Option<f64>]) -> Column {
        let numbers = present(values);
        let bins = match mean_and_sample_sd(&numbers) {
            Some((mean, sd)) => numbers
                .iter()
                .map(|value| Some(sdev_bin((*value - mean) / sd, self.count)))
                .collect(),
            None => vec![Some(0); numbers.len()],
        };
        ordinal(
            extended_name(field, &self.extension, self.prefix),
            assign(values, &bins, Value::Integer),
        )
    }
}

/// Sample mean and standard deviation (divisor n-1), updated one record at a time.
fn mean_and_sample_sd(values: &[f64]) -> Option<(f64, f64)> {
    if values.len() < 2 {
        return None;
    }
    let (mut mean, mut squares) = (0.0_f64, 0.0_f64);
    for (k, &value) in values.iter().enumerate() {
        let delta = value - mean;
        mean += delta / (k as f64 + 1.0);
        squares += delta * (value - mean);
    }
    let variance = squares / (values.len() as f64 - 1.0);
    let sd = variance.sqrt();
    (sd.is_finite() && sd > 0.0).then_some((mean, sd))
}

/// Bin of a standard-deviation z-score. See the module docs for the closures.
fn sdev_bin(z: f64, count: i64) -> i64 {
    if !z.is_finite() {
        return 0;
    }
    let limit = count as f64;
    if z < -limit {
        return -count;
    }
    if z > limit {
        return count;
    }
    if (-1.0..=1.0).contains(&z) {
        return 0;
    }
    if z > 1.0 {
        let bin = bounded_bin(z.ceil()).saturating_sub(1);
        return bin.clamp(1, count);
    }
    let bin = bounded_bin((z + 1.0).floor());
    bin.clamp(-count, -1)
}

fn assign<T>(
    values: &[Option<f64>],
    bins: &[Option<T>],
    to_value: impl Fn(T) -> Value,
) -> Vec<Value>
where
    T: Copy,
{
    let mut bins = bins.iter();
    values
        .iter()
        .map(|value| match value {
            None => Value::Null,
            Some(_) => bins
                .next()
                .copied()
                .flatten()
                .map(&to_value)
                .unwrap_or(Value::Null),
        })
        .collect()
}
