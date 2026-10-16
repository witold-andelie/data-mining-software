# `aggregate` node (`AggregateNode`)

Probe `xtask/oracle/probes/settings_aggregate.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `change: instantiate fields by running a temporary Table node on the source` | (none) |  -> (no change) |  | no |
| `keys` | `aggregateKeyFields/field`<br>also: `aggregateKeyFields`, `aggregateKeyFields/field[0]`, `aggregateKeyFields/field[1]` | ['Drug'] -> Drug<br>['Drug', 'Sex'] -> [Drug, Sex] | []; file: absent | yes |
| `contiguous` | `aggregateContiguousKeys` | True -> true | False; file: false | yes |
| `aggregates[field]` | `aggregateFields/rows/row/max`<br>also: `aggregateFields/rows/row/min`, `aggregateFields/rows/row/count`, `aggregateFields/rows/row/median` | ['Sum', 'Mean'] -> false<br>['Sum', 'Mean', 'Min', 'Max', 'SDev', 'M... -> true<br>['Mean'] -> (removed) | ; file: absent | yes |
| `aggregate_exprs[field]` | `aggregateExpressionsTable/rows`<br>also: `aggregateExpressionsTable/rows/row/field`, `aggregateExpressionsTable/rows/row/index`, `aggregateExpressionsTable/rows/row/expression` | MAX('Na') -> (removed) |  | yes |
| `extension` | `aggregateNameExtension` | Aggregated_ -> Aggregated_ |  | yes |
| `add_as` | `aggregateExtensionPosition` | Prefix -> Prefix<br>Suffix -> Suffix | Suffix; file: Suffix | yes |
| `inc_record_count` | `aggregateIncludeCount` | False -> false | True; file: true | yes |
| `count_field` | `aggregateCountFieldName` | cnt -> cnt | Record_Count; file: Record_Count | yes |
| `allow_approximation` | `aggregateAllowApproximation` | True -> true | False; file: false | yes |
| `bin_count` | `aggregateNumberOfBins` | 2000 -> 2000<br>0 -> 100 | 1000; file: 1000 | yes |

## Set failed

(none)

## Set succeeded but file unchanged

- `change: instantiate fields by running a temporary Table node on the source`

## Observations

- Class `AggregateNode`. `keys` -> `aggregateKeyFields/field[n]`; `contiguous` -> `aggregateContiguousKeys`; `extension` -> `aggregateNameExtension`; `add_as` -> `aggregateExtensionPosition` (capitalised `Suffix` default | `Prefix`); `inc_record_count` -> `aggregateIncludeCount` (default true); `count_field` -> `aggregateCountFieldName` (default `Record_Count`); `allow_approximation` -> `aggregateAllowApproximation`; `bin_count` -> `aggregateNumberOfBins` (default 1000; 0 clamps to 100).
- `aggregates[field]` is KEYED: `aggregateFields/rows/row[i]/{field,sum,mean,min,max,stddev,median,count,variance,firstQuartile,thirdQuartile}` with one boolean per mode (all ten written, true/false). Mode names: Sum, Mean, Min, Max, SDev (`stddev`), Median, Count, Variance, FirstQuartile, ThirdQuartile. A single row is unindexed. Setting the same key again replaces that row's flags.
- `aggregate_exprs[name]` is KEYED: `aggregateExpressionsTable/rows/row[i]/{field,expression,index}`.
