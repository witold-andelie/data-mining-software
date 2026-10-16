# `transform` node (`TransformNode`)

Probe `xtask/oracle/probes/settings_transform.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `fields` | `transformFields`<br>also: `transformFields/field[0]`, `transformFields/field[1]` | ['Age', 'Na'] -> (removed) | [] | yes |
| `formula` | `allTransformations` | Select -> select<br>All -> all | All; file: all | yes |
| `formula_inverse` | `includeInverse` | True -> true | False; file: false | yes |
| `formula_inverse_offset` | `inverseOffset` | 2.5 -> 2 | 0; file: 0 | yes |
| `formula_log_n` | `includeLogN` | True -> true | False; file: false | yes |
| `formula_log_n_offset` | `logNOffset` | 1 -> 1 | 0; file: 0 | yes |
| `formula_log_10` | `includeLog10` | True -> true | False; file: false | yes |
| `formula_log_10_offset` | `log10Offset` | 3 -> 3 | 0; file: 0 | yes |
| `formula_exponential` | `includeExponential` | True -> true | False; file: false | yes |
| `formula_square_root` | `includeSquareRoot` | True -> true | False; file: false | yes |
| `use_output_name` | `autoOutputName` | True -> false | False; file: true | yes |
| `output_name` | `outputName` | tr -> tr |  | yes |
| `output_mode` | `outputMode` | File -> file<br>Screen -> screen | Screen; file: screen | yes |
| `output_format` | `reportOutputFileType` | HTML -> (no change)<br>Output -> output_object | HTML | yes |
| `paginate_output` | `paginateOutput` | False -> false | True; file: true | yes |
| `lines_per_page` | `outputLinesPerPage` | 30 -> 30 | 400; file: 400 | yes |
| `full_filename` | `outputFilePath` | D:/out/t.html -> D:/out/t.html |  | yes |

## Set failed

(none)

## Set succeeded but file unchanged

(none)

## Observations

- Class `TransformNode`. `fields` -> `transformFields/field[n]`; `formula` -> `allTransformations` (`all` default | `select`); `formula_inverse`/`_log_n`/`_log_10`/`_exponential`/`_square_root` -> `includeInverse`/`includeLogN`/`includeLog10`/`includeExponential`/`includeSquareRoot` (default false); offsets -> `inverseOffset`/`logNOffset`/`log10Offset` stored as INTEGERS: `formula_inverse_offset = 2.5` was truncated to `2` in the file.
- Output: the file type element is named `reportOutputFileType` (html | output_object); `HTML` was already the default so the first set shows no change. Other output fields as in Table (`autoOutputName`, `outputName`, `outputMode`, `paginateOutput`, `outputLinesPerPage`, `outputFilePath`).
