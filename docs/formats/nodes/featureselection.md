# `featureselection` node (`BuildFeatureSelectionNode`)

Probe `xtask/oracle/probes/settings_featureselection.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `custom_fields` | `autoModelFieldSelection` | True -> false | False; file: true | yes |
| `target` | `modelTargetFieldSelection` | Drug -> Drug |  | yes |
| `inputs` | `modelInputFieldSelection`<br>also: `modelInputFieldSelection/field[0]`, `modelInputFieldSelection/field[1]`, `modelInputFieldSelection/field[2]` | ['Age', 'Na', 'K'] -> (removed) | [] | yes |
| `use_model_name` | `autoModelName` | True -> false | False; file: true | yes |
| `model_name` | `modelName` | FS_x -> FS_x |  | yes |
| `screen_single_category` | (none) | True -> (no change) | True | no |
| `max_single_category` | `maxSingleCategory` | 80 -> 80.0 | 90.0; file: 90.0 | yes |
| `screen_missing_values` | (none) | True -> (no change) | True | no |
| `max_missing_values` | (none) | 70 -> (no change) | 70.0 | no |
| `screen_num_categories` | (none) | True -> (no change) | True | no |
| `max_num_categories` | `maxCategoriesAsCases` | 33 -> 33.0 | 95.0; file: 95.0 | yes |
| `screen_std_dev` | (none) | True -> (no change) | True | no |
| `min_std_dev` | `minStdDev` | 0.2 -> 0.2 | 0.0; file: 0.0 | yes |
| `screen_coeff_of_var` | (none) | True -> (no change) | True | no |
| `min_coeff_of_var` | `minVariability` | 0.3 -> 0.3 | 0.1; file: 0.1 | yes |
| `criteria` | `criteria` | Likelihood -> likelihood<br>CramersV -> cramersV<br>Lambda -> lambda<br>Pearson -> pearson | Pearson; file: pearson | yes |
| `unimportant_below` | `importanceLowThreshold` | 0.8 -> 0.8 | 0.9; file: 0.9 | yes |
| `important_above` | `importanceMediumThreshold` | 0.9 -> 0.9 | 0.95; file: 0.95 | yes |
| `unimportant_label` | `importanceLowLabel` | Nope -> Nope | Unimportant; file: Unimportant | yes |
| `marginal_label` | `importanceMediumLabel` | Maybe -> Maybe | Marginal; file: Marginal | yes |
| `important_label` | `importanceHighLabel` | Yes -> Yes | Important; file: Important | yes |
| `selection_mode` | `fieldSelectionMode` | ImportanceValue -> importanceValue<br>TopN -> topN<br>ImportanceLevel -> importanceLevel | ImportanceLevel; file: importanceLevel | yes |
| `select_important` | `selectImportantFields` | False -> false | True; file: true | yes |
| `select_marginal` | `selectMarginalFields` | True -> true | False; file: false | yes |
| `select_unimportant` | `selectUnimportantFields` | True -> true | False; file: false | yes |
| `importance_value` | `importanceValue` | 55.5 -> 55.5 | 0.95; file: 0.95 | yes |
| `top_n` | `topNFields` | 15 -> 15<br>5000 -> 1000 | 10; file: 10 | yes |

## Set failed

(none)

## Set succeeded but file unchanged

- `screen_single_category`
- `screen_missing_values`
- `max_missing_values`
- `screen_num_categories`
- `screen_std_dev`
- `screen_coeff_of_var`

## Observations

- Common modeling properties in the XML `properties` (not the attachment): `custom_fields` -> `autoModelFieldSelection` (inverse: True -> `false`), `target` -> `modelTargetFieldSelection`, `inputs` -> `modelInputFieldSelection/field[n]` (single input is unindexed), `partition` -> `partitionField`, `use_model_name` -> `autoModelName` (inverse), `model_name` -> `modelName`, `use_frequency`/`frequency_field` -> `useFrequency`/`frequencyField`, `use_weight`/`weight_field` -> `useWeight`/`weightField`, `use_partitioned_data` -> `usePartitionedData` (default true; True = no change), `mode` -> `expertMode`.
- Class `BuildFeatureSelectionNode` (native). `max_single_category` -> `maxSingleCategory` (90.0); `max_num_categories` -> `maxCategoriesAsCases` (95.0); `min_std_dev` -> `minStdDev` (0.0); `min_coeff_of_var` -> `minVariability` (0.1); the `screen_*` flags and `max_missing_values` did not change the file when set (flag elements not identified; probably already true by default or stored elsewhere); `criteria` -> `criteria` (`pearson` default, `likelihood`, `cramersV`, `lambda`); `unimportant_below` -> `importanceLowThreshold` (0.9); `important_above` -> `importanceMediumThreshold` (0.95); labels -> `importanceLowLabel`/`importanceMediumLabel`/`importanceHighLabel` (Unimportant/Marginal/Important); `selection_mode` -> `fieldSelectionMode` (`importanceLevel` default | `importanceValue` | `topN`); `select_important`/`select_marginal`/`select_unimportant` -> `selectImportantFields` (true)/`selectMarginalFields` (false)/`selectUnimportantFields` (false); `importance_value` -> `importanceValue` (0.95); `top_n` -> `topNFields` (10; 5000 clamps to 1000).
