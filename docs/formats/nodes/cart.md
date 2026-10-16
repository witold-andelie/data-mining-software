# `cart` node (`CFBuildModelNode`)

Probe `xtask/oracle/probes/settings_cart.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `custom_fields` | `attachment:role_use` | True -> true | False; file: false | yes |
| `target` | `attachment:tree.TargetField` | Drug -> Drug |  | yes |
| `inputs` | `attachment:tree.InputFieldList` | ['Age', 'BP', 'Cholesterol'] -> [Age,BP,Cholesterol] | []; file: [] | yes |
| `use_model_name` | `autoModelName`<br>also: `attachment:modelRename` | True -> false | False; file: true | yes |
| `model_name` | `modelName`<br>also: `attachment:modelName` | Cart_x -> Cart_x |  | yes |
| `use_partitioned_data` | (none) | True -> (no change) | True | no |
| `splits` | `attachment:split_distribute.split_fields` | ['Sex'] -> [Sex] | []; file: [] | yes |
| `use_frequency` | (none) | True -> (no change) | True | no |
| `weight_field` | `attachment:tree.RegressionWeightField` | Na -> Na |  | yes |
| `continue_training_existing_model` | `attachment:objective_choose` | True -> updateExistingModel | False; file: buildNewModel | yes |
| `objective` | `attachment:objective_goal` | Boosting -> boosting<br>Bagging -> bagging<br>psm -> psm<br>Standard -> standard | standard; file: standard | yes |
| `model_output_type` | `attachment:SingleModel` | InteractiveBuilder -> InteractiveBuilder<br>Single -> Single | Single; file: Single | yes |
| `use_tree_directives` | `attachment:use_tree_directives` | True -> true | False; file: false | yes |
| `tree_directives` | `attachment:tree_directives` | Grow Node Index 0 Children 1 2 -> Grow Node Index 0 Children 1 2 |  | yes |
| `use_max_depth` | `attachment:MaxTreeDepth` | Custom -> Custom<br>Default -> Default | True; file: Default | yes |
| `max_depth` | `attachment:tree.MaxTreeDepth` | 7 -> 7<br>5000 -> 1000 | 5; file: 5 | yes |
| `prune_tree` | `attachment:tree.PruneTree` | False -> false | True; file: true | yes |
| `use_std_err` | (none) | False -> (no change) | False | no |
| `std_err_multiplier` | `attachment:tree.StdErrMultiplier` | 3.0 -> 3.0 | 1.0; file: 1.0 | yes |
| `max_surrogates` | `attachment:tree.MaxSurrogates` | 7 -> 7 | 5; file: 5 | yes |
| `use_percentage` | `attachment:tree.UsePercentageForNodeSizeRequirement` | False -> absolute | True; file: percentage | yes |
| `min_parent_records_pc` | `attachment:tree.MinParentRecordsPct` | 5 -> 5.0 | 2.0; file: 2.0 | yes |
| `min_child_records_pc` | `attachment:tree.MinChildRecordsPct` | 3 -> 3.0 | 1.0; file: 1.0 | yes |
| `min_parent_records_abs` | `attachment:tree.MinParentRecordsABS` | 50 -> 50 | 100; file: 100 | yes |
| `min_child_records_abs` | `attachment:tree.MinChildRecordsABS` | 20 -> 20 | 50; file: 50 | yes |
| `use_costs` | `attachment:tree.UseCosts` | True -> true | False; file: false | yes |
| `costs` | `attachment:tree.Costs` | [['drugA', 'drugB', 2.0]] -> [(trueValue=drugA,predictedValue=dru... | []; file: [] | yes |
| `priors` | `attachment:tree.Priors` | Equal -> Equal<br>Custom -> Custom<br>Data -> Data | Data; file: Data | yes |
| `custom_priors` | `attachment:tree.CustomPriors` | [['drugA', 0.2], ['drugB', 0.8]] -> [(value=drugA,probability=0.2),(valu... | []; file: [] | yes |
| `adjust_priors` | `attachment:tree.AdjustPriors` | True -> true | False; file: false | yes |
| `trails` | `attachment:thread_num` | 15 -> 15 | 10; file: 10 | yes |
| `set_ensemble_method` | `attachment:ensemble_cat_method` | HighestProbability -> HighestProbability<br>HighestMeanProbability -> HighestMeanProbability<br>Voting -> Voting | Voting; file: Voting | yes |
| `range_ensemble_method` | `attachment:ensemble_con_method` | Median -> Median<br>Mean -> Mean | Mean; file: Mean | yes |
| `large_boost` | `attachment:veryLargeData` | True -> true | False; file: false | yes |
| `min_impurity` | `attachment:tree.MinImpurity` | 0.0003 -> 3.0E-4 | 0.0001; file: 1.0E-4 | yes |
| `impurity_measure` | `attachment:tree.ImpurityMeasure` | Twoing -> Twoing<br>Ordered -> Ordered<br>Gini -> Gini | Gini; file: Gini | yes |
| `train_pct` | `attachment:tree.OverFitPreventionSetPct` | 33 -> 33.0 | 30.0; file: 30.0 | yes |
| `set_random_seed` | (none) | True -> (no change) | True | no |
| `seed` | `attachment:Cart_RandomSeed` | 4321 -> 4321 | 681644031; file: 681644031 | yes |
| `calculate_variable_importance` | (none) | True -> (no change) | True | no |
| `calculate_raw_propensities` | `attachment:scoring.isRawPropensity` | True -> true | False; file: false | yes |
| `calculate_adjusted_propensities` | `attachment:calculate_adjusted_propensities` | True -> Calc | False; file: NoCalc | yes |
| `adjusted_propensity_partition` | `attachment:adjusted_propensity_partition` | Validation -> Validation<br>Test -> Test | Test; file: Test | yes |

## Set failed

- `partition` = Test: The property 'partition' is read-only
- `use_split_data` = True: The property 'use_split_data' is read-only
- `frequency_field` = Age: The property 'frequency_field' is read-only
- `use_weight` = True: The property 'use_weight' is read-only

## Set succeeded but file unchanged

- `use_partitioned_data`
- `use_frequency`
- `use_std_err`
- `set_random_seed`
- `calculate_variable_importance`

## Observations

- Class `CFBuildModelNode` (extension-framework node: `@extensionID`, `@extensionObjectID`, `@properties="data/NNNN.dat"`). Most algorithm settings are NOT in the stream XML: they are `<Parameter name=... value=.../>` entries in the attachment (written below as `attachment:<name>`; list values are `<ListValue>` with `<Value value=.../>` or `<StructuredValue><Attribute name value/>`). The attachment is a plain XML text member (`Parameters` document in the `http://xml.spss.com/pasw/extension` namespace) holding the parameters of all extension algorithms (neural net, tree, boosting, scoring...), so unrelated names appear. Properties that only exist in the XML (`modelName`, `autoModelName`, `autoModelFieldSelection`, annotation fields, `usePartitionedData`, `useWeight`...) are listed without the `attachment:` prefix.
- Some common properties are read-only on extension nodes: `partition`, `use_split_data`, `frequency_field`, `use_weight` (set fails with "read-only"). `splits` and `weight_field` work (attachment `split_distribute.split_fields`, `tree.RegressionWeightField`).
- Mapping (attachment parameter names): `custom_fields` `role_use` (true = custom); `target` `tree.TargetField`; `inputs` `tree.InputFieldList` (list); `continue_training_existing_model` `objective_choose` (`buildNewModel` | `updateExistingModel`); `objective` `objective_goal` (`standard`, `boosting`, `bagging`, `psm` lower case); `model_output_type` `SingleModel` (`Single` | `InteractiveBuilder`); `use_tree_directives`/`tree_directives` same names; `use_max_depth` `MaxTreeDepth` (`Default`|`Custom`); `max_depth` `tree.MaxTreeDepth` (default 5; 5000 clamps to 1000); `prune_tree` `tree.PruneTree`; `std_err_multiplier` `tree.StdErrMultiplier` (1.0); `max_surrogates` `tree.MaxSurrogates` (5); `use_percentage` `tree.UsePercentageForNodeSizeRequirement` (`percentage` | `absolute`); `min_parent_records_pc`/`min_child_records_pc` `tree.MinParentRecordsPct`/`MinChildRecordsPct` (2.0/1.0); `min_parent_records_abs`/`min_child_records_abs` `tree.MinParentRecordsABS`/`MinChildRecordsABS` (100/50); `use_costs` `tree.UseCosts`; `costs` `tree.Costs` (structured: `trueValue`, `predictedValue`, `cost`); `priors` `tree.Priors` (`Data` default | `Equal` | `Custom`); `custom_priors` `tree.CustomPriors` (structured `value`, `probability`); `adjust_priors` `tree.AdjustPriors`; `trails` `thread_num` (10); `set_ensemble_method` `ensemble_cat_method`; `range_ensemble_method` `ensemble_con_method`; `large_boost` `veryLargeData`; `min_impurity` `tree.MinImpurity` (1.0E-4); `impurity_measure` `tree.ImpurityMeasure` (`Gini` default); `train_pct` `tree.OverFitPreventionSetPct` (30.0, stored as real); `seed` `Cart_RandomSeed`; `calculate_raw_propensities` `scoring.isRawPropensity`; `calculate_adjusted_propensities` `calculate_adjusted_propensities` (`NoCalc` | `Calc`); `adjusted_propensity_partition` `adjusted_propensity_partition` (`Test` | `Validation`); `weight_field` `tree.RegressionWeightField`; `splits` `split_distribute.split_fields`.
- `use_std_err`, `set_random_seed`, `use_frequency`, `use_partitioned_data`, `calculate_variable_importance` showed no change in the file (their attachment key was not identified or equal to default); treat as unresolved.
