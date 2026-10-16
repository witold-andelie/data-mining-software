# `statisticsimport` node (`SPSSInputNode`)

Probe `xtask/oracle/probes/settings_statisticsimport.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `full_filename` | `inputFileName` | D:/data-mining-software/corpus/ibm-demo-... -> D:/data-mining-software/corpus/ibm-d...<br>D:/data/other file.sav -> D:/data/other file.sav<br>D:/data-mining-software/corpus/ibm-demo-... -> D:/data-mining-software/corpus/ibm-d... |  | yes |
| `import_names` | `readVariableNames` | LabelsAsNames -> labelsAsNames<br>NamesAndLabels -> namesAndLabels<br>LabelsAsNames -> labelsAsNames | NamesAndLabels; file: namesAndLabels | yes |
| `import_data` | `readValues` | LabelsAsData -> labelsAsData<br>DataAndLabels -> dataAndLabels | DataAndLabels; file: dataAndLabels | yes |
| `use_field_format_for_storage` | `useFieldFormatForStorage` | True -> true<br>False -> false | False; file: false | yes |
| `file_encrypted` | `fileIsPasswordEncrypted` | True -> true<br>False -> false | False; file: false | yes |
| `password` | (none) | secret -> (no change)<br> -> (no change) |  | no |
| `change: instantiate by running a temporary Table node` | (none) |  -> (no change) |  | no |
| `direction[field]` | `typeTable/rows/row[$field]/direction`<br>also: `typeTable/rows`, `typeTable/rows/row[$field]/name` | Target -> out | Input; file: absent | yes |
| `type[field]` | `typeTable/rows/row[$field]/measure` | Range -> range | Range; file: absent | yes |
| `check[field]` | `typeTable/rows/row[$field]/check` | Coerce -> coerce | None; file: absent | yes |
| `values[field]` | `typeTable/rows/row[$field]/type/values/value[0]`<br>also: `typeTable/rows/row[$field]/name`, `typeTable/rows/row[$field]/measure`, `typeTable/rows/row[$field]/valueMode` | [0, 1] -> [0, 1] | [0.0, 1.0]; file: absent | yes |
| `missing_values[field]` | `typeTable/rows/row[$field]/missing/values/value`<br>also: `typeTable/rows/row[$field]/missing/enabled`, `typeTable/rows/row[$field]/missing/nullIncluded`, `typeTable/rows/row[$field]/missing/rangeIncluded` | [0] -> 0 | []; file: absent | yes |
| `description[field]` | `typeTable/rows/row[$field]/annotation` | Age in years -> Age in years | Age in years; file: absent | yes |

## Set failed

(none)

## Set succeeded but file unchanged

- `password`
- `change: instantiate by running a temporary Table node`

## Observations

- Class `SPSSInputNode`. `full_filename` -> `inputFileName` (verbatim, no existence check). `import_names` -> `readVariableNames` (`namesAndLabels` default | `labelsAsNames`); `import_data` -> `readValues` (`dataAndLabels` default | `labelsAsData`); `use_field_format_for_storage` -> `useFieldFormatForStorage` (default false); `file_encrypted` -> `fileIsPasswordEncrypted`.
- `password` is NOT written to the saved stream: setting it (even with `file_encrypted=true`) changed nothing in the file; the value only exists in the live session.
- Per-field keyed settings use the same `typeTable/rows/row[name]` layout as the Type node (see `type.md`); a row is created lazily when a field is first touched. `values` on a field with value labels adds `type/valueLabels` (labels from the .sav) next to `type/values`.
- `direction` default for a source field reads `Input` while the file has no `direction` element.
- Reading `import_names`/`import_data` back returns `NamesAndLabels`/`DataAndLabels`: these defaults are what the sav_to_csv oracle ran with (together with `use_field_format_for_storage=false`).

## Read-only observations

- `import_data`: `DataAndLabels`
- `import_names`: `NamesAndLabels`
