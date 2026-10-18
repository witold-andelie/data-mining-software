# `autodataprep` node (`AutoDataPrepNode`)

Saved names are the `properties` children of `corpus/misc_next/adp_balanced.str`, `adp_rescale.str` and `adp_dates.str` (probe `xtask/oracle/probes/misc_next_streams.py`). Defaults below are the balanced stream. The node reads every name it knows; a value outside the accepted set is `OperatorError::Settings` naming that property. Rules and `GAP:` notes: `crates/exec/src/nodes/auto_data_prep.rs`.

| file element | accepted | balanced default | effect |
|---|---|---|---|
| `analysisObjective` | `balance`, `speed`, `accuracy`, `custom` (scripting capitals too) | `balance` | none; the flags below decide |
| `prepareFeatures` | true, false | true | false copies inputs instead of rescaling or recoding |
| `uiScaleContinuousFeatures` | true, false | true | z-score continuous inputs and recast date elements |
| `scaleContinuousMode` | `zScoreTransform` | `zScoreTransform` | min-max is rejected |
| `zScore_mean`, `zScore_SD` | finite, sd > 0 | 0, 1 | `(x-mean)/sd * sd + mean` |
| `uiReplaceScaleFeaturesMissingValue` | true, false | true | true: missing continuous → final mean, divisor `n-1` over all rows |
| `uiCheckFeatureMeasureLevel` | true, false | true | ordinal with more than `ordinalToScaleCategoriesNumThreshold` distinct values becomes continuous; continuous with fewer than `scaleToOrdinalValuesNumThreshold` stays ordinal |
| `uiReorderNominalCatsPriorToMerging` | true, false | true | frequency codes `"0"`, `"1"`, ... |
| `nameExtForInputs` | text | `_transformed` | suffix for rescaled inputs and recoded categories |
| `prepareDatesAndTimes` | true, false | true | input-role dates |
| `uiComputeDateDuration` | true, false | true | `{field}{suffix}` |
| `autoChooseDateFieldUnit` | `fixed` | `fixed` | automatic unit is rejected |
| `dateUnit` | `month`, `day` | `month` | `/ 30.4375` or day number; `year` is rejected |
| `referenceDateType` | `fixedDate` | `fixedDate` | |
| `fixedDate` | integer (ms, seconds, or day number) | `1792278336753` | offset; cancels under z-score |
| `uiExtractYearFromDate`, `uiExtractMonthFromDate`, `uiExtractDayFromDate` | true, false | false | `{field}` + `suffixYear` / `suffixMonth` / `suffixDay` |
| `excludeNoUsedField` | `filterOut` | `filterOut` | replaced inputs are omitted |
| `excludePoorQualityFeatures` | true, false | true | fewer than 2 distinct non-missing values |
| `uiExcludeFeatureIfHasHigherPercentageOfMissingValue` | true, false | true | missing share **greater than** `maxNumberOfMissingValue` |
| `uiExcludeFeatureIfHasManyUniqueCatogories` | true, false | true | non-missing categories **greater than** `maxNOfUniqueCategories` (saved spelling) |
| `trimBlanks` | true, false | true | category identity only |
| `uiSupervisedMerging`, `uiSupervisedBinning`, `uiFeatureSelection`, `transConsSelectFeatures` | true, false | true | accepted, not applied (`GAP:`) |
| `uiComputeTimeDuration` | true, false | true | no time field: no effect; a time field is rejected. Timestamps are copied |
| `uiReplaceNominalFeaturesMissingValue` | true | true | false rejected. True does not mode-impute (`GAP:`) |
| `uiReplaceOrdinalFeaturesMissingValue` | false | false | true rejected |
| `useCustomSettings`, `useFrequency`, `useWeight` | false | false | true rejected |
| `inputFields`, `targetField`, `frequencyWeight`, `analysisWeight` | empty | empty | non-empty rejected |

Display metadata (`annotation`, `name`, `position`, and the same list as other field nodes) is ignored. Target rescale, target reorder, outlier handling, unsupervised merge, feature construction and almost-constant category screening are rejected when their flag is true.
