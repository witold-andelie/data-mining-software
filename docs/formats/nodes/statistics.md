# `statistics` node (`StatisticsNode`)

Probe `xtask/oracle/probes/settings_statistics.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `examine` | `examineFields`<br>also: `examineFields/field[0]`, `examineFields/field[1]` | ['Age', 'Na'] -> (removed) | [] | yes |
| `correlate` | `correlationFields`<br>also: `correlationFields/field` | ['K'] -> (removed) | [] | yes |
| `statistics` | `statistics`<br>also: `statistics/setElement[0]`, `statistics/setElement[1]`, `statistics/setElement[2]` | ['mean', 'sum', 'sdev'] -> statistics/setElement[0]=sum<br>['count', 'min', 'max', 'range', 'varian... -> statistics/setElement[0]=semean<br>[] ->  | [SEMean, Min, Range, Max, Count, Mean, V... | yes |
| `correlation_mode` | `correlationStrengthMode` | Absolute -> absolute<br>Probability -> probability | Probability; file: probability | yes |
| `label_correlations` | (none) | True -> (no change) | True | no |
| `weak_label` | `weakCorrelationLabel` | weakish -> weakish | Weak; file: Weak | yes |
| `medium_label` | `mediumCorrelationLabel` | midish -> midish | Medium; file: Medium | yes |
| `strong_label` | `strongCorrelationLabel` | strongish -> strongish | Strong; file: Strong | yes |
| `weak_below_probability` | `weakProbabilityThreshold` | 0.8 -> 0.8 | 0.9; file: 0.9 | yes |
| `strong_above_probability` | (none) | 0.95 -> (no change) | 0.95 | no |
| `weak_below_absolute` | `weakCorrelationThreshold` | 0.25 -> 0.25 | 0.3333; file: 0.3333 | yes |
| `strong_above_absolute` | `mediumCorrelationThreshold` | 0.75 -> 0.75 | 0.6666; file: 0.6666 | yes |
| `use_output_name` | `autoOutputName` | True -> false | False; file: true | yes |
| `output_name` | `outputName` | st -> st |  | yes |
| `output_mode` | `outputMode` | File -> file<br>Screen -> screen | Screen; file: screen | yes |
| `output_format` | `outputFileType` | HTML -> (no change)<br>Output -> output_object<br>Text -> text | HTML | yes |
| `full_filename` | `outputFilePath` | D:/out/s.html -> D:/out/s.html |  | yes |

## Set failed

(none)

## Set succeeded but file unchanged

- `label_correlations`
- `strong_above_probability`

## Observations

- Class `StatisticsNode`. `examine` -> `examineFields/field[n]`; `correlate` -> `correlationFields/field[n]`; `statistics` -> `statistics/setElement[n]` with lowercase names (`count mean sum min max range variance sdev semean median mode`); a single element is unindexed, an empty list gives an empty element, the default set is stored in a non-input order.
- `correlation_mode` -> `correlationStrengthMode` (`probability` default | `absolute`); `weak_label`/`medium_label`/`strong_label` -> `weakCorrelationLabel`/`mediumCorrelationLabel`/`strongCorrelationLabel` (Weak/Medium/Strong); `weak_below_probability` -> `weakProbabilityThreshold` (0.9); `weak_below_absolute` -> `weakCorrelationThreshold` (0.3333); `strong_above_absolute` was written to `mediumCorrelationThreshold` (default 0.6666), so the element name does not match the property name; `strong_above_probability = 0.95` and `label_correlations = True` changed nothing (equal to defaults).
- Output properties as for Table; `output_format` Text/HTML/Output -> `outputFileType` text/html/output_object.
