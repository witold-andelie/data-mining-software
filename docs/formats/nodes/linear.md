# `linear` node (`CFBuildModelNode`)

Probe `xtask/oracle/probes/settings_linear.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `custom_fields` | `attachment:role_use` | True -> true | False; file: false | yes |
| `target` | `attachment:linreg.TargetField` | Na -> Na |  | yes |
| `inputs` | `attachment:linreg.InputFieldList` | ['Age', 'K'] -> [Age,K] | []; file: [] | yes |
| `use_model_name` | `autoModelName` | True -> false | False; file: true | yes |
| `model_name` | `modelName` | Lin_x -> Lin_x |  | yes |
| `use_partitioned_data` | (none) | True -> (no change) | True | no |
| `continue_training_existing_model` | `attachment:objective_choose` | True -> updateExistingModel | False; file: buildNewModel | yes |
| `objective` | `attachment:objective_goal` | Bagging -> bagging<br>Boosting -> boosting<br>psm -> psm<br>Standard -> standard | standard; file: standard | yes |
| `use_auto_data_preparation` | `attachment:linreg.AutoDataPreparation` | False -> false | True; file: true | yes |
| `confidence_level` | `attachment:linreg.CIn` | 90 -> 90.0<br>150 -> 100.0 | 95.0; file: 95.0 | yes |
| `model_selection` | `attachment:linreg.VarSelectionMethod` | BestSubsets -> bestSubsets<br>None -> none<br>ForwardStepwise -> forwardStepwise | forwardStepwise; file: forwardStepwise | yes |
| `criteria_forward_stepwise` | `attachment:linreg.CriteriaForForwardStepwise` | Fstatistics -> FStatistics<br>AdjustedRSquare -> adjustedRSquare<br>ASE -> ASE<br>AICC -> AICC | AICC; file: AICC | yes |
| `probability_entry` | `attachment:linreg.PIn` | 0.07 -> 0.07 | 0.05; file: 0.05 | yes |
| `probability_removal` | `attachment:linreg.POut` | 0.12 -> 0.12 | 0.1; file: 0.1 | yes |
| `use_max_effects` | `attachment:linreg.UseCustomMaxEffects` | True -> true | False; file: false | yes |
| `max_effects` | `attachment:linreg.MaxEffects` | 5 -> 5 | ; file: [] | yes |
| `use_max_steps` | `attachment:linreg.UseCustomMaxSteps` | True -> true | False; file: false | yes |
| `max_steps` | `attachment:linreg.MaxSteps` | 9 -> 9 | ; file: [] | yes |
| `criteria_best_subsets` | `attachment:linreg.CriteriaForBestSubsets` | AdjustedRSquare -> adjustedRSquare<br>ASE -> ASE<br>AICC -> AICC | AICC; file: AICC | yes |
| `combining_rule_continuous` | `attachment:ensemble_con_method` | Median -> Median | Mean; file: Mean | yes |
| `combining_rule_categorical` | `attachment:ensemble_cat_method` | HighestProbability -> HighestProbability | Voting; file: Voting | yes |
| `component_models_n` | `attachment:thread_num` | 7 -> 7 | 10; file: 10 | yes |
| `use_random_seed` | (none) | True -> (no change) | True | no |
| `random_seed` | `attachment:linreg.RandomSeed` | 777 -> 777 | 54752075; file: 54752075 | yes |
| `use_custom_model_name` | `attachment:modelRename` | True -> true | False; file: false | yes |
| `custom_model_name` | `modelName`<br>also: `attachment:modelName` | CMN -> CMN | ; file: Lin_x | yes |
| `use_custom_name` | `autoLabel`<br>also: `attachment:pasw.annotation_autoLabel` | True -> false | False; file: true | yes |
| `custom_name` | `label`<br>also: `attachment:pasw.annotation_label` | CN -> CN |  | yes |
| `tooltip` | `toolTipText`<br>also: `attachment:pasw.annotation_toolTipText` | tt -> tt |  | yes |
| `keywords` | `attachment:pasw.annotation_keywords` | k1 -> k1 |  | yes |
| `annotation` | `annotation`<br>also: `attachment:pasw.annotation_description` | an -> an |  | yes |

## Set failed

- `partition` = Test: The property 'partition' is read-only

## Set succeeded but file unchanged

- `use_partitioned_data`
- `use_random_seed`

## Observations

- Class `CFBuildModelNode` (extension-framework node: `@extensionID`, `@extensionObjectID`, `@properties="data/NNNN.dat"`). Most algorithm settings are NOT in the stream XML: they are `<Parameter name=... value=.../>` entries in the attachment (written below as `attachment:<name>`; list values are `<ListValue>` with `<Value value=.../>` or `<StructuredValue><Attribute name value/>`). The attachment is a plain XML text member (`Parameters` document in the `http://xml.spss.com/pasw/extension` namespace) holding the parameters of all extension algorithms (neural net, tree, boosting, scoring...), so unrelated names appear. Properties that only exist in the XML (`modelName`, `autoModelName`, `autoModelFieldSelection`, annotation fields, `usePartitionedData`, `useWeight`...) are listed without the `attachment:` prefix.
- Some common properties are read-only on extension nodes: `partition`, `use_split_data`, `frequency_field`, `use_weight` (set fails with "read-only"). `splits` and `weight_field` work (attachment `split_distribute.split_fields`, `tree.RegressionWeightField`).
- Mapping (attachment): `role_use`; `linreg.TargetField`; `linreg.InputFieldList`; `objective_choose`; `objective_goal`; `linreg.AutoDataPreparation` (true); `linreg.CIn` (95.0; 150 clamps to 100.0); `linreg.VarSelectionMethod` (`forwardStepwise` default | `bestSubsets` | `none`); `linreg.CriteriaForForwardStepwise` (`AICC` default, `FStatistics`, `adjustedRSquare`, `ASE`); `linreg.PIn` (0.05); `linreg.POut` (0.1); `linreg.UseCustomMaxEffects` + `linreg.MaxEffects`; `linreg.UseCustomMaxSteps` + `linreg.MaxSteps`; `linreg.CriteriaForBestSubsets` (`AICC` default, `adjustedRSquare`, `ASE`); `ensemble_con_method`/`ensemble_cat_method`; `thread_num`; `linreg.RandomSeed` (random default).
- Note the enum spellings differ from the scripting values: `Fstatistics` -> `FStatistics`, `AdjustedRSquare` -> `adjustedRSquare`. Annotation properties (`custom_name`, `tooltip`, `keywords`, `annotation`) are duplicated into the attachment as `pasw.annotation_label` / `_toolTipText` / `_keywords` / `_description`, and `custom_model_name` -> `modelName` (XML and attachment) with `modelRename`. `use_random_seed` and `use_partitioned_data` showed no change in the file.
