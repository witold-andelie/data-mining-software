# `kmeans` node (`BuildKMeansNode`)

Probe `xtask/oracle/probes/settings_kmeans.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `change: instantiate fields by running a temporary Table node on the source` | (none) |  -> (no change) |  | no |
| `custom_fields` | `autoModelFieldSelection` | True -> false | False; file: true | yes |
| `inputs` | `modelInputFieldSelection/field[0]`<br>also: `modelInputFieldSelection/field[1]`, `modelInputFieldSelection/field[2]`, `modelInputFieldSelection` | ['Age', 'Na', 'K'] -> [Age, Na, K]<br>['Age'] -> (removed) | []; file: absent | yes |
| `use_model_name` | `autoModelName` | True -> false | False; file: true | yes |
| `model_name` | `modelName` | Kmeans_x -> Kmeans_x |  | yes |
| `partition` | `partitionField` | Test -> Test |  | yes |
| `use_partitioned_data` | (none) | True -> (no change) | True | no |
| `use_frequency` | `useFrequency` | True -> true | False; file: false | yes |
| `frequency_field` | `frequencyField` | Age -> Age |  | yes |
| `use_weight` | `useWeight` | True -> true | False; file: false | yes |
| `weight_field` | `weightField` | Age -> Age |  | yes |
| `num_clusters` | `numClusters` | 9 -> 9<br>0 -> 1 | 5; file: 5 | yes |
| `gen_distance` | `generateDistanceField` | True -> true | False; file: false | yes |
| `cluster_label` | `clusterLabelType` | Number -> NUMBER<br>String -> STRING | String; file: STRING | yes |
| `label_prefix` | `clusterLabel` | KM_ -> KM_ | cluster; file: cluster | yes |
| `optimize` | `optimization` | Memory -> (no change)<br>Speed -> speed | Memory | yes |
| `mode` | `expertMode` | Expert -> true<br>Simple -> false | Simple; file: false | yes |
| `stop_on` | `stoppingCondition` | Custom -> CUSTOM<br>Default -> DEFAULT | Default; file: DEFAULT | yes |
| `max_iterations` | `numberIterations` | 33 -> 33<br>0 -> 0 | 20; file: 20 | yes |
| `tolerance` | `changeTolerance` | 0.05 -> 0.05<br>-1.0 -> -1.0 | 1e-06; file: 1.0E-6 | yes |
| `encoding_value` | `encodingValueForSets` | 0.7 -> 0.7<br>5.0 -> 5.0 | 0.707107; file: 0.707107 | yes |

## Set failed

- `use_split_data` = True: The property 'use_split_data' is undefined
- `splits` = ['Sex']: The property 'splits' is undefined

## Set succeeded but file unchanged

- `change: instantiate fields by running a temporary Table node on the source`
- `use_partitioned_data`

## Observations

- Common modeling properties in the XML `properties` (not the attachment): `custom_fields` -> `autoModelFieldSelection` (inverse: True -> `false`), `target` -> `modelTargetFieldSelection`, `inputs` -> `modelInputFieldSelection/field[n]` (single input is unindexed), `partition` -> `partitionField`, `use_model_name` -> `autoModelName` (inverse), `model_name` -> `modelName`, `use_frequency`/`frequency_field` -> `useFrequency`/`frequencyField`, `use_weight`/`weight_field` -> `useWeight`/`weightField`, `use_partitioned_data` -> `usePartitionedData` (default true; True = no change), `mode` -> `expertMode`.
- Class `BuildKMeansNode` (a native node: all settings are XML elements in `properties`). `use_split_data` / `splits` are undefined on this node. `num_clusters` -> `numClusters` (default 5; 0 clamps to 1); `gen_distance` -> `generateDistanceField`; `cluster_label` -> `clusterLabelType` (`STRING` default | `NUMBER`, upper case); `label_prefix` -> `clusterLabel` (default `cluster`); `optimize` -> `optimization` (`memory` default | `speed`); `stop_on` -> `stoppingCondition` (`DEFAULT` | `CUSTOM`); `max_iterations` -> `numberIterations` (default 20; 0 accepted); `tolerance` -> `changeTolerance` (default 1.0E-6; negative accepted, no clamping); `encoding_value` -> `encodingValueForSets` (default 0.707107; 5.0 accepted).
