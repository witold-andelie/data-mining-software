//! Auto Data Prep (`AutoDataPrepNode`, catalog id `auto_data_prep`).
//!
//! The node analyses its input at execution (the `misc_next` streams were saved before Modeler
//! ran them) and rebuilds the field list. Rules below are the ones the oracle exports show.
//! Anything marked `GAP:` is not settled by those exports: the saved flag is accepted when the
//! export is unchanged with the flag on or off, and a value that would need the missing behavior
//! is rejected with [`OperatorError::Settings`].
//!
//! Field order (`adp_balanced` / `adp_rescale` header `id,txt,ts,y,pos_transformed,iv_transformed,
//! rv_transformed,dt_months,grp_transformed,flg_transformed`; `adp_dates` inserts `dt_year` after
//! `y` and replaces `dt_months` with `dt_days,dt_month,dt_day`):
//! 1. Unchanged fields, input order. `id` (role none), `txt` (typeless), `ts` (timestamp) and
//!    `y` (target) in all three cases.
//! 2. Date elements that stayed ordinal, extraction order (year, month, day). `adp_dates`
//!    `dt_year` (3 distinct years, threshold 10).
//! 3. Rescaled original continuous inputs. `adp_balanced` order is `pos_transformed`,
//!    `iv_transformed`, `rv_transformed`.
//!    `GAP:` that order matches ascending missing count, then input index (`pos` has no missing
//!    value, `iv` and `rv` have one, and `iv` precedes `rv`). No second case distinguishes this
//!    key from another key with the same result on these three fields.
//! 4. Date derivatives that are continuous: the duration, then recast elements in extraction
//!    order. `dt_months` (`adp_balanced`, unit month); `dt_days`, `dt_month`, `dt_day`
//!    (`adp_dates`, unit day, month and day recast).
//! 5. Recoded categoricals, input order: `grp_transformed`, `flg_transformed`.
//!
//! Rescale (`adp_balanced`, `adp_rescale`, `adp_dates`): on finite non-missing values, mean is
//! sum/n and the sum of squares is about that mean. The standard deviation divides by
//! `n_rows - 1`, where `n_rows` counts rows that will be written (missing rows included when
//! mean imputation is on, so imputed rows add 0 to the sum of squares and increase the divisor).
//! Output is `(x - mean) / sd * zScore_SD + zScore_mean`. `zScore_mean` 0 and `zScore_SD` 1 in
//! the three cases. A zero standard deviation writes the final mean. Missing continuous values
//! become the final mean when `uiReplaceScaleFeaturesMissingValue` is true (`iv`, `rv`, and the
//! blank date's `dt_months` / `dt_days` / `dt_month` / `dt_day` are 0).
//!
//! Outliers (`uiDetectFeatureOutliers`, `adp_next/adp_outlier_replace` / `_delete`): before
//! rescaling, a continuous input's values beyond the robust mean +/- `outlierIdentCutoff` robust
//! standard deviations (guide pp.18-19, the trimmed moments shared with Linear's ADP) are coerced
//! to the bound (`coerceToCutoffVal`) or set missing (`setToMissingVal`, then imputed and counted
//! as missing for field order). `c_out`: 60 and -45 against bounds 0.00503 +/- 3 x 2.92449.
//!
//! Min-max (`scaleContinuousMode` `minMaxTransform`, `adp_next/adp_minmax`): `(x - min) /
//! (max - min) * (minMax_max - minMax_min) + minMax_min` over the finite values. Date fields
//! under min-max have no oracle and are a `GAP:` error.
//!
//! Dates (`prepareDatesAndTimes`, `uiComputeDateDuration`): a fixed unit of `month` divides the
//! day number by 30.4375 (`adp_balanced`, suffix `_months`); `day` keeps the day number
//! (`adp_dates`, suffix `_days`). The fixed reference (`referenceDateType` `fixedDate`) is a
//! constant offset and cancels under the z-score. `adp_dates` also extracts year, month and day.
//! An element with more than `ordinalToScaleCategoriesNumThreshold` distinct non-missing values
//! (saved 10) is recast to continuous and z-scored (`dt_month` 11 months, `dt_day` 15 days).
//! `dt_year` has 3 years, stays an integer ordinal, and its missing value stays null
//! (`uiReplaceOrdinalFeaturesMissingValue` false). The original date is dropped.
//!
//! Categories (`adp_balanced` `grp`, `flg`): non-missing values and system-null are counted.
//! Codes are strings `"0"`, `"1"`, ... from the least frequent category; equal counts break by
//! [`Value::total_cmp`] (null before any other value). `grp` empty string is a real category
//! (`""` once → `"0"`). `flg` null once → `"0"`, `F` → `"1"`, `T` → `"2"`. The new field is a
//! nominal string named with `nameExtForInputs` (`_transformed`). The target `y` is not recoded.
//!
//! Screening (saved on in the three cases; no corpus field crosses a threshold): an input is
//! dropped when its missing share is greater than `maxNumberOfMissingValue` (50), when a
//! categorical input has more than `maxNOfUniqueCategories` (100) non-missing categories, or
//! when `excludePoorQualityFeatures` sees fewer than two distinct non-missing values. Role-none
//! and typeless fields are kept (`id`, `txt`). `excludeNoUsedField` `filterOut` drops inputs
//! that were replaced by a transformed field.
//!
//! `analysisObjective` `balance` (`adp_balanced`) and `custom` (`adp_rescale`, `adp_dates`) do
//! not change the math; the saved flags do. `adp_balanced` and `adp_rescale` export the same
//! table.
//!
//! `GAP:` `uiSupervisedMerging`, `uiSupervisedBinning`, `uiFeatureSelection` and
//! `transConsSelectFeatures` are true in `adp_balanced` and false in `adp_rescale` / `adp_dates`.
//! The exports match, so categories are not merged, continuous fields stay z-scores rather than
//! bins, and no field is dropped for predictive power. The p-values are validated and not applied.
//! `GAP:` `uiReplaceNominalFeaturesMissingValue` true does not substitute the mode (see `flg`).
//! false is rejected. `GAP:` timestamp `ts` is copied in all three cases while
//! `uiComputeTimeDuration` is true. A time-of-day field with that flag is rejected.
//! `GAP:` target outliers, target Box-Cox / target missing /
//! target reorder, unsupervised merge, feature construction, almost-constant category screening,
//! frequency and analysis weights, `useCustomSettings`, a non-empty `inputFields` / `targetField`,
//! automatic date or time units, `dateUnit` `year`, and `excludeNoUsedField` other than
//! `filterOut` are rejected. `GAP:` original ordinal inputs are copied, not recoded (no corpus
//! ordinal input). `GAP:` a zero-row input is copied unchanged.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::calendar::civil_from_days;
use dms_types::{DataModel, Field, Measurement, Role, Storage, Table, Value};
use std::cmp::Ordering;
use std::sync::Arc;

