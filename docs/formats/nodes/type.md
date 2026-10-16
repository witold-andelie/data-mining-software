# `type` node (`TypeNode`)

Probe `xtask/oracle/probes/settings_type.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `use_custom_name` | `autoLabel` | True -> false | False; file: true | yes |
| `custom_name` | `label` | My Type -> My Type |  | yes |
| `tooltip` | `toolTipText` | tip text -> tip text |  | yes |
| `annotation` | `annotation` | note text -> note text |  | yes |
| `keywords` | `keywords`<br>also: `keywords/value` | kw1 kw2 -> (removed) | [] | yes |
| `cache_enabled` | `cacheEnabled` | True -> true | False; file: false | yes |
| `change: instantiate the Type node by running a temporary Table node` | `typeTable/rows/row[$field]/name`<br>also: `typeTable/rows/row[$field]/valueMode`, `typeTable/rows/row[$field]/type/measure`, `typeTable/rows/row[$field]/type/storage` |  -> Sex | ; file: absent | yes |
| `direction[field]` | `typeTable/rows/row[$field]/direction` | Input -> in<br>Target -> out<br>Both -> both<br>None -> none<br>Partition -> partition<br>Split -> split<br>Frequency -> freqWeight<br>RecordID -> record_id<br>In -> in<br>Out -> out | Input; file: absent | yes |
| `type[field]` | `typeTable/rows/row[$field]/measure`<br>also: `typeTable/rows/row[$field]/valueMode`, `typeTable/rows/row[$field]/type/measure`, `typeTable/rows/row[$field]/type/storage` | Range -> discrete<br>Flag -> flag<br>Set -> set<br>Typeless -> typeless<br>Discrete -> discrete<br>OrderedSet -> orderedSet<br>Default -> discrete<br>Range -> range | Set; file: absent | yes |
| `measure_type[field]` | `typeTable/rows/row[$field]/measure`<br>also: `typeTable/rows/row[$field]/valueMode`, `typeTable/rows/row[$field]/type/values`, `typeTable/rows/row[$field]/type/measure` | Range -> (no change)<br>Flag -> flag<br>Set -> set<br>Typeless -> typeless<br>Discrete -> discrete<br>OrderedSet -> orderedSet | Discrete | yes |
| `check[field]` | `typeTable/rows/row[$field]/check` | None -> (no change)<br>Nullify -> nullify<br>Coerce -> coerce<br>Discard -> discard<br>Warn -> warn<br>Abort -> abort | None | yes |
| `values[field]` | `typeTable/rows/row[$field]/type/values/value[0]`<br>also: `typeTable/rows/row[$field]/valueMode`, `typeTable/rows/row[$field]/measure`, `typeTable/rows/row[$field]/type/values/value[1]` | [10, 50] -> [10, 50]<br>['drugA', 'drugB', 'drugC'] -> typeTable/rows/row[$field]/measure=set<br>['F', 'M'] -> typeTable/rows/row[$field]/measure=flag<br>['HIGH', 'LOW', 'NORMAL'] -> [HIGH, LOW, NORMAL]<br>[10, 50] -> [10, 50] | [15, 74]; file: 15 | yes |
| `value_mode[field]` | `typeTable/rows/row[$field]/valueMode`<br>also: `typeTable/rows/row[$field]/augment`, `typeTable/rows/row[$field]/type/measure`, `typeTable/rows/row[$field]/type/storage` | Read -> read<br>Pass -> pass<br>Read+ -> typeTable/rows/row[$field]/augment=true<br>Current -> (no change) | Specify; file: local | yes |
| `extend_values[field]` | `typeTable/rows/row[$field]/augment` | False -> (removed) | True; file: true | yes |
| `enable_missing[field]` | `typeTable/rows/row[$field]/missingEnabled` | True -> true | False; file: absent | yes |
| `missing_values[field]` | `typeTable/rows/row[$field]/missing/values/value`<br>also: `typeTable/rows/row[$field]/missing/enabled`, `typeTable/rows/row[$field]/missing/nullIncluded`, `typeTable/rows/row[$field]/missing/rangeIncluded` | ['HIGH'] -> HIGH<br>[0, 99] -> typeTable/rows/row[$field]/missing/enabled=false | []; file: absent | yes |
| `range_missing[field]` | `typeTable/rows/row[$field]/missing/rangeIncluded` | True -> true | False; file: false | yes |
| `missing_lower[field]` | `typeTable/rows/row[$field]/missing/rangeLowerBound`<br>also: `typeTable/rows/row[$field]/missing/rangeLowerBound/@type` | 5 -> 5 | ; file: absent | yes |
| `missing_upper[field]` | `typeTable/rows/row[$field]/missing/rangeUpperBound`<br>also: `typeTable/rows/row[$field]/missing/rangeUpperBound/@type` | 95 -> 95 | ; file: absent | yes |
| `null_missing[field]` | `typeTable/rows/row[$field]/missing/nullIncluded` | False -> false | True; file: true | yes |
| `whitespace_missing[field]` | `typeTable/rows/row[$field]/missing/whitespaceIncluded` | False -> false | True; file: true | yes |
| `description[field]` | `typeTable/rows/row[$field]/annotation` | Blood Pressure -> Blood Pressure | ; file: absent | yes |
| `value_labels[field]` | `typeTable/rows/row[$field]/type/valueLabels/valueKey[0]`<br>also: `typeTable/rows/row[$field]/type/valueLabels/valueKey[1]`, `typeTable/rows/row[$field]/type/valueLabels/valueLabel[0]`, `typeTable/rows/row[$field]/type/valueLabels/valueLabel[1]` | [['HIGH', 'High Blood Pressure'], ['NORM... -> [HIGH, NORMAL]<br>[[10, 'ten'], [50, 'fifty']] -> [10, 50]<br>[['F', 'Female'], ['M', 'Male']] -> [F, M] | {}; file: absent | yes |
| `display_places[field]` | `formattingTable/rows/row[$field]/displayPlaces`<br>also: `formattingTable/rows`, `formattingTable/rows/row[$field]/name`, `formattingTable/rows/row[$field]/justify` | 5 -> 5 | -1; file: absent | yes |
| `export_places[field]` | `formattingTable/rows/row[$field]/exportPlaces` | 6 -> 6 | -1; file: -1 | yes |
| `decimal_separator[field]` | `formattingTable/rows/row[$field]/decimalSymbol` | DEFAULT -> (no change)<br>PERIOD -> period<br>COMMA -> comma | DEFAULT | yes |
| `date_format[field]` | `formattingTable/rows/row[$field]/dateFormat`<br>also: `formattingTable/rows/row[$field]/name`, `formattingTable/rows/row[$field]/justify`, `formattingTable/rows/row[$field]/manualWidth` | YYYYMMDD -> format4 | DEFAULT; file: absent | yes |
| `time_format[field]` | `formattingTable/rows/row[$field]/timeFormat` | HH:MM:SS -> format4 | DEFAULT; file: absent | yes |
| `number_format[field]` | `formattingTable/rows/row[$field]/numberFormat` | DEFAULT -> (no change)<br>STANDARD -> standard<br>SCIENTIFIC -> scientific<br>CURRENCY -> currency | DEFAULT | yes |
| `standard_places[field]` | `formattingTable/rows/row[$field]/displayPlaces` | 4 -> 4 | 5; file: 5 | yes |
| `scientific_places[field]` | `formattingTable/rows/row[$field]/scientificPlaces` | 4 -> 4 | -1; file: -1 | yes |
| `currency_places[field]` | `formattingTable/rows/row[$field]/currencyPlaces` | 4 -> 4 | -1; file: -1 | yes |
| `grouping_symbol[field]` | `formattingTable/rows/row[$field]/groupingSymbol` | DEFAULT -> (no change)<br>NONE -> none<br>LOCALE -> locale<br>PERIOD -> period<br>COMMA -> comma<br>SPACE -> space | DEFAULT | yes |
| `column_width[field]` | `formattingTable/rows/row[$field]/manualWidth` | 25 -> 25 | -1; file: -1 | yes |
| `justify[field]` | `formattingTable/rows/row[$field]/justify` | AUTO -> (no change)<br>CENTER -> center<br>LEFT -> left<br>RIGHT -> right | AUTO | yes |

## Set failed

- `value_mode[Age]` = Specify: The value 'Specify' is not valid for the property 'value_mode'

## Set succeeded but file unchanged

(none)

## Observations

- Common properties (`use_custom_name`, `custom_name`, `tooltip`, `annotation`, `keywords`, `cache_enabled`) are stored as sibling elements of the node's `properties`: `autoLabel` (inverse of use_custom_name; the first scripted property set on a node also flips `autoLabel` to false), `label`, `toolTipText`, `annotation`, `keywords/value`, `cacheEnabled`. See `common.md`.
- Per-field settings live in two tables: `typeTable/rows/row` (direction, measure, valueMode, check, values, missing, valueLabels, annotation) and `formattingTable/rows/row` (display/export places, symbols, date/time format, width, justify). A row is created lazily: `formattingTable/rows` is an empty element until the first formatting property is set on a field, and then only fields that were touched get a row.
- `direction` encodings: Input -> `in` (default; element absent in file), Target -> `out`, Both -> `both`, None -> `none`, Partition -> `partition`, Split -> `split`, Frequency -> `freqWeight`, RecordID -> `record_id`. The deprecated `In`/`Out` are accepted and read back as Input/Target.
- `type`/`measure_type` are stored in `row/measure` (`range`, `discrete`, `flag`, `set`, `orderedSet`, `typeless`). Setting `Range` on a string field is silently refused and reads back `Discrete`; `Default` reads back `Discrete`. The Type node is not instantiated until the stream is run once: before that, formatting properties on fields cannot be set (set raised an error with message None) and only a few keyed properties work.
- After instantiation, the Type node stores `row/type` (storage, measure, values, statistics: `univariateStats`, `storageSize`, `elementCount`, `truncated`) as a cache; `row/valueMode` is `computed` (values read from data). Setting `values` switches `valueMode` to `local` (Specify) and replaces the cached values; `value_mode=Pass` removes the cached `type` element; `Read+` writes `augment=true`; `Current` is accepted but unchanged; `Specify` is rejected by the API.
- Value type tags on `value` elements: `J` long integer (data read), `I` integer (scripted), `D` double, `A` string.
- Missing values: `enable_missing` -> `missingEnabled` (element absent when false); `missing_values`, `range_missing`, `missing_lower/upper`, `null_missing`, `whitespace_missing` -> `missing/{enabled,nullIncluded,rangeIncluded,whitespaceIncluded,values/value,rangeLowerBound,rangeUpperBound}`. `missing/nullIncluded`/`whitespaceIncluded` default true.
- `value_labels` -> `type/valueLabels/valueKey[n]` + `valueLabel[n]` (parallel lists, key has a `@type` tag); the API reads them back as a dict.
- Formatting: `decimalSymbol` default/period/comma; `numberFormat` standard/scientific/currency; `groupingSymbol` none/locale/period/comma/space; `justify` auto/center/left/right; `dateFormat`/`timeFormat` are `formatN` indices (e.g. YYYYMMDD -> `format4`, HH:MM:SS -> `format4`); `manualWidth` -1 = Auto; places -1 = stream default. `standard_places` and `display_places` share `displayPlaces`.

## Read-only observations

- `direction_Drug_after_run`: `Target`
- `storage_Age`: `Integer`
- `storage_Na`: `Real`
- `storage_Sex`: `String`
- `type_Age_after_run`: `Range`
- `type_BP_after_run`: `OrderedSet`
- `type_Sex_after_run`: `Flag`
- `value_labels_BP`: `{u'HIGH': u'High Blood Pressure', u'NORMAL': u'normal blood pressure'}`
- `values_Age_after_run`: `[]`
- `values_BP_after_run`: `[HIGH, LOW, NORMAL]`
