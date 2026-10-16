# `table` node (`TableNode`)

Probe `xtask/oracle/probes/settings_table.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `use_custom_name` | `autoLabel` | True -> false | False; file: true | yes |
| `full_filename` | `outputFilePath` | D:/out/table_output.htm -> D:/out/table_output.htm |  | yes |
| `use_output_name` | `autoOutputName` | True -> false | False; file: true | yes |
| `output_name` | `outputName` | My table -> My table |  | yes |
| `output_mode` | `outputMode` | File -> file<br>Screen -> screen | Screen; file: screen | yes |
| `output_format` | `outputFileType` | Formatted -> (no change)<br>Delimited -> delimited<br>HTML -> html<br>Output -> output_object | Formatted | yes |
| `transpose_data` | `transposeData` | True -> true | False; file: false | yes |
| `paginate_output` | `paginateOutput` | False -> false | True; file: true | yes |
| `lines_per_page` | `outputLinesPerPage` | 50 -> 50<br>0 -> 20 | 400; file: 400 | yes |
| `highlight_expr` | `tableHighlightExpression` | Age > 30 -> Age > 30<br> ->  |  | yes |
| `change: run the table node once` | (none) |  -> (no change) |  | no |
| `display_places[field]` | `formattingTable/rows/row[$field]/displayPlaces`<br>also: `formattingTable/rows`, `formattingTable/rows/row[$field]/name`, `formattingTable/rows/row[$field]/justify` | 5 -> 5 | -1; file: absent | yes |
| `export_places[field]` | `formattingTable/rows/row[$field]/exportPlaces` | 6 -> 6 | -1; file: -1 | yes |
| `decimal_separator[field]` | `formattingTable/rows/row[$field]/decimalSymbol` | PERIOD -> period<br>COMMA -> comma<br>DEFAULT -> default | DEFAULT; file: default | yes |
| `date_format[field]` | `formattingTable/rows/row[$field]/dateFormat`<br>also: `formattingTable/rows/row[$field]/name`, `formattingTable/rows/row[$field]/justify`, `formattingTable/rows/row[$field]/manualWidth` | YYYYMMDD -> format4 | DEFAULT; file: absent | yes |
| `time_format[field]` | `formattingTable/rows/row[$field]/timeFormat` | HH:MM:SS -> format4 | DEFAULT; file: absent | yes |
| `column_width[field]` | `formattingTable/rows/row[$field]/manualWidth` | 25 -> 25 | -1; file: -1 | yes |
| `justify[field]` | `formattingTable/rows/row[$field]/justify` | CENTER -> center<br>LEFT -> left<br>RIGHT -> right<br>AUTO -> auto | AUTO; file: auto | yes |

## Set failed

(none)

## Set succeeded but file unchanged

- `change: run the table node once`

## Observations

- Class `TableNode`. `output_format` values are `Formatted`, `Delimited`, `HTML`, `Output` (the labels with extensions such as `Delimited (.csv)` printed in the guide are rejected).
- `output_mode` -> `outputMode` (`screen` default | `file`); `full_filename` -> `outputFilePath`; `use_output_name` is stored inverted as `autoOutputName`; `output_name` -> `outputName`; `transpose_data` -> `transposeData`; `lines_per_page` -> `outputLinesPerPage` (default 400; 0 is clamped to 20); `highlight_expr` -> `tableHighlightExpression`.
- `value_labels` is not defined on the Table node (despite the guide table). The per-field formatting properties (`display_places`, `export_places`, `decimal_separator`, `date_format`, `time_format`, `column_width`, `justify`) use the `formattingTable/rows/row[name]` layout described in `type.md`; a whole row (all columns with defaults) is created the first time any property of that field is set.
- `output_format` -> `outputFileType`: Formatted = `formatted` (default), Delimited = `delimited`, HTML = `html`, Output = `output_object`. `paginate_output` -> `paginateOutput` (default true).