const MAX_FIELDS: usize = 256;
const MAX_ROWS: usize = 1_000_000;
const MAX_CELLS: usize = 16_000_000;
const MAX_CATEGORIES: usize = 10_000;

const GROUP_PASS: u8 = 1;
const GROUP_DATE_ORDINAL: u8 = 2;
const GROUP_CONTINUOUS: u8 = 3;
const GROUP_DATE_SCALE: u8 = 4;
const GROUP_CATEGORICAL: u8 = 5;

const METADATA: &[&str] = &[
    "annotation",
    "keywords",
    "lastSavedAs",
    "enabled",
    "fieldMappingEssentialFields",
    "repositoryPath",
    "repositoryVersionMarker",
    "doReadValues",
    "toolTipText",
    "lastSavedBy",
    "lastSavedDate",
    "label",
    "creationDate",
    "repositoryLastModified",
    "cacheEnabled",
    "scriptExecutionModeChange",
    "autoLabel",
    "createdBy",
    "name",
    "position",
];

const KNOWN: &[&str] = &[
    "analysisObjective",
    "analysisWeight",
    "autoChooseDateFieldUnit",
    "autoChooseTimeFieldUnit",
    "cateFeatureScreenNumber",
    "clearAnalysis",
    "currentTime",
    "dateUnit",
    "excludeNoUsedField",
    "excludePoorQualityFeatures",
    "featureSelectionPValue",
    "fixedDate",
    "fixedTime",
    "frequencyWeight",
    "homogeneousSubsetsPValue",
    "inputFields",
    "keepOrClearAnalysis",
    "maxNOfUniqueCategories",
    "maxNumberOfMissingValue",
    "minMax_max",
    "minMax_min",
    "modelExist",
    "nameExtForInputs",
    "nameExtForTarget",
    "ordinalToScaleCategoriesNumThreshold",
    "outlierIdentCutoff",
    "outliersOperation",
    "prepareDatesAndTimes",
    "prepareFeatures",
    "referenceDateType",
    "referenceTimeType",
    "rootNameForConsturctedFeature",
    "scaleContinuousMode",
    "scaleToOrdinalValuesNumThreshold",
    "suffixDay",
    "suffixElapsedDays",
    "suffixElapsedHours",
    "suffixElapsedMinutes",
    "suffixElapsedMonths",
    "suffixElapsedSeconds",
    "suffixElapsedYears",
    "suffixHour",
    "suffixMinute",
    "suffixMonth",
    "suffixSecond",
    "suffixYear",
    "supervisedMergingPValue",
    "targetField",
    "targetMean",
    "targetSD",
    "timeUnit",
    "todaysDate",
    "transConsSelectFeatures",
    "trimBlanks",
    "uiCheckFeatureMeasureLevel",
    "uiCheckTargetMeasureLevel",
    "uiComputeDateDuration",
    "uiComputeTimeDuration",
    "uiDetectFeatureOutliers",
    "uiDetectTargetOutliers",
    "uiExcludeAlmostConstantCatFeature",
    "uiExcludeFeatureIfHasHigherPercentageOfMissingValue",
    "uiExcludeFeatureIfHasManyUniqueCatogories",
    "uiExtractDayFromDate",
    "uiExtractHourFromTime",
    "uiExtractMinuteFromTime",
    "uiExtractMonthFromDate",
    "uiExtractSecondFromTime",
    "uiExtractYearFromDate",
    "uiFeatureConstruction",
    "uiFeatureSelection",
    "uiReorderNominalCatsPriorToMerging",
    "uiReorderNominalTargetCategories",
    "uiReplaceNominalFeaturesMissingValue",
    "uiReplaceNominalTargetMissingValue",
    "uiReplaceOrdinalFeaturesMissingValue",
    "uiReplaceOrdinalTargetMissingValue",
    "uiReplaceScaleFeaturesMissingValue",
    "uiReplaceScaleTargetMissingValue",
    "uiRescaleScaleTargetUsingBoxCox",
    "uiScaleContinuousFeatures",
    "uiSupervisedBinning",
    "uiSupervisedMerging",
    "uiUnsupervisedMergeNominal",
    "uiUnsupervisedMergeOrdinal",
    "unsupervisedMergingMinNumberOfCases",
    "useCustomSettings",
    "useFrequency",
    "useWeight",
    "zScore_SD",
    "zScore_mean",
];

struct Prep {
    exclude_high_missing: bool,
    max_missing_pct: f64,
    exclude_many_cats: bool,
    max_unique: i64,
    exclude_poor: bool,
    reorder_nominal: bool,
    replace_scale_missing: bool,
    scale_continuous: bool,
    scale: Scale,
    outliers: Option<Outliers>,
    check_feature_level: bool,
    ordinal_to_scale: i64,
    scale_to_ordinal: i64,
    prepare_dates: bool,
    compute_date_duration: bool,
    date_divisor: f64,
    date_suffix: String,
    ref_days: f64,
    extract_year: bool,
    extract_month: bool,
    extract_day: bool,
    suffix_year: String,
    suffix_month: String,
    suffix_day: String,
    name_ext: String,
    trim_blanks: bool,
    prepare_features: bool,
    compute_time_duration: bool,
}

