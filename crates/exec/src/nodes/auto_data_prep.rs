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
//! Min-max (`scaleContinuousMode` `minMaxTransform`, `adp_next/adp_minmax`,
//! `adp_rest/rest_dates_minmax`): `(x - min) /
//! (max - min) * (minMax_max - minMax_min) + minMax_min` over the finite values, date durations
//! and recast date elements included (`d_days`/`t_hours` to 0..100).
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
//! Categories (`adp_balanced` `grp`, `flg`; `adp_rest/rest_base` `os`): non-missing values and system-null are counted.
//! Codes are strings `"0"`, `"1"`, ... from the least frequent category; equal counts break by
//! [`Value::total_cmp`] (null before any other value). Codes are zero-padded to the width of
//! the largest code (`many80` 80 categories → `"00"`..`"79"`). `grp` empty string is a real category
//! (`""` once → `"0"`). `flg` null once → `"0"`, `F` → `"1"`, `T` → `"2"`. A string ordinal
//! input is a categorical to recode (`os`: missing 12 → `"0"`, `s1` → `"1"`, `s2` → `"2"`,
//! `s3` → `"3"`, `s4` → `"4"`). The new field is a
//! nominal string named with `nameExtForInputs` (`_transformed`). The target `y` is not recoded.
//!
//! Screening (saved on in the three cases; no corpus field crosses a threshold): an input is
//! dropped when its missing share is greater than `maxNumberOfMissingValue` (50), when a
//! categorical input has more than `maxNOfUniqueCategories` (100) non-missing categories, when
//! `uiExcludeAlmostConstantCatFeature` sees its mode exceed `cateFeatureScreenNumber` percent
//! of all rows (`adp_rest/rest_almost_constant`: `acat` 90% `A` > 80 drops), or
//! when `excludePoorQualityFeatures` sees fewer than two distinct non-missing values. Role-none
//! and typeless fields are kept (`id`, `txt`). `excludeNoUsedField` `filterOut` drops inputs
//! that were replaced by a transformed field.
//!
//! `analysisObjective` `balance` (`adp_balanced`) and `custom` (`adp_rescale`, `adp_dates`) do
//! not change the math; the saved flags do. `adp_balanced` and `adp_rescale` export the same
//! table.
//!
//! Custom fields (`useCustomSettings` with `targetField`/`inputFields/field`,
//! `rest_custom_fields` keeps only `c_rel`, `rest_weight_control` repeats the Type roles) select
//! the target and inputs; other Type inputs pass through raw. Frequency weights (`useFrequency`
//! with `frequencyWeight`, `rest_freq_weight` `wfreq`) leave the weight field unchanged and have
//! no effect on rescaling/recoding without supervised steps (matches the control to every
//! printed decimal); weighted supervised steps have no oracle.
//!
//!
//! Merging (objective `custom` with `transConsSelectFeatures`, see `custom_steps`): with a
//! target, `uiSupervisedMerging` screens and merges categorical inputs (`supervised_merge`),
//! whether or not `uiReorderNominalCatsPriorToMerging` is on (`rest_nominal_noreorder` `n1`
//! merges `k1`/`k2` all the same); without one, `uiUnsupervisedMergeNominal` merges sparse
//! categories (`unsupervised_merge`). Under objective `balance` the saved flags do not apply
//! (`adp_balanced`).
//!
//! Feature selection (`uiFeatureSelection`, objective `custom`): continuous inputs whose
//! correlation p-value exceeds `featureSelectionPValue` are dropped for a continuous target
//! (`correlation_p`); with a categorical target continuous inputs are kept
//! (`rest_selection_cattarget` `c_rel`/`c_noise`); categorical inputs are screened as under
//! supervised merging.
//!
//! Supervised binning (`uiSupervisedBinning`, objective `custom`, categorical target): continuous
//! inputs become ordinal bins of homogeneous subsets (`homogeneous_cuts`); missing values are
//! replaced by the valid mean before the cuts when `uiReplaceScaleFeaturesMissingValue` is on
//! (`rest_cont_missing_binning` `c_miss` cut `-0.329…`).
//!
//! Feature construction (`uiFeatureConstruction`, objective `custom`): correlated continuous
//! inputs are replaced by principal components (`construct_features`).
//!
//! `GAP:` feature selection with date fields.
//! `GAP:` `uiReplaceNominalFeaturesMissingValue` true does not substitute the mode (see `flg`).
//! false is rejected. Timestamps are copied (`misc_next/adp_*`, `adp_time/*`).
//!
//! Times and date units (`adp_time/*`, `adp_rest/rest_time_both`): a time-of-day input gets a
//! signed duration from `fixedTime` and hour/minute/second elements (`prepare_time` keeps both
//! the duration and the three recast z-scored elements); date durations also take years,
//! and automatic units follow the largest elapsed value (`choose_unit`). A date or time input
//! with no duration and no element is excluded; when every input is excluded the node fails as
//! Modeler does.
//!
//! Target (`uiRescaleScaleTargetUsingBoxCox`): a continuous target is Box-Cox transformed and
//! rescaled to `targetMean` / `targetSD`, see `box_cox_target_with`. Outliers are handled first
//! (`rest_target_outliers_delete` Delete → missing, `rest_target_combined` Replace → bounds),
//! then missing values become the final mean when `uiReplaceScaleTargetMissingValue` is on
//! (`ycomb` 9 missing → 0, divisor `n_all - 1` like rescaling with imputation).
//!
//! Ordinal inputs (`prepare_ordinal`) and target steps (`prepare_target`) follow
//! `adp_more/*`.
//!
//! `GAP:` ordinal targets,
//! frequency weights with supervised steps, analysis weights (Modeler rejects `use_weight`,
//! kept rejected; no recorded streams), a
//! reference of the current date or time,
//! and `excludeNoUsedField` other than `filterOut`
//! are rejected. `GAP:` a zero-row input is copied
//! unchanged.

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
/// Continuous inputs binned against a categorical target follow the categorical fields
/// (`adp_next/adp_binning`).
const GROUP_BINNED: u8 = 6;
/// Constructed features go last, after categorical/date fields
/// (`rest_construct_order` `feature1` after `ncat_transformed`).
const GROUP_CONSTRUCTED: u8 = 7;

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
    exclude_almost_constant: bool,
    almost_constant_pct: f64,
    /// `useCustomSettings`: explicit target and inputs (`targetField`, `inputFields/field`).
    /// `None` means the upstream Type roles are used (`rest_weight_control` repeats them).
    custom: Option<(String, Vec<String>)>,
    /// `useFrequency` with `frequencyWeight`: case replication weights (guide 69-71).
    /// `None` when off; the weight field itself is left unchanged in the output.
    freq_weight: Option<String>,
    reorder_nominal: bool,
    replace_scale_missing: bool,
    scale_continuous: bool,
    scale: Scale,
    outliers: Option<Outliers>,
    /// `outlierIdentCutoff` and `outliersOperation`, also used for the target.
    outlier_cutoff: f64,
    outliers_to_missing: bool,
    /// `uiRescaleScaleTargetUsingBoxCox`: the final `targetMean` and `targetSD`.
    box_cox_target: Option<(f64, f64)>,
    target_ext: String,
    /// `uiSupervisedMerging` (objective `custom` only): the `supervisedMergingPValue`.
    supervised_merge: Option<f64>,
    /// `uiUnsupervisedMergeNominal` (objective `custom` only): `unsupervisedMergingMinNumberOfCases`.
    unsupervised_merge: Option<i64>,
    /// `uiUnsupervisedMergeOrdinal` (objective `custom` only): `unsupervisedMergingMinNumberOfCases`.
    unsupervised_merge_ordinal: Option<i64>,
    /// `uiReplaceOrdinalFeaturesMissingValue`: an ordinal input's missing values become its median.
    replace_ordinal_missing: bool,
    /// `uiReplaceScaleTargetMissingValue`: a continuous target's missing values become its mean.
    target_missing_mean: bool,
    /// `uiDetectTargetOutliers`: a continuous target's outliers are coerced like an input's.
    target_outliers: bool,
    /// `uiReorderNominalTargetCategories`: a nominal target is recoded from the least frequent.
    reorder_target: bool,
    /// `uiFeatureSelection` (objective `custom` only): the `featureSelectionPValue`.
    feature_selection: Option<f64>,
    /// `uiSupervisedBinning` (objective `custom` only): the `homogeneousSubsetsPValue`.
    supervised_binning: Option<f64>,
    /// `uiFeatureConstruction` (objective `custom` only): `rootNameForConsturctedFeature`.
    feature_construction: Option<String>,
    check_feature_level: bool,
    ordinal_to_scale: i64,
    scale_to_ordinal: i64,
    prepare_dates: bool,
    compute_date_duration: bool,
    /// Date duration units: a fixed `dateUnit`, or chosen from the data
    /// (`autoChooseDateFieldUnit` `automatic`).
    date_units: Units,
    /// Time durations (`uiComputeTimeDuration`): the units and the reference second of the day.
    time_units: Units,
    ref_seconds: f64,
    extract_hour: bool,
    extract_minute: bool,
    extract_second: bool,
    suffix_hour: String,
    suffix_minute: String,
    suffix_second: String,
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

