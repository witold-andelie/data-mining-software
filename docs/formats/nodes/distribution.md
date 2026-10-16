# `distribution` node (`DistributionNode`)

Probe `xtask/oracle/probes/settings_distribution.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `plot` | `distributionSelectionMode` | Flags -> flags<br>Specified -> selectedFields | Specified; file: selectedFields | yes |
| `x_field` | `graphXField` | Drug -> Drug |  | yes |
| `color_field` | `graphColorField` | Sex -> Sex |  | yes |
| `normalize` | `graphNormalizeData` | True -> true | False; file: false | yes |
| `sort_mode` | `distributionSortType` | Alphabetic -> (no change)<br>ByOccurence -> byCount | Alphabetic | yes |
| `use_proportional_scale` | `distributionProportionalScale` | True -> true | False; file: false | yes |
| `x_label_auto` | `graphAutomaticXLabel` | False -> false | True; file: true | yes |
| `x_label` | `graphXLabel` | my x -> my x |  | yes |
| `y_label_auto` | `graphAutomaticYLabel` | False -> false | True; file: true | yes |
| `y_label` | `graphYLabel` | my y -> my y |  | yes |
| `use_grid` | (none) | False -> (no change) | False | no |
| `graph_background` | `graphBackgroundColor` | #87CEEB -> -7876885 | -3355444; file: -3355444 | yes |
| `page_background` | `graphPageBackgroundColor` | #112233 -> -15654349 | -1; file: -1 | yes |
| `output_mode` | `outputMode` | File -> file<br>Screen -> screen | Screen; file: screen | yes |
| `output_format` | `graphOutputFileType` | BMP -> bmp<br>JPEG -> jpg<br>PNG -> png<br>HTML -> html<br>Output -> output_object | JPEG; file: jpg | yes |
| `full_filename` | `outputFilePath` | D:/out/g.png -> D:/out/g.png |  | yes |
| `use_output_name` | `autoOutputName` | True -> false | False; file: true | yes |
| `output_name` | `outputName` | dist -> dist |  | yes |
| `title` | `graphTitle` | My title -> My title |  | yes |
| `caption` | `graphCaption` | My caption -> My caption |  | yes |
| `use_graph_size` | `useGraphSize` | True -> true | False; file: false | yes |
| `graph_width` | `graphWidth` | 640 -> 640 | 575; file: 575 | yes |
| `graph_height` | `graphHeight` | 480 -> 480 | 475; file: 475 | yes |

## Set failed

(none)

## Set succeeded but file unchanged

- `use_grid`

## Observations

- Class `DistributionNode`. `plot` -> `distributionSelectionMode`: Flags=`flags`, Specified=`selectedFields` (default; the guide's `SelectedFields` is rejected). `sort_mode` -> `distributionSortType`: Alphabetic=`alphabetic` (default), ByOccurence=`byCount`. `use_proportional_scale` -> `distributionProportionalScale`. `x_field` -> `graphXField`.
- Graph common properties: `title` `graphTitle`, `caption` `graphCaption`, `use_graph_size` `useGraphSize` (default false), `graph_width` `graphWidth` (default 575), `graph_height` `graphHeight` (default 475), `output_mode` `outputMode` (`screen` | `file`), `output_format` `graphOutputFileType` (default `jpg`; BMP=`bmp`, JPEG=`jpg`, PNG=`png`, HTML=`html`, Output=`output_object`), `full_filename` `outputFilePath`, `use_output_name`/`output_name` `autoOutputName` (inverse) / `outputName`.
- Colors: `graph_background`/`page_background` accept `#RRGGBB` strings (named colors such as `red` are rejected) and are stored as a signed 32-bit ARGB integer: `#87CEEB` -> `-7876885` (0xFF87CEEB), `#112233` -> `-15654349`; defaults `-3355444` (0xFFCCCCCC) and `-1` (white).
- Axis/overlay fields (`x_field`, `color_field`, `panel_field`, `animation_field`, ...) are plain strings `graphXField`, `graphColorField`, `graphPanelField`, `graphAnimationField`; empty means off. Labels: `x_label_auto` `graphAutomaticXLabel` (default true) + `x_label` `graphXLabel`; same for Y/Z; `use_grid` `graphGridlines` (default true); `normalize` `graphNormalizeData`.
