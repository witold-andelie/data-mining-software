# `merge` node (`MergeNode`)

Probe `xtask/oracle/probes/settings_merge.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `change: instantiate fields by running a temporary Table node on the source` | (none) |  -> (no change) |  | no |
| `method` | `mergeMode` | Keys -> key<br>Condition -> condition<br>Order -> order | Order; file: order | yes |
| `condition` | `mergeCondition` | Age > 30 -> Age > 30 |  | yes |
| `key_fields` | `keyFields`<br>also: `keyFields/field`, `keyFields/field[0]`, `keyFields/field[1]` | ['Drug'] -> (removed)<br>['Drug', 'Sex'] -> keyFields/field=(removed) | [] | yes |
| `common_keys` | `mergeCommonKeys` | False -> false | True; file: true | yes |
| `join` | `joinType` | FullOuter -> fullOuter<br>PartialOuter -> partialOuter<br>Anti -> anti<br>Inner -> inner | Inner; file: inner | yes |
| `outer_join_tag[field]` | `includeIncompleteRecords/includeIncompleteRecordsList/includeIncompleteRecords/inputNodeTagName`<br>also: `includeIncompleteRecords/includeIncompleteRecordsList`, `includeIncompleteRecords/includeIncompleteRecordsList/includ...`, `includeIncompleteRecords/includeIncompleteRecordsList/includ...` | True -> 1<br>True -> (removed) | False; file: absent | yes |
| `single_large_input` | `mergeSingleLargeInput` | True -> true | False; file: false | yes |
| `single_large_input_tag` | `mergeSingleLargeInputTag` | 2 -> 2 |  | yes |
| `use_existing_sort_keys` | `mergePreSortedInput` | True -> true | False; file: false | yes |
| `existing_sort_keys` | `existingSortKeys/rows/row[$field]/field`<br>also: `existingSortKeys/rows/row[$field]/direction`, `existingSortKeys/rows` | [['Drug', 'Ascending'], ['Age', 'Descend... -> Age | []; file: absent | yes |
| `primary_dataset` | (none) | 1 -> (no change) | 1 | no |
| `rename_duplicate_fields` | (none) | True -> (no change) | True | no |

## Set failed

- `method` = Rankedcondition: The value 'Rankedcondition' is not valid for the property 'method'
- `merge_condition` = Age = Age: The property 'merge_condition' is undefined
- `ranking_expression` = Age: The value 'Age' is not valid for the property 'ranking_expression'
- `Num_matches` = 3: The property 'Num_matches' is undefined (+1 more values)

## Set succeeded but file unchanged

- `change: instantiate fields by running a temporary Table node on the source`
- `primary_dataset`
- `rename_duplicate_fields`

## Observations

- Class `MergeNode` (probed with two Variable File inputs). `method` -> `mergeMode`: Order=`order` (default), Keys=`key`, Condition=`condition`; `Rankedcondition` (and `merge_condition`, `ranking_expression`, `Num_matches`) are NOT accepted by this Modeler version's scripting API although the guide lists them. `condition` -> `mergeCondition`; `key_fields` -> `keyFields/field[n]` (single field unindexed); `common_keys` -> `mergeCommonKeys` (default true); `join` -> `joinType`: Inner=`inner` (default), FullOuter=`fullOuter`, PartialOuter=`partialOuter`, Anti=`anti`.
- `outer_join_tag[n]` is KEYED by tag name: `includeIncompleteRecords/includeIncompleteRecordsList/includeIncompleteRecords[i]/inputNodeTagName` (one element per tag set True). `single_large_input` -> `mergeSingleLargeInput`; `single_large_input_tag` -> `mergeSingleLargeInputTag`; `use_existing_sort_keys` -> `mergePreSortedInput`; `existing_sort_keys` -> `existingSortKeys/rows/row[i]/{field,direction}` with direction `0` = Ascending, `1` = Descending.
- `primary_dataset="1"` and `rename_duplicate_fields=True` changed nothing (equal to defaults; not explored further).
