# `dataaudit` node (`DataAuditNode`)

Probe `xtask/oracle/probes/settings_dataaudit.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `custom_fields` | `dataAuditUseCustomFields` | True -> true | False; file: false | yes |
| `fields` | `dataAuditFields`<br>also: `dataAuditFields/field[0]`, `dataAuditFields/field[1]`, `dataAuditFields/field[2]` | ['Age', 'Na', 'K'] -> (removed) | [] | yes |
| `overlay` | `dataAuditUserSpecifiedOverlayField` | Drug -> Drug |  | yes |
| `display_graphs` | `dataAuditDisplayGraphs` | False -> false | True; file: true | yes |
| `basic_stats` | `dataAuditBasicStatistics` | False -> false | True; file: true | yes |
| `advanced_stats` | `dataAuditAdvancedStatistics` | True -> true | False; file: false | yes |
| `median_stats` | `dataAuditMedianAndMode` | True -> true | False; file: false | yes |
| `calculate` | `qualityCalculateValues/setElement`<br>also: `qualityCalculateValues/setElement[0]`, `qualityCalculateValues/setElement[1]`, `qualityCalculateValues` | ['Count'] -> records<br>['Breakdown'] -> breakdown<br>['Count', 'Breakdown'] -> [breakdown, records]<br>[] -> qualityCalculateValues= | [Breakdown, Count]; file: absent | yes |
| `outlier_detection_method` | `dataAuditOutlierDetectionMethod` | iqr -> iqr<br>std -> std | std; file: std | yes |
| `outlier_detection_std_outlier` | `dataAuditStdOutlier` | 2.5 -> 2.5 | 3.0; file: 3.0 | yes |
| `outlier_detection_std_extreme` | `dataAuditStdExtreme` | 4.5 -> 4.5 | 5.0; file: 5.0 | yes |
| `outlier_detection_iqr_outlier` | `dataAuditIQROutlier` | 2.5 -> 2.5 | 1.5; file: 1.5 | yes |
| `outlier_detection_iqr_extreme` | `dataAuditIQRExtreme` | 4.5 -> 4.5 | 3.0; file: 3.0 | yes |
| `use_output_name` | `autoOutputName` | True -> false | False; file: true | yes |
| `output_name` | `outputName` | audit -> audit |  | yes |
| `output_mode` | `outputMode` | File -> file<br>Screen -> screen | Screen; file: screen | yes |
| `output_format` | `outputFileType` | Delimited -> delimited<br>HTML -> html<br>Output -> output_object<br>Formatted -> formatted | Formatted; file: formatted | yes |
| `paginate_output` | (none) | True -> (no change) | True | no |
| `lines_per_page` | `outputLinesPerPage` | 77 -> 77 | 400; file: 400 | yes |
| `full_filename` | `outputFilePath` | D:/out/audit.html -> D:/out/audit.html |  | yes |

## Set failed

(none)

## Set succeeded but file unchanged

- `paginate_output`

## Observations

- Class `DataAuditNode`. Defaults: `custom_fields` false (`dataAuditUseCustomFields`), `display_graphs` true, `basic_stats` true, `advanced_stats` false, `median_stats` false (`dataAuditMedianAndMode`), std outlier 3.0 / extreme 5.0, IQR outlier 1.5 / extreme 3.0, `outlier_detection_method` `std`.
- `calculate` -> `qualityCalculateValues/setElement`: Count=`records`, Breakdown=`breakdown`; default contains both. One element is stored without index, two or more with `[0]`,`[1]` (order: breakdown, records); an empty list gives an empty `qualityCalculateValues`.
- Output properties share the Table node encoding: `outputMode` screen|file, `outputFileType` formatted|delimited|html|output_object, `autoOutputName` (inverse of `use_output_name`), `outputName`, `outputLinesPerPage` default 400, `outputFilePath`; `paginate_output` default true. Format values are `Formatted`, `Delimited`, `HTML`, `Output` (guide shows extensions in parentheses; not used).