/// `scaleContinuousMode`.
enum Scale {
    /// `zScoreTransform`: `(x - mean) / sd * zScore_SD + zScore_mean`.
    ZScore { mean: f64, sd: f64 },
    /// `minMaxTransform`: `(x - min) / (max - min) * (minMax_max - minMax_min) + minMax_min`.
    MinMax { min: f64, max: f64 },
}

/// `uiDetectFeatureOutliers`: values beyond the robust mean +/- `outlierIdentCutoff` robust
/// standard deviations are coerced to that bound (`coerceToCutoffVal`) or become missing
/// (`setToMissingVal`).
struct Outliers {
    cutoff: f64,
    to_missing: bool,
}

enum Class {
    Pass,
    UnsupportedTime,
    Date,
    Continuous,
    Categorical,
}

struct OutCol {
    field: Field,
    values: Vec<Value>,
    group: u8,
    key_a: u32,
    key_b: usize,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    Ok(Box::new(parse(node.settings.as_ref())?))
}

impl Operator for Prep {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let Some(input) = inputs.first().filter(|_| inputs.len() == 1) else {
            return Err(OperatorError::Inputs(
                "Auto Data Prep needs one input".into(),
            ));
        };
        let n_fields = input.model().len();
        let n_rows = input.row_count();
        if n_fields > MAX_FIELDS || n_rows > MAX_ROWS || n_fields.saturating_mul(n_rows) > MAX_CELLS
        {
            return Err(OperatorError::Data(format!(
                "Auto Data Prep input is too large ({n_fields} fields, {n_rows} rows)"
            )));
        }
        // No rows: there is nothing to analyse. Copy the input rather than invent a schema.
        if n_rows == 0 {
            return Ok(input.as_ref().clone());
        }
        let mut cols = Vec::new();
        for (index, field) in input.model().fields().iter().enumerate() {
            let values = column(input, index);
            match classify(self, field) {
                Class::UnsupportedTime => {
                    return Err(OperatorError::Settings(format!(
                        "uiComputeTimeDuration: time field {:?} is not supported yet",
                        field.name
                    )));
                }
                Class::Pass => cols.push(pass_col(field, values, index)),
                Class::Date => prepare_date(self, field, index, &values, &mut cols)?,
                Class::Continuous if self.prepare_features => {
                    prepare_continuous(self, field, index, &values, &mut cols)?;
                }
                Class::Categorical if self.prepare_features => {
                    prepare_categorical(self, field, index, &values, &mut cols)?;
                }
                Class::Continuous | Class::Categorical => {
                    cols.push(pass_col(field, values, index));
                }
            }
        }
        assemble(n_rows, cols)
    }
}

fn parse(s: Option<&Element>) -> Result<Prep, OperatorError> {
    reject_unknown(s)?;
    reject_unsupported(s)?;
    acknowledge_unused(s)?;
    let date_unit = match text_or(s, "dateUnit", "month").as_str() {
        "month" => ("suffixElapsedMonths", "_months", 30.4375),
        "day" => ("suffixElapsedDays", "_days", 1.0),
        "year" => {
            return Err(OperatorError::Settings(
                "dateUnit: year is not supported yet".into(),
            ));
        }
        other => {
            return Err(OperatorError::Settings(format!(
                "dateUnit: unsupported value {other:?}"
            )));
        }
    };
    Ok(Prep {
        exclude_high_missing: flag(
            s,
            "uiExcludeFeatureIfHasHigherPercentageOfMissingValue",
            true,
        )?,
        max_missing_pct: percent_int(s, "maxNumberOfMissingValue", 50)? as f64,
        exclude_many_cats: flag(s, "uiExcludeFeatureIfHasManyUniqueCatogories", true)?,
        max_unique: nonneg_int(s, "maxNOfUniqueCategories", 100)?,
        exclude_poor: flag(s, "excludePoorQualityFeatures", true)?,
        reorder_nominal: flag(s, "uiReorderNominalCatsPriorToMerging", true)?,
        replace_scale_missing: flag(s, "uiReplaceScaleFeaturesMissingValue", true)?,
        scale_continuous: flag(s, "uiScaleContinuousFeatures", true)?,
        scale: match one_of(
            s,
            "scaleContinuousMode",
            &["zScoreTransform", "minMaxTransform"],
            "zScoreTransform",
        )?
        .as_str()
        {
            "minMaxTransform" => {
                let (min, max) = (
                    finite(s, "minMax_min", 0.0)?,
                    finite(s, "minMax_max", 100.0)?,
                );
                if min >= max {
                    return Err(OperatorError::Settings(
                        "minMax_min must be less than minMax_max".into(),
                    ));
                }
                Scale::MinMax { min, max }
            }
            _ => Scale::ZScore {
                mean: finite(s, "zScore_mean", 0.0)?,
                sd: positive(s, "zScore_SD", 1.0)?,
            },
        },
        outliers: if flag(s, "uiDetectFeatureOutliers", false)? {
            Some(Outliers {
                cutoff: positive(s, "outlierIdentCutoff", 3.0)?,
                to_missing: one_of(
                    s,
                    "outliersOperation",
                    &["coerceToCutoffVal", "setToMissingVal"],
                    "coerceToCutoffVal",
                )? == "setToMissingVal",
            })
        } else {
            None
        },
        check_feature_level: flag(s, "uiCheckFeatureMeasureLevel", true)?,
        ordinal_to_scale: nonneg_int(s, "ordinalToScaleCategoriesNumThreshold", 10)?,
        scale_to_ordinal: nonneg_int(s, "scaleToOrdinalValuesNumThreshold", 5)?,
        prepare_dates: flag(s, "prepareDatesAndTimes", true)?,
        compute_date_duration: flag(s, "uiComputeDateDuration", true)?,
        date_divisor: date_unit.2,
        date_suffix: text_or(s, date_unit.0, date_unit.1),
        ref_days: reference_days(s)?,
        extract_year: flag(s, "uiExtractYearFromDate", false)?,
        extract_month: flag(s, "uiExtractMonthFromDate", false)?,
        extract_day: flag(s, "uiExtractDayFromDate", false)?,
        suffix_year: text_or(s, "suffixYear", "_year"),
        suffix_month: text_or(s, "suffixMonth", "_month"),
        suffix_day: text_or(s, "suffixDay", "_day"),
        name_ext: text_or(s, "nameExtForInputs", "_transformed"),
        trim_blanks: flag(s, "trimBlanks", true)?,
        prepare_features: flag(s, "prepareFeatures", true)?,
        compute_time_duration: flag(s, "uiComputeTimeDuration", true)?,
    })
}

