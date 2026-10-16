# `balance` node (`BalanceNode`)

Probe `xtask/oracle/probes/settings_balance.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `training_data_only` | `balanceTrainingPartitionOnly` | True -> (no change)<br>False -> false | True | yes |
| `directives` | `balanceDirectives/entries/entry/factor`<br>also: `balanceDirectives/entries/entry/condition`, `balanceDirectives/entries`, `balanceDirectives/entries/entry[0]/factor` | [[1.3, 'Age > 60']] -> 1.3<br>[[1.3, 'Age > 60'], [1.5, 'Na > 0.5']] -> (removed)<br>[[0.5, 'Drug = "drugY"']] -> 0.5<br>[] -> (removed) | []; file: absent | yes |

## Set failed

(none)

## Set succeeded but file unchanged

(none)

## Observations

- Class `BalanceNode`. `training_data_only` -> `balanceTrainingPartitionOnly` (default true). `directives` -> `balanceDirectives/entries/entry[n]/{factor,condition}` (factor as real, condition verbatim; single entry has no index; empty list gives empty `entries`).
