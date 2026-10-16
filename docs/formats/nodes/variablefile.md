# `variablefile` node (`DelimitedFileImportNode`)

Probe `xtask/oracle/probes/settings_variablefile.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `full_filename` | `inputFileName` | D:/data-mining-software/corpus/ibm-demo-... -> D:/data-mining-software/corpus/ibm-d...<br>D:/data/other file.csv -> D:/data/other file.csv<br>D:/data-mining-software/corpus/ibm-demo-... -> D:/data-mining-software/corpus/ibm-d... |  | yes |
| `skip_header` | `skipHeaderCount` | 5 -> 5<br>70000 -> 70000 | 0; file: 0 | yes |
| `num_fields_auto` | `useManualFieldCount` | False -> true | True; file: false | yes |
| `num_fields` | `manualFieldCount` | 12 -> 12<br>0 -> 1 | 1; file: 1 | yes |
| `delimit_space` | `delimitOnSpace` | True -> true | False; file: false | yes |
| `delimit_tab` | `delimitOnTab` | True -> true | False; file: false | yes |
| `delimit_new_line` | (none) | True -> (no change) | True | no |
| `delimit_non_printing` | `delimitOnNonPrintChars` | True -> true | False; file: false | yes |
| `delimit_comma` | `delimitOnComma` | False -> false | True; file: true | yes |
| `delimit_other` | `useDelimiterChars` | True -> true | False; file: false | yes |
| `other` | `delimiterChars` | ; -> ;<br> ->  |  | yes |
| `decimal_symbol` | `realDecimalSymbol` | Comma -> comma<br>Period -> period<br>Default -> default | DEFAULT; file: default | yes |
| `multi_blank` | (none) | False -> (no change) | False | no |
| `read_field_names` | `fieldNamesIncluded` | False -> false | True; file: true | yes |
| `strip_spaces` | `trimMode` | Left -> left<br>Right -> right<br>Both -> both<br>None -> none | None; file: none | yes |
| `invalid_char_mode` | `invalidCharacterMode` | Replace -> replace<br>Discard -> discard | Discard; file: discard | yes |
| `invalid_char_replacement` | `invalidCharacterReplacement` | / -> / |  | yes |
| `break_case_by_newline` | `breakCasebyNewline` | True -> true | False; file: false | yes |
| `lines_to_scan` | `scanLineCount` | 500 -> 500<br>0 -> 1 | 50; file: 50 | yes |
| `auto_recognize_datetime` | `autoRecognizeDates` | False -> false | True; file: true | yes |
| `quotes_1` | `singleQuoteMode` | Discard -> discard<br>PairAndDiscard -> pairDiscard<br>IncludeAsText -> include | PairAndDiscard; file: pairDiscard | yes |
| `quotes_2` | `doubleQuoteMode` | Discard -> discard<br>PairAndDiscard -> pairDiscard<br>IncludeAsText -> include | PairAndDiscard; file: pairDiscard | yes |
| `encoding` | `fileEncoding` | SystemDefault -> serverDefault<br>UTF-8 -> UTF-8<br>StreamDefault -> clientDefault<br>GBK -> GBK | StreamDefault; file: clientDefault | yes |
| `change: instantiate by running a temporary Table node` | (none) |  -> (no change) |  | no |
| `use_custom_values[field]` | (none) | True -> (no change) | False | no |
| `values[field]` | `typeTable/rows/row[$field]/name`<br>also: `typeTable/rows/row[$field]/measure`, `typeTable/rows` | [1, 100] -> Age<br>['drugA', 'drugB'] -> Drug | []; file: absent | yes |
| `direction[field]` | `typeTable/rows/row[$field]/direction` | Input -> in<br>Target -> out<br>Both -> both<br>None -> none<br>Partition -> partition<br>Split -> split<br>Frequency -> freqWeight<br>RecordID -> record_id | ; file: absent | yes |
| `type[field]` | `typeTable/rows/row[$field]/measure`<br>also: `typeTable/rows/row[$field]/valueMode` | Range -> range<br>Flag -> flag<br>Set -> set<br>Typeless -> typeless<br>Discrete -> discrete<br>OrderedSet -> orderedSet<br>Default -> auto | Default; file: auto | yes |
| `measure_type[field]` | `typeTable/rows/row[$field]/measure` | Range -> range<br>Flag -> flag<br>Set -> set<br>Typeless -> typeless<br>Discrete -> discrete<br>OrderedSet -> orderedSet | Default; file: auto | yes |
| `check[field]` | `typeTable/rows/row[$field]/check` | None -> (no change)<br>Nullify -> nullify<br>Coerce -> coerce<br>Discard -> discard<br>Warn -> warn<br>Abort -> abort | None | yes |
| `custom_storage_type[field]` | `textImportTable/rows/row[$field]/storage`<br>also: `textImportTable/rows`, `textImportTable/rows/row[$field]/name`, `textImportTable/rows/row[$field]/override` | Unknown -> unknown<br>String -> string<br>Integer -> integer<br>Real -> real<br>Time -> time<br>Date -> date<br>Timestamp -> timestamp | ; file: absent | yes |
| `custom_storage[field]` | `textImportTable/rows/row[$field]/storage` | Real -> real | Timestamp; file: timestamp | yes |
| `custom_date_format[field]` | `textImportTable/rows/row[$field]/dateFormat` | DD/MM/YYYY -> format6 | ; file: absent | yes |
| `custom_time_format[field]` | `textImportTable/rows/row[$field]/timeFormat` | HH:MM:SS -> format4 | ; file: absent | yes |
| `custom_list_storage_type[field]` | `textImportTable/rows/row[$field]/storage`<br>also: `textImportTable/rows/row[$field]/dateFormat`, `textImportTable/rows/row[$field]/timeFormat`, `textImportTable/rows/row[$field]/ExtendedStorage/ListStorage` | Integer -> list | ; file: real | yes |
| `custom_list_depth[field]` | `textImportTable/rows/row[$field]/ExtendedStorage/ListStorage/@depth` | 2 -> 2 | 0; file: 0 | yes |
| `missing_values[field]` | `typeTable/rows/row[$field]/missing/values/value`<br>also: `typeTable/rows/row[$field]/missing/enabled`, `typeTable/rows/row[$field]/missing/nullIncluded`, `typeTable/rows/row[$field]/missing/rangeIncluded` | [0] -> 0 | []; file: absent | yes |
| `description[field]` | `typeTable/rows/row[$field]/annotation` | Age of patient -> Age of patient | ; file: absent | yes |

## Set failed

- `custom_decimal_symbol[Age]` = ,: java.lang.String incompatible with com.spss.clementine.component.FieldFormat$DecimalSymbol
- `geo_type[Age]` = Point: The value 'Point' is not valid for the property 'geo_type'
- `geo_coordinates[Age]` = 2D: The property 'geo_coordinates' is undefined

## Set succeeded but file unchanged

- `delimit_new_line`
- `multi_blank`
- `change: instantiate by running a temporary Table node`
- `use_custom_values[field]`

## Observations

- Class `DelimitedFileImportNode`; file name is stored in `inputFileName` exactly as given (forward slashes kept, spaces kept, no existence check). `skip_header` stored as given (70000 ok).
- Boolean flags are `true`/`false` text. Defaults differ from the guide's silence: `delimit_comma`, `delimit_new_line`, `read_field_names`, `num_fields_auto`, `auto_recognize_datetime` default true; `lines_to_scan` default 50; `num_fields` default 1; `quotes_1`/`quotes_2` default `PairAndDiscard`; `invalid_char_mode` default `Discard`; `strip_spaces` default `None`; `encoding` default `StreamDefault`.
- Clamping: `num_fields = 0` reads back 1 (file value 1); `lines_to_scan = 0` reads back 1 (file value 1).
- `num_fields_auto=False` is stored inversely as `useManualFieldCount=true`; `num_fields` -> `manualFieldCount`.
- Enum encodings: `decimal_symbol` -> `realDecimalSymbol` default/comma/period (API reads back upper-case `DEFAULT`); `strip_spaces` -> `trimMode` none/left/right/both; `invalid_char_mode` -> discard/replace; `quotes_*` -> `singleQuoteMode`/`doubleQuoteMode`: discard / pairDiscard / include; `encoding` -> `fileEncoding`: StreamDefault=`clientDefault`, SystemDefault=`serverDefault`, "UTF-8"=`UTF-8`, any other string (e.g. `GBK`) is stored verbatim (no validation).
- `multi_blank` set to False (its default is false) produced no change; `delimit_new_line` True equals its default.
- Per-field settings are in `typeTable/rows/row[name]` (same layout as the Type node: `measure` with extra value `auto` for Default, `direction`, `check`, `valueMode`, `missing/*`, `annotation`) and, for the storage overrides, `textImportTable/rows/row[name]` (`override=true`, `storage` unknown/string/integer/real/time/date/timestamp/list, `dateFormat`/`timeFormat` as `formatN`, `ExtendedStorage/ListStorage/@valueStorage/@depth` for list storage). Setting `custom_list_storage_type` changes `storage` to `list` and drops the date/time formats.
- `type=Typeless` also writes `valueMode=pass`. Setting `values` on an uninstantiated source row only creates a row with `measure=auto`; `use_custom_values=True` was not retained (reads back False) and `values` read back `[]`, so custom value ranges for source fields were NOT persisted by these calls (unresolved).
- `custom_decimal_symbol` requires an enum object, not a string (error `java.lang.String incompatible with FieldFormat$DecimalSymbol`); `geo_type`/`geo_coordinates` are not valid in this node/version for a non-geospatial field.