fn reject_unknown(s: Option<&Element>) -> Result<(), OperatorError> {
    for child in s.into_iter().flat_map(Element::child_elements) {
        let name = child.name.local.as_ref();
        if METADATA.contains(&name) || KNOWN.contains(&name) {
            continue;
        }
        return Err(OperatorError::Settings(format!(
            "unsupported setting {name}"
        )));
    }
    Ok(())
}

fn reject_unsupported(s: Option<&Element>) -> Result<(), OperatorError> {
    for key in [
        "uiReorderNominalTargetCategories",
        "uiReplaceScaleTargetMissingValue",
        "uiReplaceOrdinalTargetMissingValue",
        "uiReplaceNominalTargetMissingValue",
        "uiRescaleScaleTargetUsingBoxCox",
        "uiDetectTargetOutliers",
        "uiExtractHourFromTime",
        "uiExtractMinuteFromTime",
        "uiExtractSecondFromTime",
        "uiUnsupervisedMergeNominal",
        "uiUnsupervisedMergeOrdinal",
        "uiExcludeAlmostConstantCatFeature",
        "uiFeatureConstruction",
        "useWeight",
        "useFrequency",
        "useCustomSettings",
        "uiReplaceOrdinalFeaturesMissingValue",
    ] {
        if flag(s, key, false)? {
            return Err(OperatorError::Settings(format!(
                "{key} is not supported yet"
            )));
        }
    }
    if !flag(s, "uiReplaceNominalFeaturesMissingValue", true)? {
        return Err(OperatorError::Settings(
            "uiReplaceNominalFeaturesMissingValue is not supported yet".into(),
        ));
    }
    for key in ["targetField", "frequencyWeight", "analysisWeight"] {
        if settings::text(s, key).is_some_and(|value| !value.trim().is_empty()) {
            return Err(OperatorError::Settings(format!(
                "{key} is not supported yet"
            )));
        }
    }
    if settings::element(s, "inputFields").is_some_and(|el| el.child_elements().next().is_some()) {
        return Err(OperatorError::Settings(
            "inputFields is not supported yet".into(),
        ));
    }
    one_of(s, "excludeNoUsedField", &["filterOut"], "filterOut")?;
    one_of(s, "keepOrClearAnalysis", &["stop"], "stop")?;
    one_of(s, "autoChooseDateFieldUnit", &["fixed"], "fixed")?;
    one_of(s, "referenceDateType", &["fixedDate"], "fixedDate")?;
    one_of(s, "autoChooseTimeFieldUnit", &["fixed"], "fixed")?;
    one_of(s, "referenceTimeType", &["fixedTime"], "fixedTime")?;
    one_of(s, "timeUnit", &["hour"], "hour")?;
    let objective = text_or(s, "analysisObjective", "balance");
    if !matches!(
        objective.as_str(),
        "balance" | "Balanced" | "speed" | "Speed" | "accuracy" | "Accuracy" | "custom" | "Custom"
    ) {
        return Err(OperatorError::Settings(format!(
            "analysisObjective: unsupported value {objective:?}"
        )));
    }
    Ok(())
}

/// Settings the three cases save that do not change the exported table. Malformed numbers still
/// fail. `GAP:` the supervised steps are named in the module docs.
fn acknowledge_unused(s: Option<&Element>) -> Result<(), OperatorError> {
    let _ = flag(s, "uiSupervisedMerging", true)?;
    let _ = flag(s, "uiFeatureSelection", true)?;
    let _ = flag(s, "uiSupervisedBinning", true)?;
    let _ = flag(s, "transConsSelectFeatures", true)?;
    let _ = flag(s, "uiCheckTargetMeasureLevel", true)?;
    let _ = flag(s, "modelExist", false)?;
    let _ = flag(s, "clearAnalysis", false)?;
    let _ = unit_interval(s, "supervisedMergingPValue", 0.05)?;
    let _ = unit_interval(s, "featureSelectionPValue", 0.05)?;
    let _ = unit_interval(s, "homogeneousSubsetsPValue", 0.05)?;
    let _ = finite(s, "targetMean", 0.0)?;
    let _ = positive(s, "targetSD", 1.0)?;
    let min_cases = int_or(s, "unsupervisedMergingMinNumberOfCases", 10)?;
    if min_cases < 1 {
        return Err(OperatorError::Settings(format!(
            "unsupervisedMergingMinNumberOfCases: expected at least 1, found {min_cases}"
        )));
    }
    let _ = percent_int(s, "cateFeatureScreenNumber", 95)?;
    let _ = one_of(
        s,
        "outliersOperation",
        &["coerceToCutoffVal", "setToMissingVal"],
        "coerceToCutoffVal",
    )?;
    let _ = positive(s, "outlierIdentCutoff", 3.0)?;
    let _ = finite(s, "minMax_min", 0.0)?;
    let _ = finite(s, "minMax_max", 100.0)?;
    let _ = settings::integer(s, "todaysDate")?;
    let _ = settings::integer(s, "currentTime")?;
    let _ = settings::integer(s, "fixedTime")?;
    let _ = text_or(s, "rootNameForConsturctedFeature", "feature");
    let _ = text_or(s, "nameExtForTarget", "_transformed");
    let _ = text_or(s, "suffixElapsedSeconds", "_seconds");
    let _ = text_or(s, "suffixElapsedMinutes", "_minutes");
    let _ = text_or(s, "suffixElapsedHours", "_hours");
    let _ = text_or(s, "suffixElapsedYears", "_years");
    let _ = text_or(s, "suffixHour", "_hour");
    let _ = text_or(s, "suffixMinute", "_minute");
    let _ = text_or(s, "suffixSecond", "_second");
    // The duration suffix that this unit does not use is still a saved string.
    let _ = text_or(s, "suffixElapsedMonths", "_months");
    let _ = text_or(s, "suffixElapsedDays", "_days");
    Ok(())
}

