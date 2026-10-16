# `analysis` node (`AnalysisNode`)

Probe `xtask/oracle/probes/settings_analysis.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `coincidence` | (none) | False -> (no change) | False | no |
| `performance` | `appendTagField` | True -> true | False; file: false | yes |
| `evaluation_binary` | `analysisBinaryEvaluation` | True -> true | False; file: false | yes |
| `confidence` | `analysisConfidenceFigures` | True -> true | False; file: false | yes |
| `threshold` | `analysisThreshold` | 75 -> 75<br>150 -> 100 | 90; file: 90 | yes |
| `improve_accuracy` | `analysisImproveAccuracy` | 3 -> 3.0<br>2.5 -> 2.5 | 2.0; file: 2.0 | yes |
| `field_detection_method` | `fieldDetectionMethod` | Name -> name<br>Metadata -> metadata | Metadata; file: metadata | yes |
| `inc_user_measure` | `analysisUserDefined` | True -> true | False; file: false | yes |
| `user_if` | `analysisUserIf` | @TARGET = @PREDICTED -> @TARGET = @PREDICTED | @TARGET == @PREDICTED; file: @TARGET == @PREDICTED | yes |
| `user_then` | `analysisUserThen` | 101 -> 101 | 100; file: 100 | yes |
| `user_else` | `analysisUserElse` | 1 -> 1 | 0; file: 0 | yes |
| `user_compute` | `analysisUserCompute/setElement`<br>also: `analysisUserCompute/setElement[0]`, `analysisUserCompute/setElement[1]`, `analysisUserCompute/setElement[2]` | ['Mean', 'Sum'] -> analysisUserCompute/setElement[0]=mean<br>['Min'] -> min<br>['Max'] -> max<br>['SDev'] -> sdev<br>['Mean'] -> mean<br>['Sum'] -> sum | [SDev, Min, Mean, Max, Sum] | yes |
| `by_fields` | `analysisBreakdownBy`<br>also: `analysisBreakdownBy/field` | ['Drug'] -> (removed) | [] | yes |
| `use_output_name` | `autoOutputName` | True -> false | False; file: true | yes |
| `output_name` | `outputName` | an -> an |  | yes |
| `output_mode` | `outputMode` | File -> file<br>Screen -> screen | Screen; file: screen | yes |
| `output_format` | `outputFileType` | HTML -> (no change)<br>Output -> output_object<br>Text -> text | HTML | yes |
| `full_filename` | `outputFilePath` | D:/out/analysis_out.html -> D:/out/analysis_out.html |  | yes |

## Set failed

(none)

## Set succeeded but file unchanged

- `coincidence`

## Observations

- Class `AnalysisNode`. Defaults: `coincidence` false (it is already the default; the file element was not identified because setting False changed nothing; a True probe is remaining), `performance` false -> `appendTagField`? (mapped by the diff: performance=True changed `appendTagField`, treat as an inference), `evaluation_binary` false, `confidence` false, `threshold` 90 (clamped to 100: 150 -> 100), `improve_accuracy` 2.0 (stored as real: 3 -> `3.0`), `field_detection_method` `metadata`, user measure `analysisUserIf` default `@TARGET == @PREDICTED`, then `100`, else `0`, `analysisUserCompute` default all five (sdev,min,mean,max,sum).
- `user_compute` -> `analysisUserCompute/setElement[n]` lowercase (`mean`, `sum`, `min`, `max`, `sdev`); single element has no index. `by_fields` -> `analysisBreakdownBy/field`.
- Output properties as for Table/Data Audit: `outputFileType` `text`|`html`|`output_object` (values `Text`, `HTML`, `Output`).
