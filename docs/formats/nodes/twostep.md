# `twostep` node (`BuildTwoStepNode`)

Probe `xtask/oracle/probes/settings_twostep.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `custom_fields` | `autoModelFieldSelection` | True -> false | False; file: true | yes |
| `inputs` | `modelInputFieldSelection`<br>also: `modelInputFieldSelection/field[0]`, `modelInputFieldSelection/field[1]`, `modelInputFieldSelection/field[2]` | ['Age', 'Na', 'K'] -> (removed) | [] | yes |
| `use_model_name` | `autoModelName` | True -> false | False; file: true | yes |
| `model_name` | `modelName` | TwoStep_x -> TwoStep_x |  | yes |
| `standardize` | `standardizeFields` | False -> false | True; file: true | yes |
| `exclude_outliers` | `excludeOutliers` | True -> true | False; file: false | yes |
| `percentage` | `noiseDef` | 12.5 -> 12<br>150 -> 100 | 25; file: 25 | yes |
| `cluster_num_auto` | `autoCluster` | False -> false | True; file: true | yes |
| `min_num_clusters` | `minClusters` | 4 -> 4 | 2; file: 2 | yes |
| `max_num_clusters` | `maxClusters` | 12 -> 12 | 15; file: 15 | yes |
| `num_clusters` | `numClusters` | 7 -> 7 | 10; file: 10 | yes |
| `cluster_label` | `clusterLabelType` | String -> (no change)<br>Number -> NUMBER | String | yes |
| `label_prefix` | `clusterLabel` | TS_ -> TS_ | cluster; file: cluster | yes |
| `distance_measure` | `distanceMeasure` | Loglikelihood -> (no change)<br>Euclidean -> Euclidean | loglikelihood | yes |
| `clustering_criterion` | `clusteringCriterion` | AIC -> AIC<br>BIC -> BIC | BIC; file: BIC | yes |

## Set failed

(none)

## Set succeeded but file unchanged

(none)

## Observations

- Common modeling properties in the XML `properties` (not the attachment): `custom_fields` -> `autoModelFieldSelection` (inverse: True -> `false`), `target` -> `modelTargetFieldSelection`, `inputs` -> `modelInputFieldSelection/field[n]` (single input is unindexed), `partition` -> `partitionField`, `use_model_name` -> `autoModelName` (inverse), `model_name` -> `modelName`, `use_frequency`/`frequency_field` -> `useFrequency`/`frequencyField`, `use_weight`/`weight_field` -> `useWeight`/`weightField`, `use_partitioned_data` -> `usePartitionedData` (default true; True = no change), `mode` -> `expertMode`.
- Class `BuildTwoStepNode` (native). `standardize` -> `standardizeFields` (default true); `exclude_outliers` -> `excludeOutliers`; `percentage` -> `noiseDef` stored as an INTEGER (default 25; 12.5 -> 12; 150 clamps to 100); `cluster_num_auto` -> `autoCluster` (default true); `min_num_clusters`/`max_num_clusters`/`num_clusters` -> `minClusters` (2) / `maxClusters` (15) / `numClusters` (10); `cluster_label` -> `clusterLabelType` (`STRING` default | `NUMBER`); `label_prefix` -> `clusterLabel`; `distance_measure` -> `distanceMeasure` (`Loglikelihood` default | `Euclidean`); `clustering_criterion` -> `clusteringCriterion` (`BIC` default | `AIC`).