fn classify(prep: &Prep, field: &Field) -> Class {
    if field.storage == Storage::Time && prep.compute_time_duration {
        return Class::UnsupportedTime;
    }
    if matches!(
        field.storage,
        Storage::Time | Storage::Timestamp | Storage::List | Storage::Unknown
    ) {
        return Class::Pass;
    }
    if field.role != Role::Input {
        return Class::Pass;
    }
    if matches!(
        field.measurement,
        Measurement::Typeless | Measurement::Geospatial | Measurement::Collection
    ) {
        return Class::Pass;
    }
    if field.storage == Storage::Date {
        return if prep.prepare_dates {
            Class::Date
        } else {
            Class::Pass
        };
    }
    // GAP: original ordinal inputs are not recoded.
    if field.measurement == Measurement::Ordinal {
        return Class::Pass;
    }
    if is_nominal_like(field) {
        return Class::Categorical;
    }
    if matches!(field.storage, Storage::Integer | Storage::Real) {
        return Class::Continuous;
    }
    Class::Pass
}

fn is_nominal_like(field: &Field) -> bool {
    match field.measurement {
        Measurement::Nominal | Measurement::Flag | Measurement::Categorical => true,
        Measurement::Continuous | Measurement::Default => field.storage == Storage::String,
        _ => false,
    }
}

fn prepare_date(
    prep: &Prep,
    field: &Field,
    index: usize,
    values: &[Value],
    cols: &mut Vec<OutCol>,
) -> Result<(), OperatorError> {
    let days: Vec<Option<f64>> = values
        .iter()
        .map(|value| as_date_days(field, value))
        .collect();
    if screen_numeric(prep, &days) {
        return Ok(());
    }
    let mut years = Vec::with_capacity(days.len());
    let mut months = Vec::with_capacity(days.len());
    let mut day_of_month = Vec::with_capacity(days.len());
    for day in &days {
        match day.and_then(ymd) {
            Some((year, month, day_n)) => {
                years.push(Some(year));
                months.push(Some(month));
                day_of_month.push(Some(day_n));
            }
            None => {
                years.push(None);
                months.push(None);
                day_of_month.push(None);
            }
        }
    }
    let mut produced = false;
    if prep.extract_year {
        push_element(prep, field, index, 1, &prep.suffix_year, &years, cols)?;
        produced = true;
    }
    if prep.extract_month {
        push_element(prep, field, index, 2, &prep.suffix_month, &months, cols)?;
        produced = true;
    }
    if prep.extract_day {
        push_element(prep, field, index, 3, &prep.suffix_day, &day_of_month, cols)?;
        produced = true;
    }
    if prep.compute_date_duration {
        if prep.date_divisor == 0.0 {
            return Err(OperatorError::Settings(
                "dateUnit: duration divisor is 0".into(),
            ));
        }
        let series: Vec<Option<f64>> = days
            .iter()
            .map(|day| day.map(|value| (value - prep.ref_days) / prep.date_divisor))
            .collect();
        let name = extended(&field.name, &prep.date_suffix)?;
        let out_values = if prep.scale_continuous {
            rescale(&series, date_scale(prep)?, prep.replace_scale_missing)
        } else {
            optional_reals(&series)
        };
        cols.push(OutCol {
            field: continuous_field(name),
            values: out_values,
            group: GROUP_DATE_SCALE,
            key_a: as_u32(index)?,
            key_b: 0,
        });
        produced = true;
    }
    if !produced {
        cols.push(pass_col(field, values.to_vec(), index));
    }
    Ok(())
}

fn push_element(
    prep: &Prep,
    field: &Field,
    field_index: usize,
    extract_index: usize,
    suffix: &str,
    components: &[Option<i64>],
    cols: &mut Vec<OutCol>,
) -> Result<(), OperatorError> {
    let name = extended(&field.name, suffix)?;
    let distinct = distinct_i64(components);
    let recast = prep.check_feature_level && exceeds(distinct, prep.ordinal_to_scale);
    let key_index = as_u32(field_index)?;
    if recast {
        let series: Vec<Option<f64>> = components
            .iter()
            .map(|value| value.map(|number| number as f64))
            .collect();
        let values = if prep.scale_continuous {
            rescale(&series, date_scale(prep)?, prep.replace_scale_missing)
        } else {
            optional_reals(&series)
        };
        cols.push(OutCol {
            field: continuous_field(name),
            values,
            group: GROUP_DATE_SCALE,
            key_a: key_index,
            key_b: extract_index,
        });
    } else {
        cols.push(OutCol {
            field: ordinal_field(name),
            values: components
                .iter()
                .map(|value| match value {
                    Some(number) => Value::Integer(*number),
                    None => Value::Null,
                })
                .collect(),
            group: GROUP_DATE_ORDINAL,
            key_a: key_index,
            key_b: extract_index,
        });
    }
    Ok(())
}

