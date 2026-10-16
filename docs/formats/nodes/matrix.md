# `matrix` node (`MatrixNode`)

Probe `xtask/oracle/probes/settings_matrix.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `fields` | `matrixType` | Flags -> flags<br>Numerics -> numbers<br>Selected -> specified | Selected; file: specified | yes |
| `row` | `rowField` | Drug -> Drug |  | yes |
| `column` | `columnField` | BP -> BP |  | yes |
| `include_missing_values` | (none) | True -> (no change) | True | no |
| `cell_contents` | `cellContents` | CrossTabs -> (no change)<br>Function -> parameter | CrossTabs | yes |
| `function_field` | `parameterField` | Age -> Age |  | yes |
| `function` | `parameterFunction` | Sum -> sum<br>Mean -> mean<br>Min -> min<br>Max -> max<br>SDev -> sdev | Mean; file: mean | yes |
| `sort_mode` | `rowColumnSort` | Ascending -> ascending<br>Descending -> descending<br>Unsorted -> unsorted | Unsorted; file: unsorted | yes |
| `highlight_bottom` | `highlightBottom`<br>also: `useHighlightBottom` | 5 -> 5 | 0; file: 0 | yes |
| `highlight_top` | (none) | 0 -> (no change) | 0 | no |
| `display` | `coincidenceFunction/setElement`<br>also: `coincidenceFunction/setElement[0]`, `coincidenceFunction/setElement[1]`, `coincidenceFunction/setElement[2]` | ['Counts'] -> (no change)<br>['Counts', 'Expected', 'Residuals'] -> [counts, expected, residual]<br>['RowPct'] -> row<br>['ColumnPct'] -> column<br>['TotalPct'] -> total | [Counts] | yes |
| `include_totals` | `showTotals` | True -> true | False; file: false | yes |
| `use_output_name` | `autoOutputName` | True -> false | False; file: true | yes |
| `output_name` | `outputName` | mx -> mx |  | yes |
| `output_mode` | `outputMode` | File -> file<br>Screen -> screen | Screen; file: screen | yes |
| `output_format` | `outputFileType` | Delimited -> delimited<br>HTML -> html<br>Output -> output_object<br>Formatted -> formatted | Formatted; file: formatted | yes |
| `paginate_output` | (none) | True -> (no change) | True | no |
| `lines_per_page` | `outputLinesPerPage` | 50 -> 50 | 400; file: 400 | yes |
| `full_filename` | `outputFilePath` | D:/out/m.html -> D:/out/m.html |  | yes |

## Set failed

- `highlight_top` = 1.5: The value '1.5' is not valid for the property 'highlight_top'
- `output_format` = Delimited transposed: The value 'Delimited transposed' is not valid for the property 'output_format'

## Set succeeded but file unchanged

- `include_missing_values`
- `highlight_top`
- `paginate_output`

## Observations

- Class `MatrixNode`. `fields` -> `matrixType`: Selected=`specified` (default), Flags=`flags`, Numerics=`numbers`; `row`/`column` -> `rowField`/`columnField`; `cell_contents` -> `cellContents`: CrossTabs=`coincidences` (default), Function=`parameter`; `function_field` -> `parameterField`; `function` -> `parameterFunction` (`mean` default, `sum`, `min`, `max`, `sdev`); `sort_mode` -> `rowColumnSort` (`unsorted` default, `ascending`, `descending`); `highlight_bottom=5` -> `highlightBottom=5` + `useHighlightBottom=true` (setting 0 turns the flag off); `highlight_top` must be an integer (1.5 rejected); `display` -> `coincidenceFunction/setElement[n]`: Counts=`counts` (default), Expected=`expected`, Residuals=`residual`, RowPct=`row`, ColumnPct=`column`, TotalPct=`total` (single element unindexed); `include_totals` -> `showTotals`; `include_missing_values` default true.
- Output properties as for Table (`outputMode`, `outputFileType`, `outputName`, `outputLinesPerPage`, `outputFilePath`); the `transposed` modifier value ("Delimited transposed") is rejected.