/// Duration units: `(divisor, suffix)` from the smallest unit up; `fixed` picks one, otherwise the
/// largest elapsed value chooses (see [`choose_unit`]).
struct Units {
    units: Vec<(f64, String)>,
    fixed: Option<usize>,
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
    /// A time-of-day input (`adp_time/time_*`).
    Time,
    Date,
    Continuous,
    Categorical,
    /// A numeric ordinal input.
    Ordinal,
    /// A target with a preparation step switched on (see [`prepare_target`]).
    Target,
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
        // `rest_freq_weight`: the weight field exists and is left unchanged (it is a
        // pass-through `None` role). The oracles show no effect for rescaling/recoding
        // without supervised steps (`rest_freq_weight` matches `rest_weight_control` to
        // every printed decimal); weighted supervised steps have no oracle.
        if let Some(name) = &self.freq_weight {
            let exists = input.model().fields().iter().any(|f| &f.name == name);
            if !exists {
                return Err(OperatorError::Settings(format!(
                    "frequencyWeight {name:?} is not an input field"
                )));
            }
            if self.supervised_merge.is_some()
                || self.unsupervised_merge.is_some()
                || self.unsupervised_merge_ordinal.is_some()
                || self.feature_selection.is_some()
                || self.supervised_binning.is_some()
            {
                return Err(OperatorError::Settings(
                    "GAP: frequency weights with supervised steps have no oracle".into(),
                ));
            }
        }
        let target = merge_target(self, input)?;
        let mut cols = Vec::new();
        // Positions in `cols` of the continuous inputs feature construction may group.
        let mut candidates = Vec::new();
        // Inputs the preparation looked at, and the fields it produced from them.
        let (mut prepared, mut produced) = (0usize, 0usize);
        for (index, field) in input.model().fields().iter().enumerate() {
            let mut effective = field.clone();
            effective.role = self.effective_role(field);
            let values = column(input, index);
            let class = classify(self, &effective);
            let before = cols.len();
            let examined = match class {
                Class::Time | Class::Date => true,
                Class::Continuous | Class::Categorical | Class::Ordinal => self.prepare_features,
                Class::Pass | Class::Target => false,
            };
            match class {
                Class::Time => {
                    prepare_time(self, &effective, index, &values, target.as_ref(), &mut cols)?
                }
                Class::Pass => cols.push(pass_col(&effective, values, index)),
                Class::Date => {
                    prepare_date(self, &effective, index, &values, target.as_ref(), &mut cols)?
                }
                Class::Continuous if self.prepare_features => {
                    let before = cols.len();
                    prepare_continuous(
                        self,
                        &effective,
                        index,
                        &values,
                        target.as_ref(),
                        false,
                        &mut cols,
                    )?;
                    if let Some(col) = cols.get(before).filter(|_| cols.len() == before + 1)
                        && col.field.measurement == Measurement::Continuous
                        && matches!(col.group, GROUP_CONTINUOUS | GROUP_PASS)
                    {
                        candidates.push(before);
                    }
                }
                Class::Categorical if self.prepare_features => {
                    prepare_categorical(
                        self,
                        &effective,
                        index,
                        &values,
                        target.as_ref(),
                        &mut cols,
                    )?;
                }
                Class::Ordinal if self.prepare_features => {
                    let before = cols.len();
                    prepare_ordinal(self, &effective, index, &values, target.as_ref(), &mut cols)?;
                    if let Some(col) = cols.get(before).filter(|_| cols.len() == before + 1)
                        && col.field.measurement == Measurement::Continuous
                        && col.group == GROUP_CONTINUOUS
                    {
                        candidates.push(before);
                    }
                }
                Class::Target if self.prepare_features => {
                    cols.push(prepare_target(self, &effective, index, &values)?);
                }
                Class::Continuous | Class::Categorical | Class::Ordinal | Class::Target => {
                    cols.push(pass_col(&effective, values, index));
                }
            }
            if examined {
                prepared += 1;
                produced += cols.len() - before;
            }
        }
        // `adp_time/datetime_off`: with every input excluded Modeler stops with this error.
        if prepared > 0 && produced == 0 {
            return Err(OperatorError::Data(
                "All features have been excluded. No inputs will be provided for model building."
                    .into(),
            ));
        }
        if let Some(root) = &self.feature_construction {
            construct_features(self, root, target.as_ref(), &candidates, &mut cols)?;
        }
        assemble(n_rows, cols)
    }
}