fn prepare_continuous(
    prep: &Prep,
    field: &Field,
    index: usize,
    values: &[Value],
    cols: &mut Vec<OutCol>,
) -> Result<(), OperatorError> {
    let series: Vec<Option<f64>> = values.iter().map(|value| as_number(field, value)).collect();
    if screen_numeric(prep, &series) {
        return Ok(());
    }
    let distinct = distinct_f64(&series);
    if prep.check_feature_level && below(distinct, prep.scale_to_ordinal) {
        let mut kept = field.clone();
        kept.measurement = Measurement::Ordinal;
        cols.push(OutCol {
            field: kept,
            values: values.to_vec(),
            group: GROUP_PASS,
            key_a: 0,
            key_b: index,
        });
        return Ok(());
    }
    if !prep.scale_continuous {
        cols.push(pass_col(field, values.to_vec(), index));
        return Ok(());
    }
    let series = match &prep.outliers {
        Some(rule) => handle_outliers(rule, series, &field.name)?,
        None => series,
    };
    let missing = series.iter().filter(|value| value.is_none()).count();
    let name = extended(&field.name, &prep.name_ext)?;
    cols.push(OutCol {
        field: continuous_field(name),
        values: rescale(&series, &prep.scale, prep.replace_scale_missing),
        group: GROUP_CONTINUOUS,
        key_a: as_u32(missing)?,
        key_b: index,
    });
    Ok(())
}

fn prepare_categorical(
    prep: &Prep,
    field: &Field,
    index: usize,
    values: &[Value],
    cols: &mut Vec<OutCol>,
) -> Result<(), OperatorError> {
    let mut counts: Vec<(Value, usize)> = Vec::new();
    let mut non_null = 0usize;
    for value in values {
        let key = category_key(field, value, prep.trim_blanks);
        if let Some(slot) = counts
            .iter_mut()
            .find(|(seen, _)| seen.total_cmp(&key) == Ordering::Equal)
        {
            slot.1 = slot.1.saturating_add(1);
            continue;
        }
        if !key.is_null() {
            non_null = non_null.saturating_add(1);
            if prep.exclude_many_cats && exceeds(non_null, prep.max_unique) {
                return Ok(());
            }
        }
        if counts.len() >= MAX_CATEGORIES {
            return Err(OperatorError::Data(format!(
                "field {:?} has more than {MAX_CATEGORIES} categories",
                field.name
            )));
        }
        counts.push((key, 1));
    }
    let missing = counts
        .iter()
        .find(|(key, _)| key.is_null())
        .map_or(0, |(_, count)| *count);
    let n = values.len();
    if n > 0 && prep.exclude_high_missing && percent(missing, n) > prep.max_missing_pct {
        return Ok(());
    }
    if prep.exclude_poor && non_null <= 1 {
        return Ok(());
    }
    if !prep.reorder_nominal {
        cols.push(pass_col(field, values.to_vec(), index));
        return Ok(());
    }
    let labeled = assign_codes(counts);
    let name = extended(&field.name, &prep.name_ext)?;
    let out_values = values
        .iter()
        .map(|value| {
            let key = category_key(field, value, prep.trim_blanks);
            lookup(&labeled, &key)
        })
        .collect();
    cols.push(OutCol {
        field: nominal_field(name),
        values: out_values,
        group: GROUP_CATEGORICAL,
        key_a: 0,
        key_b: index,
    });
    Ok(())
}

fn screen_numeric(prep: &Prep, values: &[Option<f64>]) -> bool {
    let n = values.len();
    if n == 0 {
        return false;
    }
    let missing = values.iter().filter(|value| value.is_none()).count();
    if prep.exclude_high_missing && percent(missing, n) > prep.max_missing_pct {
        return true;
    }
    prep.exclude_poor && distinct_f64(values) <= 1
}

/// Date durations and recast date elements have z-score evidence only (`misc_next/adp_*`).
fn date_scale(prep: &Prep) -> Result<&Scale, OperatorError> {
    match prep.scale {
        Scale::ZScore { .. } => Ok(&prep.scale),
        Scale::MinMax { .. } => Err(OperatorError::Settings(
            "GAP: min-max rescaling of date fields has no oracle".into(),
        )),
    }
}

