# `logreg` node (`BuildLogisticNode`)

Probe `xtask/oracle/probes/settings_logreg.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `change: instantiate fields by running a temporary Table node on the source` | (none) |  -> (no change) |  | no |
| `custom_fields` | `autoModelFieldSelection` | True -> false | False; file: true | yes |
| `target` | `modelTargetFieldSelection` | Drug -> Drug |  | yes |
| `inputs` | `modelInputFieldSelection`<br>also: `modelInputFieldSelection/field[0]`, `modelInputFieldSelection/field[1]`, `modelInputFieldSelection/field[2]` | ['BP', 'Cholesterol', 'Age'] -> (removed) | [] | yes |
| `partition` | `partitionField` | Test -> Test |  | yes |
| `use_model_name` | `autoModelName` | True -> false | False; file: true | yes |
| `model_name` | `modelName` | Log_x -> Log_x |  | yes |
| `use_partitioned_data` | (none) | True -> (no change) | True | no |
| `logistic_procedure` | `logisticProcedure` | Binomial -> Binomial<br>Multinomial -> Multinomial | Multinomial; file: Multinomial | yes |
| `include_constant` | `includeConstant` | False -> false | True; file: true | yes |
| `mode` | `expertMode` | Expert -> true<br>Simple -> false | Simple; file: false | yes |
| `method` | `LogregMethod` | Stepwise -> stepwise<br>Forwards -> forwards<br>Backwards -> backwards<br>BackwardsStepwise -> remove<br>Enter -> enter | Enter; file: enter | yes |
| `binomial_method` | `LogregBinomialMethod` | Forwards -> Forwards<br>Backwards -> Backwards<br>Enter -> Enter | Enter; file: Enter | yes |
| `model_type` | `modelType` | FullFactorial -> fullFactorial<br>Custom -> custom<br>MainEffects -> mainEffects | MainEffects; file: mainEffects | yes |
| `custom_terms` | `customEffects`<br>also: `customEffects/persistentObjectListMember[1]/EffectTerm`, `customEffects/persistentObjectListMember[0]/EffectTerm[0]`, `customEffects/persistentObjectListMember[0]/EffectTerm[1]` | [['BP', 'Sex'], ['Age'], ['Na', 'K']] -> (removed) | [] | yes |
| `multinomial_base_category` | `multinomialBaseCategory` | BP -> BP |  | yes |
| `binomial_categorical_input` | `logisticInputsTable/rows`<br>also: `logisticInputsTable/rows/row/field`, `logisticInputsTable/rows/row/contrast`, `logisticInputsTable/rows/row/base_category` | Sex -> (removed) |  | yes |
| `binomial_input_contrast[field]` | `logisticInputsTable/rows/row/contrast` | Simple -> Simple<br>Indicator -> Indicator<br>Difference -> Difference<br>Helmert -> Helmert<br>Repeated -> Repeated<br>Polynomial -> Polynomial<br>Deviation -> Deviation | Indicator; file: Indicator | yes |
| `binomial_input_category[field]` | `logisticInputsTable/rows/row/base_category` | First -> (no change)<br>Last -> Last | First | yes |
| `scale` | `scale` | Pearson -> pearsonScaling<br>Deviance -> devianceScaling<br>UserDefined -> userDefined<br>None -> none | None; file: none | yes |
| `scale_value` | `scaleValue` | 3.0 -> 3.0 | 1.0; file: 1.0 | yes |
| `all_probabilities` | `appendProbabilities` | True -> true | False; file: false | yes |
| `tolerance` | `singularityTolerance` | 1.0E-5 -> 1.0e-5<br>1.0E-6 -> 1.0e-6<br>1.0E-7 -> 1.0e-7<br>1.0E-8 -> 1.0e-8<br>1.0E-9 -> 1.0e-9<br>1.0E-10 -> 1.0e-10 | 1.0E-8; file: 1.0e-8 | yes |
| `min_terms` | `minTerms` | 7 -> 7 | 0; file: 0 | yes |
| `use_max_terms` | `LogregEnableMaxTerms` | True -> true | False; file: false | yes |
| `max_terms` | (none) | 10 -> (no change) | 10 | no |
| `entry_criterion` | `stepwiseEntryMethod` | LR -> LR<br>Score -> Score | Score; file: Score | yes |
| `removal_criterion` | `stepwiseRemovalMethod` | Wald -> Wald<br>LR -> LR | LR; file: LR | yes |
| `probability_entry` | `pIn` | 0.03 -> 0.03 | 0.05; file: 0.05 | yes |
| `probability_removal` | `pOut` | 0.05 -> 0.05 | 0.1; file: 0.1 | yes |
| `binomial_probability_entry` | `binomialPIn` | 0.04 -> 0.04 | 0.05; file: 0.05 | yes |
| `binomial_probability_removal` | `binomialPOut` | 0.2 -> 0.2 | 0.1; file: 0.1 | yes |
| `requirements` | `LogregSteppingRequirements` | HierarchyAll -> hierarchy_all<br>Containment -> containment_all<br>None -> none<br>HierarchyDiscrete -> hierarchy_discrete | None; file: none | yes |
| `max_iterations` | `maximumIterations` | 50 -> 50 | 20; file: 20 | yes |
| `max_steps` | `maximumStepHalving` | 3 -> 3 | 5; file: 5 | yes |
| `p_converge` | `parameterConvergence` | 1.0E-5 -> 1.0e-5<br>1.0E-6 -> 1.0e-6<br>1.0E-7 -> 1.0e-7<br>1.0E-8 -> 1.0e-8<br>1.0E-4 -> 1.0e-4 | 1.0E-6; file: 1.0e-6 | yes |
| `l_converge` | `logLikelihoodConvergence` | 0 -> (no change)<br>1.0E-1 -> 1.0e-1<br>1.0E-2 -> 1.0e-2<br>1.0E-3 -> 1.0e-3<br>1.0E-4 -> 1.0e-4<br>1.0E-5 -> 1.0e-5 | 0 | yes |
| `delta` | `delta` | 0.03 -> 0.03 | 0.0; file: 0.0 | yes |
| `iteration_history` | `iterationHistory` | True -> true | False; file: false | yes |
| `summary` | (none) | True -> (no change) | True | no |
| `likelihood_ratio` | `likelihoodRatioTests` | True -> true | False; file: false | yes |
| `asymptotic_correlation` | `asymptoticCorrelation` | True -> true | False; file: false | yes |
| `goodness_fit` | `goodnessOfFit` | True -> true | False; file: false | yes |
| `parameters` | (none) | True -> (no change) | True | no |
| `asymptotic_covariance` | `asymptoticCovariance` | True -> true | False; file: false | yes |
| `classification_table` | `classificationTable` | True -> true | False; file: false | yes |
| `stepwise_summary` | (none) | True -> (no change) | True | no |
| `info_criteria` | (none) | True -> (no change) | True | no |
| `monotonicity_measures` | `associationTable` | True -> true | False; file: false | yes |
| `history_steps` | `historyIterations` | 3 -> 3 | 1; file: 1 | yes |
| `confidence_interval` | `confidenceInterval` | 90 -> 90.0 | 95.0; file: 95.0 | yes |
| `binomial_output_display` | `binomialOutputDisplay` | at_last_step -> at_last_step<br>at_each_step -> at_each_step | at_each_step; file: at_each_step | yes |
| `binomial_goodness_of_fit` | `binomialGoodnessOfFit` | True -> true | False; file: false | yes |
| `binomial_parameters` | `binomialCorrelationOfEstimates` | True -> true | False; file: false | yes |
| `binomial_iteration_history` | `binomialIterationHistory` | True -> true | False; file: false | yes |
| `binomial_classification_plots` | `binomialClassificationPlots` | True -> true | False; file: false | yes |
| `binomial_ci_enable` | `binomialCIEnable` | True -> true | False; file: false | yes |
| `binomial_residual_enable` | `binomialCasewiseEnable` | True -> true | False; file: false | yes |
| `binomial_ci` | `binomialCIValue` | 85 -> 85 | 95; file: 95 | yes |
| `binomial_residual` | `binomialCasewiseOutliers` | all -> all<br>outliers -> outliers | outliers; file: outliers | yes |
| `binomial_outlier_threshold` | `binomialCasewiseThreshold` | 3.5 -> 3.5 | 2.0; file: 2.0 | yes |
| `binomial_classification_cutoff` | `binomialClassificationCutoff` | 0.6 -> 0.6 | 0.5; file: 0.5 | yes |
| `binomial_removal_criterion` | `binomialStepwiseRemovalMethod` | Wald -> Wald<br>Conditional -> Conditional<br>LR -> LR | LR; file: LR | yes |
| `calculate_variable_importance` | `calculateVariableImportances` | True -> true | False; file: false | yes |
| `calculate_raw_propensities` | `calculateRawPropensities` | True -> true | False; file: false | yes |

## Set failed

- `binomial_method` = BackwardsStepwise: The value 'BackwardsStepwise' is not valid for the property 'binomial_method'
- `confidence_interval` = True: The value 'true' is not valid for the property 'confidence_interval'

## Set succeeded but file unchanged

- `change: instantiate fields by running a temporary Table node on the source`
- `use_partitioned_data`
- `max_terms`
- `summary`
- `parameters`
- `stepwise_summary`
- `info_criteria`

## Observations

- Common modeling properties in the XML `properties` (not the attachment): `custom_fields` -> `autoModelFieldSelection` (inverse: True -> `false`), `target` -> `modelTargetFieldSelection`, `inputs` -> `modelInputFieldSelection/field[n]` (single input is unindexed), `partition` -> `partitionField`, `use_model_name` -> `autoModelName` (inverse), `model_name` -> `modelName`, `use_frequency`/`frequency_field` -> `useFrequency`/`frequencyField`, `use_weight`/`weight_field` -> `useWeight`/`weightField`, `use_partitioned_data` -> `usePartitionedData` (default true; True = no change), `mode` -> `expertMode`.
- Class `BuildLogisticNode` (native, all settings in XML `properties`). `logistic_procedure` -> `logisticProcedure` (`Multinomial` default | `Binomial`); `include_constant` -> `includeConstant` (true); `method` -> `LogregMethod` (`enter` default, `stepwise`, `forwards`, `backwards`; BackwardsStepwise -> `remove`); `binomial_method` -> `LogregBinomialMethod` (`Enter` default, `Forwards`, `Backwards`; `BackwardsStepwise` rejected); `model_type` -> `modelType` (`mainEffects` default | `fullFactorial` | `custom`); `custom_terms` -> `customEffects/persistentObjectListMember[i]/EffectTerm[j]/@termName`; `multinomial_base_category` -> `multinomialBaseCategory`; `scale` -> `scale` (`none` default, `pearsonScaling`, `devianceScaling`, `userDefined`); `scale_value` -> `scaleValue` (1.0); `all_probabilities` -> `appendProbabilities`; `tolerance` -> `singularityTolerance` (strings kept in `1.0e-8` form, default `1.0e-8`, input `1.0E-5` was written as `1.0e-5`); `min_terms` -> `minTerms` (0); `use_max_terms` -> `LogregEnableMaxTerms`; `entry_criterion`/`removal_criterion` -> `stepwiseEntryMethod` (`Score` default | `LR`) / `stepwiseRemovalMethod` (`LR` default | `Wald`); `probability_entry`/`probability_removal` -> `pIn` (0.05) / `pOut` (0.1); `binomial_probability_*` -> `binomialPIn`/`binomialPOut`; `requirements` -> `LogregSteppingRequirements` (`none`, `hierarchy_all`, `containment_all`, `hierarchy_discrete`; `Containment` -> `containment_all`); `max_iterations` -> `maximumIterations` (20); `max_steps` -> `maximumStepHalving` (5); `p_converge` -> `parameterConvergence` (`1.0e-6`); `l_converge` -> `logLikelihoodConvergence` (`0`); `delta` -> `delta` (0.0).
- Keyed binomial inputs: `binomial_categorical_input = "Sex"` creates `logisticInputsTable/rows/row/{field,contrast,base_category}` (defaults `Indicator`, `First`); `binomial_input_contrast[Sex]` -> `contrast` (Indicator, Simple, Difference, Helmert, Repeated, Polynomial, Deviation); `binomial_input_category[Sex]` -> `base_category` (First | Last).
- Output flags: `iteration_history` `iterationHistory`, `history_steps` `historyIterations` (1), `likelihood_ratio` `likelihoodRatioTests`, `asymptotic_correlation` `asymptoticCorrelation`, `goodness_fit` `goodnessOfFit`, `asymptotic_covariance` `asymptoticCovariance`, `classification_table` `classificationTable`, `monotonicity_measures` `associationTable`, `confidence_interval` `confidenceInterval` (95.0; as a NUMBER; setting `true` is rejected); `summary`, `parameters`, `stepwise_summary`, `info_criteria` and `max_terms` showed no change when set (already default or not persisted). Binomial output: `binomialOutputDisplay` (`at_each_step` default), `binomialGoodnessOfFit`, `binomial_parameters` -> `binomialCorrelationOfEstimates` (name mismatch), `binomialIterationHistory`, `binomialClassificationPlots`, `binomialCIEnable`, `binomialCIValue` (95), `binomialCasewiseEnable`, `binomialCasewiseOutliers`, `binomialCasewiseThreshold` (2.0), `binomialClassificationCutoff` (0.5), `binomialStepwiseRemovalMethod` (LR | Wald | Conditional); `calculate_variable_importance` `calculateVariableImportances`; `calculate_raw_propensities` `calculateRawPropensities`.
