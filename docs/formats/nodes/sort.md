# `sort` node (`SortNode`)

Probe `xtask/oracle/probes/settings_sort.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `keys` | `sortKeys/rows/row/field`<br>also: `sortKeys/rows/row/direction`, `sortKeys/rows/row[$field]/field`, `sortKeys/rows/row[$field]/direction` | [['Age', 'Ascending']] -> Age<br>[['Age', 'Ascending'], ['Sex', 'Descendi... -> (removed)<br>[['Drug', 'Descending']] -> Drug<br>[] -> (removed) | []; file: absent | yes |
| `default_ascending` | `sortDefaultOrder` | False -> false | True; file: true | yes |
| `use_existing_keys` | `useExistingSortOrder` | True -> true | False; file: false | yes |
| `existing_keys` | `existingSortKeys/rows/row/field`<br>also: `existingSortKeys/rows/row/direction`, `existingSortKeys/rows/row[$field]/field`, `existingSortKeys/rows/row[$field]/direction` | [['Age', 'Ascending']] -> Age<br>[['Age', 'Descending'], ['Sex', 'Ascendi... -> (removed) | []; file: absent | yes |

## Set failed

- `keys` = ['Age']: The value '[Age]' is not valid for the property 'keys'

## Set succeeded but file unchanged

(none)

## Observations

- Class `SortNode`. `keys` -> `sortKeys/rows/row[i]/{field,direction}` with direction `0` = Ascending, `1` = Descending (a single row is unindexed; an empty list gives an empty `rows`). A bare field name in the list (`["Age"]`) is rejected: every key must be a `[field, direction]` pair. `default_ascending` -> `sortDefaultOrder` (default true); `use_existing_keys` -> `useExistingSortOrder`; `existing_keys` -> `existingSortKeys/rows/row[i]/{field,direction}` (same encoding).