/// Outliers of one continuous input (`adp_next/adp_outlier_*`): the bounds are the robust mean
/// +/- `cutoff` robust standard deviations ([`dms_algo::adp::robust_moments`] from the sample
/// mean and standard deviation of the finite values). `c_out`'s 60 and -45 become the bounds
/// 0.00503 +/- 3 x 2.92449 with `coerceToCutoffVal`, and missing with `setToMissingVal`; the
/// rescaling then sees the handled values (and counts the new missing values for field order).
fn handle_outliers(
    rule: &Outliers,
    series: Vec<Option<f64>>,
    name: &str,
) -> Result<Vec<Option<f64>>, OperatorError> {
    let finite: Vec<f64> = series
        .iter()
        .flatten()
        .copied()
        .filter(|x| x.is_finite())
        .collect();
    if finite.len() < 2 {
        return Ok(series);
    }
    let n = finite.len() as f64;
    let mean = finite.iter().sum::<f64>() / n;
    let sd = (finite.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
    if !(sd.is_finite() && sd > 0.0) {
        return Ok(series);
    }
    let (robust_mean, robust_sd) = dms_algo::adp::robust_moments(&finite, mean, sd)
        .map_err(|error| OperatorError::Data(format!("field {name:?}: {error}")))?;
    let (low, high) = (
        robust_mean - rule.cutoff * robust_sd,
        robust_mean + rule.cutoff * robust_sd,
    );
    Ok(series
        .into_iter()
        .map(|value| match value {
            Some(x) if x < low => (!rule.to_missing).then_some(low),
            Some(x) if x > high => (!rule.to_missing).then_some(high),
            other => other,
        })
        .collect())
}

/// Rescales a continuous series. Z-score: sample standard deviation; with imputation the divisor
/// is `n_all - 1` (missing rows sit at the mean), without it `n_valid - 1` and missing rows stay
/// null. Min-max (`adp_next/adp_minmax`): over the finite values' range; an imputed row takes
/// the mean's position.
fn rescale(values: &[Option<f64>], scale: &Scale, impute: bool) -> Vec<Value> {
    let (final_mean, final_sd) = match *scale {
        Scale::ZScore { mean, sd } => (mean, sd),
        Scale::MinMax { min, max } => return min_max(values, min, max, impute),
    };
    let n_all = values.len();
    let mut n_valid = 0usize;
    let mut sum = 0.0;
    for value in values.iter().flatten().copied() {
        if value.is_finite() {
            n_valid = n_valid.saturating_add(1);
            sum += value;
        }
    }
    if n_valid == 0 || !sum.is_finite() {
        return vec![Value::Null; n_all];
    }
    let mean = sum / (n_valid as f64);
    if !mean.is_finite() {
        return vec![Value::Null; n_all];
    }
    let mut ss = 0.0;
    for value in values.iter().flatten().copied() {
        if value.is_finite() {
            let delta = value - mean;
            ss += delta * delta;
        }
    }
    let denom = if impute {
        n_all.saturating_sub(1)
    } else {
        n_valid.saturating_sub(1)
    };
    let sd = if denom == 0 || ss <= 0.0 {
        0.0
    } else {
        (ss / (denom as f64)).sqrt()
    };
    let usable = sd.is_finite() && sd > 0.0;
    values
        .iter()
        .map(|value| match value {
            Some(x) if x.is_finite() && usable => {
                finite_real((x - mean) / sd * final_sd + final_mean)
            }
            Some(x) if x.is_finite() => finite_real(final_mean),
            _ if impute => finite_real(final_mean),
            _ => Value::Null,
        })
        .collect()
}

fn min_max(values: &[Option<f64>], low: f64, high: f64, impute: bool) -> Vec<Value> {
    let finite: Vec<f64> = values
        .iter()
        .flatten()
        .copied()
        .filter(|x| x.is_finite())
        .collect();
    let (Some(min), Some(max)) = (
        finite.iter().copied().reduce(f64::min),
        finite.iter().copied().reduce(f64::max),
    ) else {
        return vec![Value::Null; values.len()];
    };
    let mean = finite.iter().sum::<f64>() / finite.len() as f64;
    let span = max - min;
    let scaled = |x: f64| {
        if span > 0.0 {
            finite_real((x - min) / span * (high - low) + low)
        } else {
            finite_real(low)
        }
    };
    values
        .iter()
        .map(|value| match value {
            Some(x) if x.is_finite() => scaled(*x),
            _ if impute => scaled(mean),
            _ => Value::Null,
        })
        .collect()
}

fn assign_codes(mut counts: Vec<(Value, usize)>) -> Vec<(Value, Value)> {
    counts.sort_by(|left, right| {
        left.1
            .cmp(&right.1)
            .then_with(|| left.0.total_cmp(&right.0))
    });
    let mut labeled: Vec<(Value, Value)> = counts
        .into_iter()
        .enumerate()
        .map(|(index, (key, _))| (key, Value::string(index.to_string())))
        .collect();
    labeled.sort_by(|left, right| left.0.total_cmp(&right.0));
    labeled
}

fn lookup(labeled: &[(Value, Value)], key: &Value) -> Value {
    if let Ok(index) = labeled.binary_search_by(|probe| probe.0.total_cmp(key))
        && let Some(pair) = labeled.get(index)
    {
        return pair.1.clone();
    }
    Value::Null
}

fn category_key(field: &Field, value: &Value, trim: bool) -> Value {
    if field.is_missing(value) {
        return Value::Null;
    }
    match value {
        Value::String(text) if trim => Value::string(text.trim()),
        other => other.clone(),
    }
}

fn as_number(field: &Field, value: &Value) -> Option<f64> {
    if field.is_missing(value) {
        return None;
    }
    value.as_f64().filter(|number| number.is_finite())
}

fn as_date_days(field: &Field, value: &Value) -> Option<f64> {
    if field.is_missing(value) {
        return None;
    }
    match value {
        Value::Date(days) => Some(*days as f64),
        _ => None,
    }
}

fn ymd(days: f64) -> Option<(i64, i64, i64)> {
    if !days.is_finite() {
        return None;
    }
    let rounded = days.round();
    if (days - rounded).abs() > 1e-6 {
        return None;
    }
    let day = rounded as i64;
    let (year, month, day_of_month) = civil_from_days(day)?;
    Some((year, i64::from(month), i64::from(day_of_month)))
}

fn distinct_f64(values: &[Option<f64>]) -> usize {
    let mut nums: Vec<f64> = values
        .iter()
        .flatten()
        .copied()
        .filter(|v| v.is_finite())
        .collect();
    nums.sort_by(f64::total_cmp);
    nums.dedup_by(|left, right| *left == *right);
    nums.len()
}

fn distinct_i64(values: &[Option<i64>]) -> usize {
    let mut nums: Vec<i64> = values.iter().flatten().copied().collect();
    nums.sort_unstable();
    nums.dedup();
    nums.len()
}

fn percent(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        (part as f64) * 100.0 / (whole as f64)
    }
}

fn exceeds(distinct: usize, max: i64) -> bool {
    match i64::try_from(distinct) {
        Ok(value) => value > max,
        Err(_) => true,
    }
}

fn below(distinct: usize, threshold: i64) -> bool {
    match i64::try_from(distinct) {
        Ok(value) => value < threshold,
        Err(_) => false,
    }
}