fn parse(s: Option<&Element>) -> Result<Prep, OperatorError> {
    reject_unknown(s)?;
    reject_unsupported(s)?;
    acknowledge_unused(s)?;
    let date_units = Units {
        units: vec![
            (1.0, text_or(s, "suffixElapsedDays", "_days")),
            (30.4375, text_or(s, "suffixElapsedMonths", "_months")),
            (365.25, text_or(s, "suffixElapsedYears", "_years")),
        ],
        fixed: match one_of(
            s,
            "autoChooseDateFieldUnit",
            &["fixed", "automatic"],
            "fixed",
        )?
        .as_str()
        {
            "automatic" => None,
            _ => Some(
                match one_of(s, "dateUnit", &["day", "month", "year"], "month")?.as_str() {
                    "day" => 0,
                    "month" => 1,
                    _ => 2,
                },
            ),
        },
    };
    let time_units = Units {
        units: vec![
            (1.0, text_or(s, "suffixElapsedSeconds", "_seconds")),
            (60.0, text_or(s, "suffixElapsedMinutes", "_minutes")),
            (3600.0, text_or(s, "suffixElapsedHours", "_hours")),
        ],
        fixed: match one_of(
            s,
            "autoChooseTimeFieldUnit",
            &["fixed", "automatic"],
            "fixed",
        )?
        .as_str()
        {
            "automatic" => None,
            _ => Some(
                match one_of(s, "timeUnit", &["second", "minute", "hour"], "hour")?.as_str() {
                    "second" => 0,
                    "minute" => 1,
                    _ => 2,
                },
            ),
        },
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
        exclude_almost_constant: flag(s, "uiExcludeAlmostConstantCatFeature", false)?,
        almost_constant_pct: percent_int(s, "cateFeatureScreenNumber", 95)? as f64,
        custom: custom_fields(s)?,
        freq_weight: frequency_field(s)?,
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
        box_cox_target: if flag(s, "uiRescaleScaleTargetUsingBoxCox", false)? {
            Some((finite(s, "targetMean", 0.0)?, positive(s, "targetSD", 1.0)?))
        } else {
            None
        },
        target_ext: text_or(s, "nameExtForTarget", "_transformed"),
        outlier_cutoff: positive(s, "outlierIdentCutoff", 3.0)?,
        outliers_to_missing: one_of(
            s,
            "outliersOperation",
            &["coerceToCutoffVal", "setToMissingVal"],
            "coerceToCutoffVal",
        )? == "setToMissingVal",
        supervised_merge: if custom_steps(s)? && flag(s, "uiSupervisedMerging", true)? {
            Some(unit_interval(s, "supervisedMergingPValue", 0.05)?)
        } else {
            None
        },
        feature_selection: if custom_steps(s)? && flag(s, "uiFeatureSelection", true)? {
            Some(unit_interval(s, "featureSelectionPValue", 0.05)?)
        } else {
            None
        },
        feature_construction: if custom_steps(s)? && flag(s, "uiFeatureConstruction", false)? {
            Some(text_or(s, "rootNameForConsturctedFeature", "feature"))
        } else {
            None
        },
        supervised_binning: if custom_steps(s)? && flag(s, "uiSupervisedBinning", true)? {
            Some(unit_interval(s, "homogeneousSubsetsPValue", 0.05)?)
        } else {
            None
        },
        unsupervised_merge: if custom_steps(s)? && flag(s, "uiUnsupervisedMergeNominal", false)? {
            Some(int_or(s, "unsupervisedMergingMinNumberOfCases", 10)?)
        } else {
            None
        },
        unsupervised_merge_ordinal: if custom_steps(s)?
            && flag(s, "uiUnsupervisedMergeOrdinal", false)?
        {
            Some(int_or(s, "unsupervisedMergingMinNumberOfCases", 10)?)
        } else {
            None
        },
        replace_ordinal_missing: flag(s, "uiReplaceOrdinalFeaturesMissingValue", false)?,
        target_missing_mean: flag(s, "uiReplaceScaleTargetMissingValue", false)?,
        target_outliers: flag(s, "uiDetectTargetOutliers", false)?,
        reorder_target: flag(s, "uiReorderNominalTargetCategories", false)?,
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
        date_units,
        time_units,
        ref_seconds: {
            let seconds = settings::integer(s, "fixedTime")?.unwrap_or(0);
            // 86400 (the end of the day) is what an unused reference saves.
            if !(0..=86_400).contains(&seconds) {
                return Err(OperatorError::Settings(format!(
                    "fixedTime: {seconds} is not a second of the day"
                )));
            }
            seconds as f64
        },
        extract_hour: flag(s, "uiExtractHourFromTime", false)?,
        extract_minute: flag(s, "uiExtractMinuteFromTime", false)?,
        extract_second: flag(s, "uiExtractSecondFromTime", false)?,
        suffix_hour: text_or(s, "suffixHour", "_hour"),
        suffix_minute: text_or(s, "suffixMinute", "_minute"),
        suffix_second: text_or(s, "suffixSecond", "_second"),
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

fn custom_fields(s: Option<&Element>) -> Result<Option<(String, Vec<String>)>, OperatorError> {
    let use_custom = flag(s, "useCustomSettings", false)?;
    let target = settings::text(s, "targetField")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let inputs: Vec<String> = settings::children(s, "inputFields", "field")
        .iter()
        .map(|element| element.text().trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    if use_custom {
        let Some(target) = target else {
            return Err(OperatorError::Settings(
                "useCustomSettings needs a targetField".into(),
            ));
        };
        if inputs.is_empty() {
            return Err(OperatorError::Settings(
                "useCustomSettings needs inputFields".into(),
            ));
        }
        Ok(Some((target, inputs)))
    } else {
        if target.is_some() || !inputs.is_empty() {
            return Err(OperatorError::Settings(
                "targetField/inputFields need useCustomSettings".into(),
            ));
        }
        Ok(None)
    }
}

/// `useFrequency` with `frequencyWeight` (`adp_rest/rest_freq_weight`).
fn frequency_field(s: Option<&Element>) -> Result<Option<String>, OperatorError> {
    let use_freq = flag(s, "useFrequency", false)?;
    let weight = settings::text(s, "frequencyWeight")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if use_freq {
        weight.map_or_else(
            || {
                Err(OperatorError::Settings(
                    "useFrequency needs a frequencyWeight".into(),
                ))
            },
            |name| Ok(Some(name)),
        )
    } else {
        if weight.is_some() {
            return Err(OperatorError::Settings(
                "frequencyWeight needs useFrequency".into(),
            ));
        }
        Ok(None)
    }
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
    for key in ["uiReplaceOrdinalTargetMissingValue", "useWeight"] {
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
    for key in ["analysisWeight"] {
        if settings::text(s, key).is_some_and(|value| !value.trim().is_empty()) {
            return Err(OperatorError::Settings(format!(
                "{key} is not supported yet"
            )));
        }
    }
    one_of(s, "excludeNoUsedField", &["filterOut"], "filterOut")?;
    one_of(s, "keepOrClearAnalysis", &["stop"], "stop")?;
    one_of(s, "referenceDateType", &["fixedDate"], "fixedDate")?;
    one_of(s, "referenceTimeType", &["fixedTime"], "fixedTime")?;
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
    let _ = flag(s, "transConsSelectFeatures", true)?;
    let _ = flag(s, "uiCheckTargetMeasureLevel", true)?;
    let _ = flag(s, "modelExist", false)?;
    let _ = flag(s, "clearAnalysis", false)?;
    // `adp_more/more_nominal_target_missing`: an empty-string target value is a category of its
    // own, untouched by this flag; system-null target values with it have no oracle (checked in
    // `prepare_target`).
    let _ = flag(s, "uiReplaceNominalTargetMissingValue", false)?;
    let min_cases = int_or(s, "unsupervisedMergingMinNumberOfCases", 10)?;
    if min_cases < 1 {
        return Err(OperatorError::Settings(format!(
            "unsupervisedMergingMinNumberOfCases: expected at least 1, found {min_cases}"
        )));
    }
    // `cateFeatureScreenNumber` is read in `parse` as `almost_constant_pct`.
    let _ = finite(s, "minMax_min", 0.0)?;
    let _ = finite(s, "minMax_max", 100.0)?;
    let _ = settings::integer(s, "todaysDate")?;
    let _ = settings::integer(s, "currentTime")?;
    Ok(())
}

/// Whether the transform-and-select steps apply. `misc_next/adp_balanced` saves them on under
/// objective `balance` with `transConsSelectFeatures` true, yet keeps `grp` (chi-square p .37
/// against `y`) unmerged and unscreened: only objective `custom` (`adp_next/*`) runs them.
fn custom_steps(s: Option<&Element>) -> Result<bool, OperatorError> {
    Ok(matches!(
        text_or(s, "analysisObjective", "balance").as_str(),
        "custom" | "Custom"
    ) && flag(s, "transConsSelectFeatures", true)?)
}

impl Prep {
    /// The role Modeler prepares with: the upstream Type role, or the explicit
    /// `useCustomSettings` target/inputs (`rest_custom_fields` keeps only `c_rel`;
    /// `rest_weight_control` repeats the Type roles).
    fn effective_role(&self, field: &Field) -> Role {
        if let Some((target, inputs)) = &self.custom {
            if field.name == *target {
                return Role::Target;
            }
            if inputs.contains(&field.name) {
                return Role::Input;
            }
            return match field.role {
                Role::Input | Role::Target => Role::None,
                _ => field.role,
            };
        }
        field.role
    }
}

fn classify(prep: &Prep, field: &Field) -> Class {
    // A time input without a duration or element is still excluded (`adp_time/date_*`: `t`).
    if field.storage == Storage::Time && field.role == Role::Input && prep.prepare_dates {
        return Class::Time;
    }
    if matches!(
        field.storage,
        Storage::Time | Storage::Timestamp | Storage::List | Storage::Unknown
    ) {
        return Class::Pass;
    }
    if field.role == Role::Target {
        let continuous = field.measurement == Measurement::Continuous
            && matches!(field.storage, Storage::Integer | Storage::Real);
        if (continuous
            && (prep.box_cox_target.is_some() || prep.target_missing_mean || prep.target_outliers))
            || (prep.reorder_target && is_nominal_like(field))
        {
            return Class::Target;
        }
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
    if field.measurement == Measurement::Ordinal {
        // String ordinals are categoricals to recode (`adp_rest/rest_base`,
        // `rest_strord_missing`, `rest_strord_merge`: `os_transformed` after the
        // nominal fields, missing as a category, nominal supervised merge).
        if matches!(field.storage, Storage::Integer | Storage::Real) {
            return Class::Ordinal;
        }
        if field.storage == Storage::String {
            return Class::Categorical;
        }
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
    target: Option<&MergeTarget>,
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
    if prep.extract_year {
        push_element(
            prep,
            field,
            index,
            1,
            &prep.suffix_year,
            &years,
            target,
            cols,
        )?;
    }
    if prep.extract_month {
        push_element(
            prep,
            field,
            index,
            2,
            &prep.suffix_month,
            &months,
            target,
            cols,
        )?;
    }
    if prep.extract_day {
        push_element(
            prep,
            field,
            index,
            3,
            &prep.suffix_day,
            &day_of_month,
            target,
            cols,
        )?;
    }
    if prep.compute_date_duration {
        let elapsed: Vec<Option<f64>> = days
            .iter()
            .map(|day| day.map(|value| value - prep.ref_days))
            .collect();
        let (divisor, suffix) = choose_unit(&prep.date_units, &elapsed, &[31.0, 366.0]);
        let series: Vec<Option<f64>> = elapsed.iter().map(|x| x.map(|x| x / divisor)).collect();
        // `rest_dates_selection`: date durations are screened like continuous inputs (both
        // `d_days` and `t_hours` drop with p above alpha).
        if !keep_date_continuous(prep, &series, target, &field.name)? {
            // Dropped, like a screened continuous input.
        } else {
            let name = extended(&field.name, suffix)?;
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
        }
    }
    // With no duration and no element the date is still excluded (`adp_time/time_*`: `d`).
    Ok(())
}

/// The duration unit: the fixed one, or (guide "Date/Time Handling", with the largest elapsed
/// value where the guide says the smallest) the first unit whose upper bound in `limits` exceeds
/// the largest signed elapsed value. Dates: days below 31, months below 366, else years
/// (`adp_time/date_auto` largest -3 days: days; `date_auto_after` 1461: years, where the smallest,
/// 366, would give months). Times: seconds below 60, minutes below 3600, else hours
/// (`time_auto_midnight` 86087 s: hours, the smallest, 307, would give minutes).
fn choose_unit<'a>(units: &'a Units, elapsed: &[Option<f64>], limits: &[f64]) -> (f64, &'a str) {
    let index = units.fixed.unwrap_or_else(|| {
        let largest = elapsed
            .iter()
            .flatten()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        limits
            .iter()
            .position(|limit| largest < *limit)
            .unwrap_or(limits.len())
    });
    let (divisor, suffix) = &units.units[index];
    (*divisor, suffix)
}

/// A time-of-day input (`adp_time/time_*`): with `uiComputeTimeDuration` the signed elapsed
/// seconds from `fixedTime` in the chosen unit (`time_hours_raw`: 00:05:07 against 12:00:00 is
/// -11.9147 hours), rescaled like a date duration; with the extraction flags its hour, minute and
/// second as date elements are (`time_extract`: 24 and 60 levels, recast and z-scored). The time
/// field itself is dropped. `rest_time_both` keeps both the duration and the three recast
/// elements (all z-scored).
fn prepare_time(
    prep: &Prep,
    field: &Field,
    index: usize,
    values: &[Value],
    target: Option<&MergeTarget>,
    cols: &mut Vec<OutCol>,
) -> Result<(), OperatorError> {
    let seconds: Vec<Option<i64>> = values
        .iter()
        .map(|value| match value {
            Value::Time(micros) if !field.is_missing(value) => Some(micros.div_euclid(1_000_000)),
            _ => None,
        })
        .collect();
    let as_f64: Vec<Option<f64>> = seconds.iter().map(|x| x.map(|x| x as f64)).collect();
    if screen_numeric(prep, &as_f64) {
        return Ok(());
    }
    if prep.compute_time_duration {
        let elapsed: Vec<Option<f64>> = as_f64
            .iter()
            .map(|x| x.map(|x| x - prep.ref_seconds))
            .collect();
        let (divisor, suffix) = choose_unit(&prep.time_units, &elapsed, &[60.0, 3600.0]);
        let series: Vec<Option<f64>> = elapsed.iter().map(|x| x.map(|x| x / divisor)).collect();
        if keep_date_continuous(prep, &series, target, &field.name)? {
            cols.push(OutCol {
                field: continuous_field(extended(&field.name, suffix)?),
                values: if prep.scale_continuous {
                    rescale(&series, date_scale(prep)?, prep.replace_scale_missing)
                } else {
                    optional_reals(&series)
                },
                group: GROUP_DATE_SCALE,
                key_a: as_u32(index)?,
                key_b: 0,
            });
        }
        // Without elements the duration is the only output; with them (`rest_time_both`) the
        // elements follow it.
        if !(prep.extract_hour || prep.extract_minute || prep.extract_second) {
            return Ok(());
        }
    }
    type Part<'a> = (bool, &'a str, fn(i64) -> i64);
    let parts: [Part<'_>; 3] = [
        (prep.extract_hour, &prep.suffix_hour, |s| s / 3600),
        (prep.extract_minute, &prep.suffix_minute, |s| s % 3600 / 60),
        (prep.extract_second, &prep.suffix_second, |s| s % 60),
    ];
    for (position, (wanted, suffix, part)) in parts.into_iter().enumerate() {
        if wanted {
            let components: Vec<Option<i64>> = seconds.iter().map(|x| x.map(part)).collect();
            push_element(
                prep,
                field,
                index,
                position + 1,
                suffix,
                &components,
                target,
                cols,
            )?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn push_element(
    prep: &Prep,
    field: &Field,
    field_index: usize,
    extract_index: usize,
    suffix: &str,
    components: &[Option<i64>],
    target: Option<&MergeTarget>,
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
        if !keep_date_continuous(prep, &series, target, &field.name)? {
            return Ok(());
        }
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
        // Ordinal date elements with feature selection have no oracle.
        if prep.feature_selection.is_some() && target.is_some() {
            return Err(OperatorError::Settings(
                "GAP: feature selection of date-derived fields has no oracle".into(),
            ));
        }
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

/// Whether a continuous date/time derivative survives feature selection
/// (`rest_dates_selection` drops `d_days`/`t_hours` with p above alpha, like a continuous
/// input). With a categorical target there is no oracle.
fn keep_date_continuous(
    prep: &Prep,
    series: &[Option<f64>],
    target: Option<&MergeTarget>,
    name: &str,
) -> Result<bool, OperatorError> {
    match (prep.feature_selection, target) {
        (Some(alpha), Some(MergeTarget::Continuous(y))) => Ok(correlation_p(series, y) <= alpha),
        (Some(_), Some(MergeTarget::Categorical(_))) => Err(OperatorError::Settings(format!(
            "GAP: feature selection of date-derived field {name:?} against a categorical target has no oracle"
        ))),
        _ => Ok(true),
    }
}

fn prepare_continuous(
    prep: &Prep,
    field: &Field,
    index: usize,
    values: &[Value],
    target: Option<&MergeTarget>,
    recast: bool,
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
    match (prep.feature_selection, target) {
        (Some(alpha), Some(MergeTarget::Continuous(y))) if correlation_p(&series, y) > alpha => {
            return Ok(());
        }
        // `rest_selection_cattarget` (`yn` Flag, `c_rel`/`c_noise`): both kept. Selection with a
        // categorical target does not drop continuous inputs (guide step 1 is stated for
        // continuous targets only, 885-888).
        _ => {}
    }
    if let (Some(alpha), Some(MergeTarget::Categorical(classes))) =
        (prep.supervised_binning, target)
    {
        // `rest_cont_missing_binning` (`c_miss` 15 missing, `replace_missing_continuous_inputs`
        // on): missing values are replaced by the valid mean before the cuts, so the cuts
        // include the 15 imputed rows (`-0.329…`, not `-0.413…` without them); missing target
        // rows are still left out (pairwise deletion, 60-66).
        let for_cuts: Vec<Option<f64>> = if prep.replace_scale_missing {
            let finite: Vec<f64> = series
                .iter()
                .flatten()
                .copied()
                .filter(|x| x.is_finite())
                .collect();
            if finite.is_empty() {
                return Ok(());
            }
            let mean = finite.iter().sum::<f64>() / finite.len() as f64;
            series.iter().map(|v| Some(v.unwrap_or(mean))).collect()
        } else {
            series.clone()
        };
        let Some(cuts) = homogeneous_cuts(&for_cuts, classes, alpha) else {
            // One homogeneous subset: the binned field is constant and excluded (`c_noise`).
            return Ok(());
        };
        let mut binned = ordinal_field(extended(&field.name, &prep.name_ext)?);
        binned.storage = Storage::Real;
        cols.push(OutCol {
            field: binned,
            values: for_cuts
                .iter()
                .map(|x| match x {
                    Some(x) if x.is_finite() => {
                        Value::real(cuts.iter().filter(|c| *x > **c).count() as f64)
                    }
                    _ => Value::Null,
                })
                .collect(),
            group: GROUP_BINNED,
            key_a: 0,
            key_b: index,
        });
        return Ok(());
    }
    let missing = series.iter().filter(|value| value.is_none()).count();
    let name = extended(&field.name, &prep.name_ext)?;
    cols.push(OutCol {
        field: continuous_field(name),
        values: rescale(&series, &prep.scale, prep.replace_scale_missing),
        group: GROUP_CONTINUOUS,
        // Recast ordinals follow the original continuous inputs (`adp_more/more_base`: `o12`
        // after `c1`).
        key_a: if recast { RECAST_KEY } else { as_u32(missing)? },
        key_b: index,
    });
    Ok(())
}

/// Sort key of recast ordinals in the continuous group: after the original continuous inputs,
/// before constructed features (`u32::MAX - 1`) and a Box-Cox target (`u32::MAX`).
const RECAST_KEY: u32 = u32::MAX - 2;

/// A numeric ordinal input (`adp_more/more_*`, `adp_rest/rest_ordinal_*`). More than `ordinalToScaleCategoriesNumThreshold`
/// distinct values recast it as continuous (`o12`, 12 levels, z-scored). Otherwise it is copied
/// unchanged (`o4`, `om` in `more_base`) unless a step changes it:
/// - with a target, the association screen and the supervised merge of adjacent levels
///   (`more_ordinal_merge_sup`: `o4` levels 1 and 2 join, codes 0, 1, 2 in level order; `om`
///   p .031 stays and levels 1-4 join; missing values stay missing;
///   `rest_ordinal_merge_missing`: `om` 10 missing become the lower-middle median 3, then
///   levels 1-2 join (codes 0, 1 in level order with missing, 2, 3));
/// - without one, `uiUnsupervisedMergeOrdinal` merges levels in value order by counts
///   (`more_ordinal_merge_unsup`: `om`'s 4-record level 5 joins level 4; missing values stay
///   missing; `o4`, whose levels all reach the threshold, keeps its name);
/// - `uiReplaceOrdinalFeaturesMissingValue` replaces missing values by the lower-middle median,
///   keeping the level values (`more_ordinal_missing`: `om_transformed`, missing to 3;
///   `rest_ordinal_median_between`: `o_med` 145x1/145x2 +10 missing → 1).
///
/// Merged codes are integers and follow the categorical fields; a median-filled field follows the
/// copied fields (`GROUP_PASS` `key_a` 1). A merge that changes nothing copies the field.
fn prepare_ordinal(
    prep: &Prep,
    field: &Field,
    index: usize,
    values: &[Value],
    target: Option<&MergeTarget>,
    cols: &mut Vec<OutCol>,
) -> Result<(), OperatorError> {
    let series: Vec<Option<f64>> = values.iter().map(|value| as_number(field, value)).collect();
    if screen_numeric(prep, &series) {
        return Ok(());
    }
    let distinct = distinct_f64(&series);
    if prep.check_feature_level && exceeds(distinct, prep.ordinal_to_scale) {
        let mut recast = field.clone();
        recast.measurement = Measurement::Continuous;
        return prepare_continuous(prep, &recast, index, values, target, true, cols);
    }
    let keys: Vec<Value> = series
        .iter()
        .map(|x| x.map_or(Value::Null, Value::real))
        .collect();
    // `rest_ordinal_merge_missing` (`om` 10 missing) and `rest_ordinal_median_between` (`o_med`
    // 145x1/145x2 +10 missing): missing ordinal values become the lower-middle valid value
    // (`om` 3, `o_med` 1), the median-category count updated (guide 336-344). No GAP for a
    // between-levels median: the lower middle is used.
    let median: Option<f64> = if prep.replace_ordinal_missing && series.iter().any(Option::is_none)
    {
        let mut valid: Vec<f64> = series.iter().flatten().copied().collect();
        if valid.is_empty() {
            None
        } else {
            valid.sort_by(f64::total_cmp);
            Some(valid[(valid.len() - 1) / 2])
        }
    } else {
        None
    };
    let merging = prep.supervised_merge.is_some() || prep.unsupervised_merge_ordinal.is_some();
    if merging && prep.replace_ordinal_missing {
        // Supervised merge after median replacement has an oracle (`om`); unsupervised merge
        // after replacement has none. When both flags are on with a target (`rest_ordinal_merge_missing`
        // saves `uiUnsupervisedMergeOrdinal` true alongside the supervised merge), the supervised
        // merge wins and the unsupervised flag is ignored.
        if prep.unsupervised_merge_ordinal.is_some() && prep.supervised_merge.is_none() {
            return Err(OperatorError::Settings(
                "GAP: merging an ordinal input whose missing values are replaced has no oracle"
                    .into(),
            ));
        }
        let Some(median) = median else {
            return Err(OperatorError::Settings(format!(
                "GAP: the median of {:?} has no valid values",
                field.name
            )));
        };
        let filled: Vec<Value> = series
            .iter()
            .map(|x| x.map_or(Value::real(median), Value::real))
            .collect();
        let screen = prep.supervised_merge.or(prep.feature_selection);
        let merged: Option<Vec<(Value, usize)>> = match (screen, target) {
            (Some(alpha), Some(target)) => {
                let (groups, continuous) = target_groups(&filled, target, &field.name)?;
                let all: Vec<&MergeGroup> = groups.iter().collect();
                if association_p(&all, continuous) > alpha {
                    return Ok(());
                }
                prep.supervised_merge
                    .map(|alpha| supervised_merge(groups, continuous, alpha, true))
            }
            _ => None,
        };
        let name = extended(&field.name, &prep.name_ext)?;
        if let Some(labeled) = merged {
            let groups = labeled
                .iter()
                .map(|(_, code)| *code)
                .max()
                .map_or(0, |m| m + 1);
            if groups < labeled.len() {
                let mut out = ordinal_field(name);
                out.storage = Storage::Integer;
                cols.push(OutCol {
                    field: out,
                    values: filled
                        .iter()
                        .map(|key| {
                            match labeled
                                .iter()
                                .find(|(k, _)| k.total_cmp(key) == Ordering::Equal)
                            {
                                Some((_, code)) => Value::Integer(*code as i64),
                                None => Value::Null,
                            }
                        })
                        .collect(),
                    group: GROUP_CATEGORICAL,
                    key_a: 0,
                    key_b: index,
                });
                return Ok(());
            }
        }
        // Merge changed nothing: fall through to the median-filled field below.
        let mut out = field.clone();
        out.name = name;
        cols.push(OutCol {
            field: out,
            values: values
                .iter()
                .zip(&series)
                .map(|(value, x)| match (x, field.storage) {
                    (Some(_), _) => value.clone(),
                    (None, Storage::Integer) => Value::Integer(median as i64),
                    (None, _) => Value::real(median),
                })
                .collect(),
            group: GROUP_PASS,
            key_a: 1,
            key_b: index,
        });
        return Ok(());
    }
    let screen = prep.supervised_merge.or(prep.feature_selection);
    let merged: Option<Vec<(Value, usize)>> = match (screen, target) {
        (Some(alpha), Some(target)) => {
            let (groups, continuous) = target_groups(&keys, target, &field.name)?;
            let all: Vec<&MergeGroup> = groups.iter().collect();
            if association_p(&all, continuous) > alpha {
                return Ok(());
            }
            prep.supervised_merge
                .map(|alpha| supervised_merge(groups, continuous, alpha, true))
        }
        _ => match (prep.unsupervised_merge_ordinal, target) {
            (Some(percent), None) => {
                let mut counts: Vec<(Value, usize)> = Vec::new();
                for key in keys.iter().filter(|k| !k.is_null()) {
                    match counts
                        .iter_mut()
                        .find(|(seen, _)| seen.total_cmp(key) == Ordering::Equal)
                    {
                        Some(slot) => slot.1 += 1,
                        None => counts.push((key.clone(), 1)),
                    }
                }
                counts.sort_by(|a, b| a.0.total_cmp(&b.0));
                let valid = counts.iter().map(|(_, c)| c).sum();
                Some(unsupervised_merge(counts, valid, percent, false))
            }
            _ => None,
        },
    };
    let name = extended(&field.name, &prep.name_ext)?;
    if let Some(labeled) = merged {
        let groups = labeled
            .iter()
            .map(|(_, code)| *code)
            .max()
            .map_or(0, |m| m + 1);
        if groups < labeled.len() {
            let mut out = ordinal_field(name);
            out.storage = Storage::Integer;
            cols.push(OutCol {
                field: out,
                values: keys
                    .iter()
                    .map(|key| {
                        match labeled
                            .iter()
                            .find(|(k, _)| k.total_cmp(key) == Ordering::Equal)
                        {
                            Some((_, code)) => Value::Integer(*code as i64),
                            None => Value::Null,
                        }
                    })
                    .collect(),
                group: GROUP_CATEGORICAL,
                key_a: 0,
                key_b: index,
            });
            return Ok(());
        }
    }
    if prep.replace_ordinal_missing && series.iter().any(Option::is_none) {
        let mut valid: Vec<f64> = series.iter().flatten().copied().collect();
        valid.sort_by(f64::total_cmp);
        // Lower middle (`o_med` 145x1/145x2 +10 missing → 1; `om` → 3).
        let median = valid[(valid.len() - 1) / 2];
        let mut out = field.clone();
        out.name = name;
        cols.push(OutCol {
            field: out,
            values: values
                .iter()
                .zip(&series)
                .map(|(value, x)| match (x, field.storage) {
                    (Some(_), _) => value.clone(),
                    (None, Storage::Integer) => Value::Integer(median as i64),
                    (None, _) => Value::real(median),
                })
                .collect(),
            group: GROUP_PASS,
            key_a: 1,
            key_b: index,
        });
        return Ok(());
    }
    cols.push(pass_col(field, values.to_vec(), index));
    Ok(())
}

/// Feature construction (guide "Feature Selection and Construction" and "Principal Component
/// Analysis", `adp_corr/corr_construct*`, `adp_rest/rest_construct_order`). Over the continuous inputs as prepared (z-scores, or
/// raw values without rescaling), for the correlation levels .9, .8, ..., .2 (exact decimals:
/// `x5`/`x6` at r .191 stay apart): the most correlated pair above the level starts a group, which
/// takes the input whose smallest absolute correlation with the members is largest and above the
/// level until it holds more than 5 (ties: dataset order); the group's first principal component
/// of the covariance matrix, `e'x / sqrt(lambda_1)` without centering, becomes `<root><k>` and its
/// members leave. `{x1, x2, x8}` and the anti-correlated `{x4, x7}` give `feature1` and
/// `feature2`. The loading signs make the last loading positive; the oracles fit that as well as
/// "largest loading positive" and "loadings sum positive", so a group where those disagree is a
/// `GAP:`. With feature selection, an input whose partial correlation with the target given a new
/// feature has p above `featureSelectionPValue` is dropped (guide step 7). Constructed features
/// go last, after categorical and date fields (`rest_construct_order` `feature1` after
/// `ncat_transformed`).
fn construct_features(
    prep: &Prep,
    root: &str,
    target: Option<&MergeTarget>,
    candidates: &[usize],
    cols: &mut Vec<OutCol>,
) -> Result<(), OperatorError> {
    let gap = |what: String| OperatorError::Settings(format!("GAP: feature construction: {what}"));
    let mut data: Vec<Vec<f64>> = Vec::with_capacity(candidates.len());
    for &position in candidates {
        let column: Option<Vec<f64>> = cols[position]
            .values
            .iter()
            .map(|value| match value {
                Value::Real(x) => Some(*x),
                Value::Integer(x) => Some(*x as f64),
                _ => None,
            })
            .collect();
        data.push(column.ok_or_else(|| {
            gap(format!(
                "{:?} has missing values",
                cols[position].field.name
            ))
        })?);
    }
    let selection_y = match (prep.feature_selection, target) {
        (Some(alpha), Some(MergeTarget::Continuous(y))) => {
            let y: Option<Vec<f64>> = y.iter().copied().collect();
            Some((
                alpha,
                y.ok_or_else(|| gap("the target has missing values".into()))?,
            ))
        }
        _ => None,
    };
    let m = data.len();
    let r: Vec<Vec<f64>> = (0..m)
        .map(|i| (0..m).map(|j| pearson(&data[i], &data[j])).collect())
        .collect();
    let mut remaining: Vec<usize> = (0..m).collect();
    let mut dropped: Vec<usize> = Vec::new();
    let mut features: Vec<(Vec<usize>, Vec<f64>)> = Vec::new();
    for level in (2..=9).rev().map(|k| k as f64 / 10.0) {
        loop {
            let mut best: Option<(f64, usize, usize)> = None;
            for (a, &i) in remaining.iter().enumerate() {
                for &j in &remaining[a + 1..] {
                    let value = r[i][j].abs();
                    if value > level && best.is_none_or(|(top, _, _)| value > top) {
                        best = Some((value, i, j));
                    }
                }
            }
            let Some((_, i, j)) = best else { break };
            let mut group = vec![i, j];
            while group.len() <= 5 {
                let mut next: Option<(f64, usize)> = None;
                for &k in remaining.iter().filter(|k| !group.contains(k)) {
                    let to_group = group
                        .iter()
                        .map(|&g| r[k][g].abs())
                        .fold(f64::INFINITY, f64::min);
                    if to_group > level && next.is_none_or(|(top, _)| to_group > top) {
                        next = Some((to_group, k));
                    }
                }
                match next {
                    Some((_, k)) => group.push(k),
                    None => break,
                }
            }
            let feature =
                principal_component(&group.iter().map(|&k| &data[k][..]).collect::<Vec<_>>())
                    .map_err(|what| gap(what.to_string()))?;
            remaining.retain(|k| !group.contains(k));
            if let Some((alpha, y)) = &selection_y {
                let r_yf = pearson(y, &feature);
                remaining.retain(|&k| {
                    let keep = partial_correlation_p(&data[k], y, &feature, r_yf) <= *alpha;
                    if !keep {
                        dropped.push(k);
                    }
                    keep
                });
            }
            features.push((group, feature));
        }
    }
    if features.is_empty() {
        return Ok(());
    }
    // `rest_construct_order` keeps `d_days`, `t_hours` and `ncat_transformed` alongside the new
    // `feature1`; binned fields and a Box-Cox target with construction have no oracle.
    if cols.iter().any(|col| {
        matches!(col.group, GROUP_BINNED)
            || (col.group == GROUP_CONTINUOUS && col.key_a == u32::MAX)
    }) {
        return Err(gap(
            "the place of constructed features next to binned or target fields has no oracle"
                .into(),
        ));
    }
    let mut gone: Vec<usize> = features
        .iter()
        .flat_map(|(group, _)| group.iter().map(|&k| candidates[k]))
        .chain(dropped.iter().map(|&k| candidates[k]))
        .collect();
    gone.sort_unstable();
    for position in gone.into_iter().rev() {
        cols.remove(position);
    }
    for (number, (_, values)) in features.into_iter().enumerate() {
        cols.push(OutCol {
            field: continuous_field(format!("{root}{}", number + 1)),
            values: values.into_iter().map(finite_real).collect(),
            group: GROUP_CONSTRUCTED,
            key_a: 0,
            key_b: number,
        });
    }
    Ok(())
}

/// Pearson correlation of two complete columns (0 when either is constant).
fn pearson(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len() as f64;
    let (ma, mb) = (a.iter().sum::<f64>() / n, b.iter().sum::<f64>() / n);
    let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        sab += (x - ma) * (y - mb);
        saa += (x - ma).powi(2);
        sbb += (y - mb).powi(2);
    }
    if saa <= 0.0 || sbb <= 0.0 {
        0.0
    } else {
        (sab / (saa * sbb).sqrt()).clamp(-1.0, 1.0)
    }
}

/// p-value of the partial correlation of input `x` with the target given a constructed feature
/// (guide "Partial correlation and P-value": t with `n - 3` df, clamped correlations).
fn partial_correlation_p(x: &[f64], y: &[f64], feature: &[f64], r_yf: f64) -> f64 {
    let (r_xy, r_xf) = (pearson(x, y), pearson(x, feature));
    let denominator = ((1.0 - r_xf * r_xf) * (1.0 - r_yf * r_yf)).sqrt();
    if denominator <= 0.0 {
        return 1.0;
    }
    let partial = ((r_xy - r_xf * r_yf) / denominator).clamp(-1.0, 1.0);
    let df = x.len() as f64 - 3.0;
    if df <= 0.0 {
        return 1.0;
    }
    if partial.abs() >= 1.0 {
        return 0.0;
    }
    let t = partial * (df / (1.0 - partial * partial)).sqrt();
    2.0 * (1.0 - dms_algo::stats::student_t_cdf(t.abs(), df))
}

/// The first principal component of `columns` (sample covariance), as `e'x / sqrt(lambda_1)`.
fn principal_component(columns: &[&[f64]]) -> Result<Vec<f64>, String> {
    let k = columns.len();
    let n = columns[0].len() as f64;
    let means: Vec<f64> = columns.iter().map(|c| c.iter().sum::<f64>() / n).collect();
    let covariance: Vec<Vec<f64>> = (0..k)
        .map(|i| {
            (0..k)
                .map(|j| {
                    columns[i]
                        .iter()
                        .zip(columns[j])
                        .map(|(a, b)| (a - means[i]) * (b - means[j]))
                        .sum::<f64>()
                        / (n - 1.0)
                })
                .collect()
        })
        .collect();
    let (values, vectors) =
        dms_algo::stats::jacobi_eigen(covariance).map_err(|error| error.to_string())?;
    let lambda = values[0];
    if !(lambda.is_finite() && lambda > 0.0) {
        return Err("the first eigenvalue is not positive".into());
    }
    let mut e: Vec<f64> = (0..k).map(|i| vectors[i][0]).collect();
    if e[k - 1] < 0.0 {
        e.iter_mut().for_each(|x| *x = -*x);
    }
    let largest = e.iter().copied().fold(0.0_f64, |a, b| a.max(b.abs()));
    let ambiguous = 1e-9 * largest;
    let largest_negative = e.iter().any(|x| *x < 0.0 && x.abs() > largest - ambiguous)
        && !e.iter().any(|x| *x > 0.0 && x.abs() > largest - ambiguous);
    let sum: f64 = e.iter().sum();
    if largest_negative || sum < -ambiguous {
        return Err("the loading sign conventions disagree".into());
    }
    let scale = lambda.sqrt();
    Ok((0..columns[0].len())
        .map(|row| (0..k).map(|i| e[i] * columns[i][row]).sum::<f64>() / scale)
        .collect())
}

/// Box-Cox grid of the guide ("Target Handling"): lambda from -3 to 3 in steps of 0.5.
const BOX_COX_GRID: (f64, f64, usize) = (-3.0, 0.5, 13);

/// A target with a preparation step (`adp_more/more_target_*`, `more_nominal_target*`,
/// `adp_rest/rest_target_*`). These
/// replace the target with `<name><nameExtForTarget>` right after the copied fields:
/// - `uiReorderNominalTargetCategories`: codes from the least frequent category (`yn`: c 0,
///   b 1, a 2); an empty string is a category (`ynm`: `""` 0);
/// - `uiDetectTargetOutliers`: values beyond the robust bounds are coerced (`coerceToCutoffVal`,
///   `yco` 49.27 to 19.21) or set to missing (`setToMissingVal`, `rest_target_outliers_delete`
///   `yco` 53.73 and -20.04 → null, 31.25 → null);
/// - `uiReplaceScaleTargetMissingValue`: missing values become the mean (`ycm`); with Box-Cox
///   they become the final mean (`rest_target_combined` `ycomb` 9 missing → 0).
///
/// Box-Cox ([`box_cox_target_with`]) follows the continuous inputs instead.
fn prepare_target(
    prep: &Prep,
    field: &Field,
    index: usize,
    values: &[Value],
) -> Result<OutCol, OperatorError> {
    let gap = |what: &str| OperatorError::Settings(format!("GAP: target {:?}: {what}", field.name));
    let name = extended(&field.name, &prep.target_ext)?;
    if is_nominal_like(field) {
        if values.iter().any(|value| field.is_missing(value)) {
            return Err(gap("missing nominal target values have no oracle"));
        }
        let mut counts: Vec<(Value, usize)> = Vec::new();
        for value in values {
            let key = category_key(field, value, prep.trim_blanks);
            match counts
                .iter_mut()
                .find(|(seen, _)| seen.total_cmp(&key) == Ordering::Equal)
            {
                Some(slot) => slot.1 += 1,
                None => counts.push((key, 1)),
            }
        }
        let labeled = assign_codes(counts);
        let mut out = nominal_field(name);
        out.role = Role::Target;
        return Ok(OutCol {
            field: out,
            values: values
                .iter()
                .map(|value| lookup(&labeled, &category_key(field, value, prep.trim_blanks)))
                .collect(),
            group: GROUP_PASS,
            key_a: 1,
            key_b: index,
        });
    }
    if prep.box_cox_target.is_some() {
        let mut series: Vec<Option<f64>> = values
            .iter()
            .map(|value| as_number(field, value).filter(|x| x.is_finite()))
            .collect();
        // Outliers first (guide order), then Box-Cox over the handled values
        // (`rest_target_combined`: `ycomb` outliers coerced, missing to the final mean).
        if prep.target_outliers {
            series = handle_outliers(
                &Outliers {
                    cutoff: prep.outlier_cutoff,
                    to_missing: prep.outliers_to_missing,
                },
                series,
                &field.name,
            )?;
        }
        return box_cox_target_with(prep, field, index, &series);
    }
    let mut series: Vec<Option<f64>> = values
        .iter()
        .map(|value| as_number(field, value).filter(|x| x.is_finite()))
        .collect();
    if prep.target_outliers {
        // `rest_target_outliers_delete`: outliers set to missing (guide 296-306), the rest
        // copied; stats updated (309-313) is the rescaling of the inputs, which already sees
        // the handled target only via merging/selection (none here).
        series = handle_outliers(
            &Outliers {
                cutoff: prep.outlier_cutoff,
                to_missing: prep.outliers_to_missing,
            },
            series,
            &field.name,
        )?;
    }
    if prep.target_missing_mean {
        let valid: Vec<f64> = series.iter().flatten().copied().collect();
        if !valid.is_empty() {
            let mean = valid.iter().sum::<f64>() / valid.len() as f64;
            series.iter_mut().for_each(|x| {
                x.get_or_insert(mean);
            });
        }
    }
    let mut out = continuous_field(name);
    out.role = Role::Target;
    Ok(OutCol {
        field: out,
        values: optional_reals(&series),
        group: GROUP_PASS,
        key_a: 1,
        key_b: index,
    })
}

/// The continuous target (`uiRescaleScaleTargetUsingBoxCox`, guide "Target Handling",
/// `adp_next/adp_target_rescale` and `adp_target_boxcox`): `c = min(y) - 1`; the lambda of the
/// grid with the largest log-likelihood `-n/2 ln((n-1)/n sd(g)^2) + (lambda-1) sum ln(y-c)`
/// (ties: the smallest lambda) gives `g = ((y-c)^lambda - 1)/lambda` (`ln(y-c)` at 0), then
/// `targetSD * (g - mean(g)) / sd(g) + targetMean` with the sample standard deviation.
/// `ycont` picks lambda 1, the skewed `c_skew` lambda 0. Missing values stay missing and are left
/// out of the estimates (`adp_more/more_target_boxcox_missing`), unless
/// `uiReplaceScaleTargetMissingValue` fills them with the final mean
/// (`rest_target_combined` `ycomb` missing → 0). The new field follows every
/// continuous input and replaces the target.
/// Box-Cox over an already outlier-handled series (`rest_target_combined`).
fn box_cox_target_with(
    prep: &Prep,
    field: &Field,
    index: usize,
    observed: &[Option<f64>],
) -> Result<OutCol, OperatorError> {
    let Some((final_mean, final_sd)) = prep.box_cox_target else {
        return Err(OperatorError::Settings(format!(
            "GAP: Box-Cox target {:?}: no targetMean/targetSD",
            field.name
        )));
    };
    let gap = |what: &str| {
        OperatorError::Settings(format!("GAP: Box-Cox target {:?}: {what}", field.name))
    };
    let y: Vec<f64> = observed.iter().flatten().copied().collect();
    if y.len() < 2 {
        return Err(gap("fewer than two records"));
    }
    let n_valid = y.len() as f64;
    let n_all = observed.len() as f64;
    let c = y.iter().copied().fold(f64::INFINITY, f64::min) - 1.0;
    let transform = |v: f64, lambda: f64| {
        if lambda == 0.0 {
            (v - c).ln()
        } else {
            ((v - c).powf(lambda) - 1.0) / lambda
        }
    };
    let moments = |lambda: f64| {
        let g: Vec<f64> = y.iter().map(|&v| transform(v, lambda)).collect();
        let mean = g.iter().sum::<f64>() / n_valid;
        let variance = g.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n_valid - 1.0);
        (mean, variance)
    };
    let sum_log: f64 = y.iter().map(|&v| (v - c).ln()).sum();
    let (start, step, count) = BOX_COX_GRID;
    let mut best: Option<(f64, f64)> = None;
    for j in 0..count {
        let lambda = start + step * j as f64;
        let (_, variance) = moments(lambda);
        let likelihood =
            -n_valid / 2.0 * ((n_valid - 1.0) / n_valid * variance).ln() + (lambda - 1.0) * sum_log;
        if likelihood.is_finite() && best.is_none_or(|(_, top)| likelihood > top) {
            best = Some((lambda, likelihood));
        }
    }
    let lambda = best.ok_or_else(|| gap("no finite log-likelihood"))?.0;
    let (mean, variance_valid) = moments(lambda);
    // With missing-value replacement the imputed rows sit at the final mean: they add 0 to
    // the sum of squares but increase the divisor (`rest_target_combined` `ycomb` 9 missing:
    // sd 5.654 over 299, not 5.741 over 290), like rescaling with imputation.
    let variance = if prep.target_missing_mean && observed.len() > y.len() {
        variance_valid * (n_valid - 1.0) / (n_all - 1.0)
    } else {
        variance_valid
    };
    let sd = variance.sqrt();
    if !(sd.is_finite() && sd > 0.0) {
        return Err(gap("constant after the transformation"));
    }
    let mut out = continuous_field(extended(&field.name, &prep.target_ext)?);
    out.role = Role::Target;
    Ok(OutCol {
        field: out,
        values: observed
            .iter()
            .map(|v| match v {
                Some(v) => finite_real(final_sd * (transform(*v, lambda) - mean) / sd + final_mean),
                // `rest_target_combined` fills the 9 missing `ycomb` with the final mean;
                // without `uiReplaceScaleTargetMissingValue` missing stays missing
                // (`adp_more/more_target_boxcox_missing`).
                None if prep.target_missing_mean => finite_real(final_mean),
                None => Value::Null,
            })
            .collect(),
        group: GROUP_CONTINUOUS,
        key_a: u32::MAX,
        key_b: index,
    })
}

/// The target the supervised merge tests categories against.
enum MergeTarget {
    Continuous(Vec<Option<f64>>),
    Categorical(Vec<Value>),
}

/// The single target of the input when a supervised step or a merge needs one. Supervised
/// merging without a target and unsupervised merging with one have no oracle (the guide merges by
/// counts only "if there is no target"); without a target the guide does no feature selection.
fn merge_target(prep: &Prep, input: &Table) -> Result<Option<MergeTarget>, OperatorError> {
    if prep.supervised_merge.is_none()
        && prep.unsupervised_merge.is_none()
        && prep.unsupervised_merge_ordinal.is_none()
        && prep.feature_selection.is_none()
        && prep.supervised_binning.is_none()
    {
        return Ok(None);
    }
    let gap = |what: &str| OperatorError::Settings(format!("GAP: Auto Data Prep merge: {what}"));
    let targets: Vec<usize> = (0..input.model().len())
        .filter(|&i| prep.effective_role(&input.model().fields()[i]) == Role::Target)
        .collect();
    let index = match targets.as_slice() {
        [] if prep.supervised_merge.is_some() => {
            return Err(gap("supervised merging without a target"));
        }
        [] => return Ok(None),
        [index] => *index,
        _ => return Err(gap("more than one target")),
    };
    if (prep.unsupervised_merge.is_some() || prep.unsupervised_merge_ordinal.is_some())
        && prep.supervised_merge.is_none()
    {
        return Err(gap("unsupervised merging with a target"));
    }
    let field = &input.model().fields()[index];
    let values = column(input, index);
    Ok(Some(match field.measurement {
        Measurement::Continuous => MergeTarget::Continuous(
            values
                .iter()
                .map(|value| as_number(field, value).filter(|x| x.is_finite()))
                .collect(),
        ),
        Measurement::Flag | Measurement::Nominal => MergeTarget::Categorical(
            values
                .iter()
                .map(|value| category_key(field, value, prep.trim_blanks))
                .collect(),
        ),
        _ => return Err(gap("the target is neither continuous nor flag/nominal")),
    }))
}

fn prepare_categorical(
    prep: &Prep,
    field: &Field,
    index: usize,
    values: &[Value],
    target: Option<&MergeTarget>,
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
    // Guide "Basic Variable Screening" step 3 (`adp_rest/rest_almost_constant`): a categorical
    // is dropped when its mode exceeds `cateFeatureScreenNumber` percent of all rows
    // (`acat` 270/300 = 90% > 80 drops; default 95 keeps). Greater-than, like the missing
    // share screen.
    if prep.exclude_almost_constant && n > 0 {
        let mode = counts
            .iter()
            .filter(|(key, _)| !key.is_null())
            .map(|(_, count)| *count)
            .max()
            .unwrap_or(0);
        if percent(mode, n) > prep.almost_constant_pct {
            return Ok(());
        }
    }
    if !prep.reorder_nominal && prep.supervised_merge.is_none() && prep.unsupervised_merge.is_none()
    {
        cols.push(pass_col(field, values.to_vec(), index));
        return Ok(());
    }
    // `rest_nominal_noreorder`: supervised merging without reordering still merges (`n1`
    // `k1`/`k2` → 2, `k4` → 1, `k3` → 0, the same codes as the reordered merge would give);
    // the reorder flag only skips the plain recoding above.
    let keys: Vec<Value> = values
        .iter()
        .map(|value| category_key(field, value, prep.trim_blanks))
        .collect();
    // Guide "Identify Highly Associated Categorical Features": with a target, a predictor whose
    // association p-value is above alpha is dropped, under supervised merging
    // (`adp_merge_supervised`) or feature selection (`adp_selection`) alike (`n_miss`, p .67); with
    // neither (`adp_binning`) it stays.
    let screen = prep.supervised_merge.or(prep.feature_selection);
    let labeled = match (screen, target) {
        (Some(alpha), Some(target)) => {
            let (groups, continuous) = target_groups(&keys, target, &field.name)?;
            let all: Vec<&MergeGroup> = groups.iter().collect();
            if association_p(&all, continuous) > alpha {
                return Ok(());
            }
            match prep.supervised_merge {
                Some(_) if keys.iter().any(Value::is_null) => {
                    return Err(OperatorError::Settings(format!(
                        "GAP: supervised merge of {:?}, which has missing values",
                        field.name
                    )));
                }
                Some(alpha) => string_codes(supervised_merge(groups, continuous, alpha, false)),
                None => assign_codes(counts),
            }
        }
        _ => match (prep.unsupervised_merge, target) {
            (Some(percent), None) => string_codes(unsupervised_merge(counts, n, percent, true)),
            _ => assign_codes(counts),
        },
    };
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

/// Date durations and recast date elements rescale like continuous inputs
/// (`rest_dates_minmax` min-maxs `d_days`/`t_hours` to 0..100).
fn date_scale(prep: &Prep) -> Result<&Scale, OperatorError> {
    Ok(&prep.scale)
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

/// One category's target summary during the supervised merge.
struct MergeGroup {
    keys: Vec<Value>,
    n: f64,
    sum: f64,
    sum_sq: f64,
    classes: Vec<f64>,
}

impl MergeGroup {
    fn ss(&self) -> f64 {
        (self.sum_sq - self.sum * self.sum / self.n).max(0.0)
    }

    fn join(self, other: MergeGroup) -> MergeGroup {
        let mut keys = self.keys;
        keys.extend(other.keys);
        MergeGroup {
            keys,
            n: self.n + other.n,
            sum: self.sum + other.sum,
            sum_sq: self.sum_sq + other.sum_sq,
            classes: self
                .classes
                .iter()
                .zip(&other.classes)
                .map(|(a, b)| a + b)
                .collect(),
        }
    }
}

/// Pearson chi-square of a groups-by-classes count table over the classes with a positive total.
fn chi_square(groups: &[&MergeGroup]) -> f64 {
    let total: f64 = groups.iter().map(|g| g.n).sum();
    let classes = groups.first().map_or(0, |g| g.classes.len());
    let mut statistic = 0.0;
    let mut used = 0usize;
    for j in 0..classes {
        let column: f64 = groups.iter().map(|g| g.classes[j]).sum();
        if column <= 0.0 {
            continue;
        }
        used += 1;
        for group in groups {
            let expected = group.n * column / total;
            if expected > 0.0 {
                statistic += (group.classes[j] - expected).powi(2) / expected;
            }
        }
    }
    let df = (groups.len().saturating_sub(1) * used.saturating_sub(1)) as f64;
    if df <= 0.0 {
        1.0
    } else {
        dms_algo::stats::chi_square_p(statistic, df)
    }
}

/// p-value of the association between `groups` and the target (guide "P-value Calculations"):
/// the one-way F test for a continuous target, Pearson chi-square for a categorical one. A
/// single group has p 1.
fn association_p(groups: &[&MergeGroup], continuous: bool) -> f64 {
    if groups.len() < 2 {
        return 1.0;
    }
    if !continuous {
        return chi_square(groups);
    }
    let n: f64 = groups.iter().map(|g| g.n).sum();
    let mean = groups.iter().map(|g| g.sum).sum::<f64>() / n;
    let between: f64 = groups
        .iter()
        .map(|g| g.n * (g.sum / g.n - mean).powi(2))
        .sum();
    let within: f64 = groups.iter().map(|g| g.ss()).sum();
    let (d1, d2) = ((groups.len() - 1) as f64, n - groups.len() as f64);
    if between == 0.0 {
        1.0
    } else if within <= 0.0 || d2 <= 0.0 {
        0.0
    } else {
        dms_algo::stats::f_p(between / d1 / (within / d2), d1, d2)
    }
}

/// A categorical predictor's categories with their target summaries, in value order, and whether
/// the target is continuous. Records missing the predictor or the target are left out (the
/// guide's pairwise deletion).
fn target_groups(
    keys: &[Value],
    target: &MergeTarget,
    name: &str,
) -> Result<(Vec<MergeGroup>, bool), OperatorError> {
    let continuous = matches!(target, MergeTarget::Continuous(_));
    let classes: Vec<Value> = match target {
        MergeTarget::Continuous(_) => Vec::new(),
        MergeTarget::Categorical(values) => {
            let mut seen: Vec<Value> = values.iter().filter(|v| !v.is_null()).cloned().collect();
            seen.sort_by(Value::total_cmp);
            seen.dedup_by(|a, b| a.total_cmp(b) == Ordering::Equal);
            seen
        }
    };
    let mut groups: Vec<MergeGroup> = Vec::new();
    for (row, key) in keys.iter().enumerate() {
        if key.is_null() {
            continue;
        }
        let (y, class) = match target {
            MergeTarget::Continuous(values) => match values.get(row).copied().flatten() {
                Some(y) => (y, None),
                None => continue,
            },
            MergeTarget::Categorical(values) => match values.get(row).filter(|v| !v.is_null()) {
                Some(value) => (0.0, classes.binary_search_by(|c| c.total_cmp(value)).ok()),
                None => continue,
            },
        };
        let position = match groups
            .iter()
            .position(|g| g.keys[0].total_cmp(key) == Ordering::Equal)
        {
            Some(position) => position,
            None => {
                if groups.len() >= MAX_CATEGORIES {
                    return Err(OperatorError::Data(format!(
                        "field {name:?} has more than {MAX_CATEGORIES} categories"
                    )));
                }
                groups.push(MergeGroup {
                    keys: vec![key.clone()],
                    n: 0.0,
                    sum: 0.0,
                    sum_sq: 0.0,
                    classes: vec![0.0; classes.len()],
                });
                groups.len() - 1
            }
        };
        let group = &mut groups[position];
        group.n += 1.0;
        group.sum += y;
        group.sum_sq += y * y;
        if let Some(class) = class {
            group.classes[class] += 1.0;
        }
    }
    groups.sort_by(|a, b| a.keys[0].total_cmp(&b.keys[0]));
    Ok((groups, continuous))
}

/// Supervised merge (guide "Supervised Merge", `adp_next/adp_merge_supervised` and
/// `adp_merge_flag`): the pair with the largest p-value (F with 1 and `n_i + n_j - 2` df, or
/// chi-square of the 2-row table) merges while that p-value is above `alpha`; categories start in
/// value order, a merged category goes last, and the final position is the code (`n_merge`: k1+k2
/// at p .976, then rare at .889, stop at 2e-11: k3 0, k4 1, k1/k2/rare 2).
/// For an ordinal (`adjacent`) only neighbouring levels may join and the merged level keeps its
/// place (guide step 4 and step 7).
fn supervised_merge(
    mut groups: Vec<MergeGroup>,
    continuous: bool,
    alpha: f64,
    adjacent: bool,
) -> Vec<(Value, usize)> {
    loop {
        let mut best: Option<(f64, usize, usize)> = None;
        for i in 0..groups.len() {
            let last = if adjacent {
                (i + 2).min(groups.len())
            } else {
                groups.len()
            };
            for j in i + 1..last {
                let p = association_p(&[&groups[i], &groups[j]], continuous);
                if best.is_none_or(|(top, _, _)| p > top) {
                    best = Some((p, i, j));
                }
            }
        }
        match best {
            Some((p, i, j)) if p > alpha => {
                let second = groups.remove(j);
                let first = groups.remove(i);
                let joined = first.join(second);
                if adjacent {
                    groups.insert(i, joined);
                } else {
                    groups.push(joined);
                }
            }
            _ => break,
        }
    }
    let mut labeled: Vec<(Value, usize)> = groups
        .into_iter()
        .enumerate()
        .flat_map(|(code, group)| group.keys.into_iter().map(move |key| (key, code)))
        .collect();
    labeled.sort_by(|left, right| left.0.total_cmp(&right.0));
    labeled
}

/// String codes of a nominal recoding (`adp_rest/rest_base` `many80`: 80 categories
/// become `"00"`..`"79"`, zero-padded to the width of the largest code so lexical
/// order stays numeric; fewer than 10 codes need no padding).
fn string_codes(labeled: Vec<(Value, usize)>) -> Vec<(Value, Value)> {
    let total = labeled
        .iter()
        .map(|(_, code)| *code)
        .max()
        .map_or(0, |m| m + 1);
    labeled
        .into_iter()
        .map(|(key, code)| (key, code_string(code, total)))
        .collect()
}

/// Zero-padded code: `index` among `total` codes, width of `total - 1`.
fn code_string(index: usize, total: usize) -> Value {
    let width = total.saturating_sub(1).to_string().len().max(1);
    let mut text = index.to_string();
    while text.len() < width {
        text.insert(0, '0');
    }
    Value::string(text)
}

/// Supervised binning (guide "Supervised Binning", `adp_next/adp_binning`): the predictor's class
/// means, ascending, are grouped into homogeneous subsets with the Scheffe critical value
/// `sqrt(2 (J-1) F_{1-alpha}(J-1, N-J)) * MS * sqrt(sum(1/W_i) / J)` (`MS` the pooled standard
/// deviation; with `N = J` the range of the means); the cut between subsets `a` and `b` is
/// `mean_a + sd_a / (sd_a + sd_b) * (mean_b - mean_a)`, and bin `k` holds `cut_k < x <= cut_k+1`.
/// `None` when the means form one subset. `c_out` cuts at 0.80197, `c_skew` at 11.1323 (against
/// the flag `ycat`); `c_noise` is one subset. Records with a missing target are left out.
fn homogeneous_cuts(x: &[Option<f64>], classes: &[Value], alpha: f64) -> Option<Vec<f64>> {
    // Per class: count, sum and the values for the within-class sum of squares.
    let mut stats: Vec<(Value, f64, f64, Vec<f64>)> = Vec::new();
    for (value, class) in x.iter().zip(classes) {
        let (Some(value), false) = (value, class.is_null()) else {
            continue;
        };
        match stats
            .iter_mut()
            .find(|(seen, ..)| seen.total_cmp(class) == Ordering::Equal)
        {
            Some(entry) => {
                entry.1 += 1.0;
                entry.2 += value;
                entry.3.push(*value);
            }
            None => stats.push((class.clone(), 1.0, *value, vec![*value])),
        }
    }
    // (mean, count, within sum of squares), means ascending.
    let mut subsets: Vec<(f64, f64, f64)> = stats
        .into_iter()
        .map(|(_, w, sum, values)| {
            let mean = sum / w;
            (mean, w, values.iter().map(|v| (v - mean).powi(2)).sum())
        })
        .collect();
    subsets.sort_by(|a, b| a.0.total_cmp(&b.0));
    let j = subsets.len();
    if j < 2 {
        return None;
    }
    let n: f64 = subsets.iter().map(|s| s.1).sum();
    let jf = j as f64;
    let critical = if n <= jf {
        subsets[j - 1].0 - subsets[0].0
    } else {
        let ms = (subsets.iter().map(|s| s.2).sum::<f64>() / (n - jf)).sqrt();
        let r =
            (2.0 * (jf - 1.0) * dms_algo::stats::f_quantile(1.0 - alpha, jf - 1.0, n - jf)).sqrt();
        r * ms * (subsets.iter().map(|s| 1.0 / s.1).sum::<f64>() / jf).sqrt()
    };
    let mut merged: Vec<(f64, f64)> = Vec::new();
    let mut i = 0;
    while i < j {
        let mut q = j - 1;
        while q > i && (subsets[q].0 - subsets[i].0).abs() > critical {
            q -= 1;
        }
        let group = &subsets[i..=q];
        let w: f64 = group.iter().map(|s| s.1).sum();
        let mean = group.iter().map(|s| s.1 * s.0).sum::<f64>() / w;
        let m2: f64 = group.iter().map(|s| s.2 + s.1 * (mean - s.0).powi(2)).sum();
        merged.push((
            mean,
            if w > 1.0 {
                (m2 / (w - 1.0)).sqrt()
            } else {
                0.0
            },
        ));
        i = q + 1;
    }
    if merged.len() < 2 {
        return None;
    }
    Some(
        merged
            .windows(2)
            .map(|pair| {
                let ((ma, sa), (mb, sb)) = (pair[0], pair[1]);
                if sa + sb > 0.0 {
                    ma + sa / (sa + sb) * (mb - ma)
                } else {
                    (ma + mb) / 2.0
                }
            })
            .collect(),
    )
}

/// Two-sided p-value of the Pearson correlation of a continuous predictor with the continuous
/// target over the records where both are present (guide "Correlation and P-value": t with
/// `n - 2` df; `|r| = 1` gives 0). `adp_next/adp_selection`: `c_noise` r .066, p .25 is dropped;
/// `c_out` and `c_skew` (p < 1e-12) stay.
fn correlation_p(x: &[Option<f64>], y: &[Option<f64>]) -> f64 {
    let pairs: Vec<(f64, f64)> = x
        .iter()
        .zip(y)
        .filter_map(|(a, b)| match (a, b) {
            (Some(a), Some(b)) if a.is_finite() && b.is_finite() => Some((*a, *b)),
            _ => None,
        })
        .collect();
    let n = pairs.len() as f64;
    if n < 3.0 {
        return 1.0;
    }
    let (mx, my) = (
        pairs.iter().map(|p| p.0).sum::<f64>() / n,
        pairs.iter().map(|p| p.1).sum::<f64>() / n,
    );
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (a, b) in &pairs {
        sxy += (a - mx) * (b - my);
        sxx += (a - mx).powi(2);
        syy += (b - my).powi(2);
    }
    if sxx <= 0.0 || syy <= 0.0 {
        return 1.0;
    }
    let r = (sxy / (sxx * syy).sqrt()).clamp(-1.0, 1.0);
    if r.abs() >= 1.0 {
        return 0.0;
    }
    let t = r * ((n - 2.0) / (1.0 - r * r)).sqrt();
    2.0 * (1.0 - dms_algo::stats::student_t_cdf(t.abs(), n - 2.0))
}

/// Unsupervised merge (guide "Unsupervised Merge", `adp_next/adp_merge_unsupervised`):
/// categories, the missing value included, from lowest to highest count (ties in value order)
/// are joined while a group holds fewer than `[percent% x N]` records; a short tail joins per the
/// guide's step 4. Codes follow the groups (`n_merge`: rare(3)+k2 0, k4 1, k3 2, k1 3 at 30 of
/// 300; `n_miss`: missing(12)+q 0, r 1, p 2).
/// `by_count` (nominal) sorts the categories by count first; an ordinal keeps the value order it
/// is given and counts only its valid records in `n`.
fn unsupervised_merge(
    mut counts: Vec<(Value, usize)>,
    n: usize,
    percent: i64,
    by_count: bool,
) -> Vec<(Value, usize)> {
    if by_count {
        counts.sort_by(|left, right| {
            left.1
                .cmp(&right.1)
                .then_with(|| left.0.total_cmp(&right.0))
        });
    }
    let threshold = (percent as f64 * n as f64 / 100.0).round() as usize;
    let count = counts.len();
    let mut group_of = vec![0usize; count];
    let (mut j1, mut j2, mut g) = (0usize, 0usize, 0usize);
    while j1 < count {
        let sum: usize = counts[j1..=j2].iter().map(|(_, c)| c).sum();
        if sum >= threshold {
            group_of[j1..=j2].iter_mut().for_each(|slot| *slot = g);
            g += 1;
            j1 = j2 + 1;
            j2 = j1;
        } else if j2 + 1 < count {
            j2 += 1;
        } else {
            // Step 4: the tail j1.. never reaches the threshold.
            if g == 0 {
                group_of[..count - 1].iter_mut().for_each(|slot| *slot = 0);
                group_of[count - 1] = 1;
            } else if g == 1 {
                group_of[j1..].iter_mut().for_each(|slot| *slot = 1);
            } else {
                group_of[j1..].iter_mut().for_each(|slot| *slot = g - 1);
            }
            break;
        }
    }
    let mut labeled: Vec<(Value, usize)> = counts
        .into_iter()
        .zip(group_of)
        .map(|((key, _), group)| (key, group))
        .collect();
    labeled.sort_by(|left, right| left.0.total_cmp(&right.0));
    labeled
}

fn assign_codes(mut counts: Vec<(Value, usize)>) -> Vec<(Value, Value)> {
    counts.sort_by(|left, right| {
        left.1
            .cmp(&right.1)
            .then_with(|| left.0.total_cmp(&right.0))
    });
    let total = counts.len();
    let mut labeled: Vec<(Value, Value)> = counts
        .into_iter()
        .enumerate()
        .map(|(index, (key, _))| (key, code_string(index, total)))
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

/// The day number of a saved reference date. Modeler saves the picked date as the instant of
/// local midnight and counts whole UTC days from it (`adp_time/date_years_raw`: 2022-01-01 picked
/// at UTC+1 counts from 2021-12-31, so 2019-01-01 is -1095 days, -2.99795 years).
fn epoch_days(raw: i64) -> f64 {
    let abs = raw.unsigned_abs();
    if abs > 1_000_000_000_000 {
        raw.div_euclid(86_400_000) as f64
    } else if abs > 1_000_000_000 {
        raw.div_euclid(86_400) as f64
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

#[cfg(test)]
mod tests {
    use super::*;

    fn groups(counts: &[usize], n: usize) -> Vec<String> {
        let keyed = counts
            .iter()
            .enumerate()
            .map(|(i, &c)| (Value::string(format!("c{i}")), c))
            .collect();
        unsupervised_merge(keyed, n, 10, true)
            .into_iter()
            .map(|(_, code)| code.to_string())
            .collect()
    }

    /// The guide's step 4 for a tail that never reaches `[10% x N]` (30 of 300 here). Counts are
    /// given in ascending order, the order the merge walks them.
    #[test]
    fn unsupervised_merge_tail_rules_follow_the_guide() {
        // g = 1 (no group yet): all but the last category form group 1, the last stays alone.
        assert_eq!(groups(&[5, 6, 7], 300), ["0", "0", "1"]);
        // g = 2: 12+12+12 forms group 1, the 24-record tail forms group 2.
        assert_eq!(groups(&[12; 5], 300), ["0", "0", "0", "1", "1"]);
        // g = 3: groups 1 and 2 of 36, the 24-record tail joins group 2.
        assert_eq!(
            groups(&[12; 8], 300),
            ["0", "0", "0", "1", "1", "1", "1", "1"]
        );
    }
}
