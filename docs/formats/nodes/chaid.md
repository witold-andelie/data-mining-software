# `chaid` node (`CFBuildModelNode`)

Probe `xtask/oracle/probes/settings_chaid.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `custom_fields` | `attachment:role_use` | True -> true | False; file: false | yes |
| `target` | `attachment:tree.TargetField` | Drug -> Drug |  | yes |
| `inputs` | `attachment:tree.InputFieldList` | ['Age', 'Na', 'K'] -> [Age,Na,K] | []; file: [] | yes |
| `use_model_name` | `autoModelName`<br>also: `attachment:modelRename` | True -> false | False; file: true | yes |
| `model_name` | `modelName`<br>also: `attachment:modelName` | CHAID_x -> CHAID_x |  | yes |
| `continue_training_existing_model` | `attachment:objective_choose` | True -> updateExistingModel | False; file: buildNewModel | yes |
| `objective` | `attachment:objective_goal` | Boosting -> boosting<br>Bagging -> bagging<br>psm -> psm<br>Standard -> standard | standard; file: standard | yes |
| `model_output_type` | `attachment:SingleModel` | InteractiveBuilder -> InteractiveBuilder<br>Single -> Single | Single; file: Single | yes |
| `use_tree_directives` | `attachment:use_tree_directives` | True -> true | False; file: false | yes |
| `tree_directives` | `attachment:tree_directives` | Test -> Test |  | yes |
| `method` | `attachment:method` | ExhaustiveChaid -> ExhaustiveChaid<br>Chaid -> Chaid | Chaid; file: Chaid | yes |
| `use_max_depth` | `attachment:MaxTreeDepth` | Custom -> Custom<br>Default -> Default | True; file: Default | yes |
| `max_depth` | `attachment:tree.MaxTreeDepth` | 7 -> 7 | 5; file: 5 | yes |
| `use_percentage` | `attachment:tree.UsePercentageForNodeSizeRequirement` | False -> absolute | True; file: percentage | yes |
| `min_parent_records_pc` | `attachment:tree.MinParentRecordsPct` | 5 -> 5.0 | 2.0; file: 2.0 | yes |
| `min_child_records_pc` | `attachment:tree.MinChildRecordsPct` | 3 -> 3.0 | 1.0; file: 1.0 | yes |
| `min_parent_records_abs` | `attachment:tree.MinParentRecordsABS` | 40 -> 40 | 100; file: 100 | yes |
| `min_child_records_abs` | `attachment:tree.MinChildRecordsABS` | 30 -> 30 | 50; file: 50 | yes |
| `use_costs` | `attachment:tree.UseCosts` | True -> true | False; file: false | yes |
| `costs` | `attachment:tree.Costs` | [['drugA', 'drugB', 2.0]] -> [(trueValue=drugA,predictedValue=dru... | []; file: [] | yes |
| `trails` | `attachment:thread_num` | 15 -> 15 | 10; file: 10 | yes |
| `set_ensemble_method` | `attachment:ensemble_cat_method` | HighestProbability -> HighestProbability<br>HighestMeanProbability -> HighestMeanProbability<br>Voting -> Voting | Voting; file: Voting | yes |
| `range_ensemble_method` | `attachment:ensemble_con_method` | Median -> Median<br>Mean -> Mean | Mean; file: Mean | yes |
| `large_boost` | `attachment:veryLargeData` | True -> true | False; file: false | yes |
| `split_alpha` | `attachment:tree.SplitAlpha` | 0.03 -> 0.03 | 0.05; file: 0.05 | yes |
| `merge_alpha` | `attachment:tree.MergeAlpha` | 0.04 -> 0.04 | 0.05; file: 0.05 | yes |
| `bonferroni_adjustment` | `attachment:tree.BonferroniAdjustment` | False -> false | True; file: true | yes |
| `split_merged_categories` | `attachment:tree.SplitMergedCategories` | True -> true | False; file: false | yes |
| `chi_square` | `attachment:tree.ChiSquareType` | LR -> LR<br>Pearson -> pearson | pearson; file: pearson | yes |
| `epsilon` | `attachment:tree.Epsilon` | 0.003 -> 0.003 | 0.001; file: 0.001 | yes |
| `max_iterations` | `attachment:tree.MaxIterations` | 75 -> 75 | 100; file: 100 | yes |
| `set_random_seed` | (none) | True -> (no change) | True | no |
| `seed` | `attachment:Chaid_RandomSeed` | 4321 -> 4321 | 701499504; file: 701499504 | yes |
| `calculate_variable_importance` | (none) | True -> (no change) | True | no |
| `calculate_raw_propensities` | `attachment:scoring.isRawPropensity` | True -> true | False; file: false | yes |
| `calculate_adjusted_propensities` | `attachment:calculate_adjusted_propensities` | True -> Calc | False; file: NoCalc | yes |
| `adjusted_propensity_partition` | `attachment:adjusted_propensity_partition` | Validation -> Validation<br>Test -> Test | Test; file: Test | yes |

## Set failed

- `maximum_number_of_models` = 5: The property 'maximum_number_of_models' is read-only

## Set succeeded but file unchanged

- `set_random_seed`
- `calculate_variable_importance`

## Observations

- Class `CFBuildModelNode` (extension-framework node: `@extensionID`, `@extensionObjectID`, `@properties="data/NNNN.dat"`). Most algorithm settings are NOT in the stream XML: they are `<Parameter name=... value=.../>` entries in the attachment (written below as `attachment:<name>`; list values are `<ListValue>` with `<Value value=.../>` or `<StructuredValue><Attribute name value/>`). The attachment is a plain XML text member (`Parameters` document in the `http://xml.spss.com/pasw/extension` namespace) holding the parameters of all extension algorithms (neural net, tree, boosting, scoring...), so unrelated names appear. Properties that only exist in the XML (`modelName`, `autoModelName`, `autoModelFieldSelection`, annotation fields, `usePartitionedData`, `useWeight`...) are listed without the `attachment:` prefix.
- Some common properties are read-only on extension nodes: `partition`, `use_split_data`, `frequency_field`, `use_weight` (set fails with "read-only"). `splits` and `weight_field` work (attachment `split_distribute.split_fields`, `tree.RegressionWeightField`).
- Mapping: as for C&R Tree for the shared options (`role_use`, `tree.TargetField`, `tree.InputFieldList`, `objective_goal`, `SingleModel`, `MaxTreeDepth`/`tree.MaxTreeDepth`, `tree.*Records*`, `thread_num`, `ensemble_*`, `veryLargeData`, `Chaid_RandomSeed` for `seed`). CHAID specific: `method` -> `method` (`Chaid` default | `ExhaustiveChaid`); `split_alpha` `tree.SplitAlpha` (0.05); `merge_alpha` `tree.MergeAlpha` (0.05); `bonferroni_adjustment` `tree.BonferroniAdjustment` (true); `split_merged_categories` `tree.SplitMergedCategories` (false); `chi_square` `tree.ChiSquareType` (`pearson` default for Pearson, `LR` stays upper case); `epsilon` `tree.Epsilon` (0.001); `max_iterations` `tree.MaxIterations` (100).
- `maximum_number_of_models` is read-only; `use_costs`, `costs`, `set_random_seed`, `calculate_variable_importance` as for C&R Tree.