fn optional_reals(values: &[Option<f64>]) -> Vec<Value> {
    values
        .iter()
        .map(|value| match value {
            Some(number) => finite_real(*number),
            None => Value::Null,
        })
        .collect()
}

fn finite_real(value: f64) -> Value {
    if value.is_finite() {
        Value::Real(value)
    } else {
        Value::Null
    }
}

fn column(table: &Table, index: usize) -> Vec<Value> {
    table
        .rows()
        .iter()
        .map(|row| row.get(index).cloned().unwrap_or(Value::Null))
        .collect()
}

fn pass_col(field: &Field, values: Vec<Value>, index: usize) -> OutCol {
    OutCol {
        field: field.clone(),
        values,
        group: GROUP_PASS,
        key_a: 0,
        key_b: index,
    }
}

fn assemble(n: usize, mut cols: Vec<OutCol>) -> Result<Table, OperatorError> {
    cols.sort_by(|left, right| {
        left.group
            .cmp(&right.group)
            .then(left.key_a.cmp(&right.key_a))
            .then(left.key_b.cmp(&right.key_b))
    });
    for col in &cols {
        if col.values.len() != n {
            return Err(OperatorError::Data(format!(
                "field {:?} has {} values for {n} rows",
                col.field.name,
                col.values.len()
            )));
        }
    }
    let fields = cols.iter().map(|col| col.field.clone()).collect();
    let model = DataModel::new(fields)
        .map_err(|error| OperatorError::Data(format!("Auto Data Prep: {error}")))?;
    let mut rows = Vec::with_capacity(n);
    for row_index in 0..n {
        let mut row = Vec::with_capacity(cols.len());
        for col in &cols {
            row.push(col.values.get(row_index).cloned().unwrap_or(Value::Null));
        }
        rows.push(row);
    }
    Table::new(model, rows).map_err(|error| OperatorError::Data(format!("Auto Data Prep: {error}")))
}

fn extended(base: &str, extension: &str) -> Result<String, OperatorError> {
    if base.is_empty() && extension.is_empty() {
        return Err(OperatorError::Data(
            "Auto Data Prep derived an empty field name".into(),
        ));
    }
    Ok(format!("{base}{extension}"))
}

fn as_u32(value: usize) -> Result<u32, OperatorError> {
    u32::try_from(value).map_err(|_| OperatorError::Data("Auto Data Prep index overflow".into()))
}

fn continuous_field(name: String) -> Field {
    let mut field = Field::new(name, Storage::Real);
    field.measurement = Measurement::Continuous;
    field.role = Role::Input;
    field
}

fn ordinal_field(name: String) -> Field {
    let mut field = Field::new(name, Storage::Integer);
    field.measurement = Measurement::Ordinal;
    field.role = Role::Input;
    field
}

fn nominal_field(name: String) -> Field {
    let mut field = Field::new(name, Storage::String);
    field.measurement = Measurement::Nominal;
    field.role = Role::Input;
    field
}

fn reference_days(s: Option<&Element>) -> Result<f64, OperatorError> {
    let raw = settings::integer(s, "fixedDate")?.unwrap_or(1_792_278_336_753);
    let days = epoch_days(raw);
    if days.is_finite() {
        Ok(days)
    } else {
        Err(OperatorError::Settings(
            "fixedDate: cannot convert to a day number".into(),
        ))
    }
}

fn epoch_days(raw: i64) -> f64 {
    let abs = raw.unsigned_abs();
    if abs > 1_000_000_000_000 {
        (raw as f64) / 86_400_000.0
    } else if abs > 1_000_000_000 {
        (raw as f64) / 86_400.0
    } else {
        raw as f64
    }
}

fn flag(s: Option<&Element>, key: &str, default: bool) -> Result<bool, OperatorError> {
    Ok(settings::flag(s, key)?.unwrap_or(default))
}

fn text_or(s: Option<&Element>, key: &str, default: &str) -> String {
    settings::text(s, key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn int_or(s: Option<&Element>, key: &str, default: i64) -> Result<i64, OperatorError> {
    Ok(settings::integer(s, key)?.unwrap_or(default))
}

fn finite(s: Option<&Element>, key: &str, default: f64) -> Result<f64, OperatorError> {
    Ok(settings::real(s, key)?.unwrap_or(default))
}

fn positive(s: Option<&Element>, key: &str, default: f64) -> Result<f64, OperatorError> {
    let value = finite(s, key, default)?;
    if value > 0.0 {
        Ok(value)
    } else {
        Err(OperatorError::Settings(format!(
            "{key}: expected a positive number, found {value}"
        )))
    }
}

fn unit_interval(s: Option<&Element>, key: &str, default: f64) -> Result<f64, OperatorError> {
    let value = finite(s, key, default)?;
    if (0.0..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err(OperatorError::Settings(format!(
            "{key}: expected a probability from 0 to 1, found {value}"
        )))
    }
}

fn nonneg_int(s: Option<&Element>, key: &str, default: i64) -> Result<i64, OperatorError> {
    let value = int_or(s, key, default)?;
    if value >= 0 {
        Ok(value)
    } else {
        Err(OperatorError::Settings(format!(
            "{key}: expected a non-negative integer, found {value}"
        )))
    }
}

fn percent_int(s: Option<&Element>, key: &str, default: i64) -> Result<i64, OperatorError> {
    let value = int_or(s, key, default)?;
    if (0..=100).contains(&value) {
        Ok(value)
    } else {
        Err(OperatorError::Settings(format!(
            "{key}: expected a percentage from 0 to 100, found {value}"
        )))
    }
}

fn one_of(
    s: Option<&Element>,
    key: &str,
    allowed: &[&str],
    default: &str,
) -> Result<String, OperatorError> {
    let value = text_or(s, key, default);
    if allowed.contains(&value.as_str()) {
        Ok(value)
    } else {
        Err(OperatorError::Settings(format!(
            "{key}: unsupported value {value:?}"
        )))
    }
}
