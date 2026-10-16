# `partition` node (`PartitionNode`)

Probe `xtask/oracle/probes/settings_partition.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `new_name` | `partitionFieldName` | MyPart -> MyPart | Partition; file: Partition | yes |
| `create_validation` | `createValidationPartition` | True -> true<br>False -> false | False; file: false | yes |
| `training_size` | `trainingPartitionSize` | 40 -> 40<br>150 -> 100<br>-5 -> 0 | 50; file: 50 | yes |
| `testing_size` | `testingPartitionSize` | 30 -> 30 | 50; file: 50 | yes |
| `validation_size` | `validationPartitionSize` | 20 -> 20 | 0; file: 0 | yes |
| `training_label` | `trainingPartitionLabel` | Train -> Train | Training; file: Training | yes |
| `testing_label` | `testingPartitionLabel` | Test -> Test | Testing; file: Testing | yes |
| `validation_label` | `validationPartitionLabel` | Valid -> Valid | Validation; file: Validation | yes |
| `value_mode` | `partitionValueMode` | SystemAndLabel -> (no change)<br>Label -> label<br>System -> system | SystemAndLabel | yes |
| `set_random_seed` | (none) | True -> (no change) | True | no |
| `random_seed` | `partitionRandomSeed` | 123 -> 123<br>0 -> 0<br>2147483647 -> 2147483647<br>-1 -> 0 | 1234567; file: 1234567 | yes |
| `enable_sql_generation` | `usePartitionHashColumn` | True -> true | False; file: false | yes |
| `unique_field` | `partitionHashColumn` | Age -> Age |  | yes |

## Set failed

(none)

## Set succeeded but file unchanged

- `set_random_seed`

## Observations

- Class `PartitionNode`. Defaults: `partitionFieldName` `Partition`, training 50 / testing 50 / validation 0, `createValidationPartition` false, labels `Training`/`Testing`/`Validation`, `partitionValueMode` `system_label` (SystemAndLabel; Label=`label`, System=`system`), `set_random_seed` true by default (not stored as a separate flag change when set True), `partitionRandomSeed` 1234567.
- Clamping: sizes are clamped to 0..100 (150 -> 100, -5 -> 0); `random_seed=-1` clamps to 0; 2147483647 accepted. Sizes are not checked for summing to 100.
- `enable_sql_generation` -> `usePartitionHashColumn`; `unique_field` -> `partitionHashColumn`.
- `set_random_seed=True` produced no change because it is already the default; a False setting was not probed.
