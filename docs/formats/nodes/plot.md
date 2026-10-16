# `plot` node (`ScatterPlotNode`)

Probe `xtask/oracle/probes/settings_plot.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `three_D` | `graph3D` | True -> true | False; file: false | yes |
| `x_field` | `graphXField` | Age -> Age |  | yes |
| `y_field` | `graphYField` | Na -> Na |  | yes |
| `z_field` | `graphZField` | K -> K |  | yes |
| `color_field` | `graphColorField` | Drug -> Drug |  | yes |
| `size_field` | `graphSizeField` | Age -> Age |  | yes |
| `shape_field` | `graphShapeField` | Sex -> Sex |  | yes |
| `panel_field` | `graphPanelField` | BP -> BP |  | yes |
| `animation_field` | `graphAnimationField` | Cholesterol -> Cholesterol |  | yes |
| `transp_field` | `graphTransparencyField` | Drug -> Drug |  | yes |
| `overlay_type` | `graphOverlayType` | Smoother -> Smoother<br>Function -> Function<br>None -> None | None; file: None | yes |
| `overlay_expression` | `graphOverlayFunction` | Na * 2 -> Na * 2 |  | yes |
| `use_overlay_expr` | `graphIncludeOverlayFunction` | True -> true | False; file: false | yes |
| `style` | `scatterShowLine` | Line -> true<br>Point -> false | Point; file: false | yes |
| `point_type` | (none) | Rectangle -> (no change)<br>Dot -> (no change)<br>Triangle -> (no change)<br>Hexagon -> (no change)<br>Plus -> (no change)<br>Pentagon -> (no change)<br>Star -> (no change)<br>BowTie -> (no change)<br>HorizontalDash -> (no change)<br>VerticalDash -> (no change) |  | no |
| `x_mode` | `scatterXMode` | Overlay -> Overlay<br>Sort -> Sort<br>AsRead -> AsRead | Sort; file: Sort | yes |
| `x_range_mode` | `automaticRangeX` | UserDefined -> false<br>Automatic -> true | Automatic; file: true | yes |
| `x_range_min` | `userDefinedRangeXMin` | -2.5 -> -2.5 | 0.0; file: 0.0 | yes |
| `x_range_max` | `userDefinedRangeXMax` | 99.5 -> 99.5 | 100.0; file: 100.0 | yes |
| `x_label_auto` | `graphAutomaticXLabel` | False -> false | True; file: true | yes |
| `x_label` | `graphXLabel` | lab x -> lab x |  | yes |
| `y_range_mode` | `automaticRangeY` | UserDefined -> false<br>Automatic -> true | Automatic; file: true | yes |
| `y_range_min` | `userDefinedRangeYMin` | -2.5 -> -2.5 | 0.0; file: 0.0 | yes |
| `y_range_max` | `userDefinedRangeYMax` | 99.5 -> 99.5 | 100.0; file: 100.0 | yes |
| `y_label_auto` | `graphAutomaticYLabel` | False -> false | True; file: true | yes |
| `y_label` | `graphYLabel` | lab y -> lab y |  | yes |
| `z_range_mode` | `automaticRangeZ` | UserDefined -> false<br>Automatic -> true | Automatic; file: true | yes |
| `z_range_min` | `userDefinedRangeZMin` | -2.5 -> -2.5 | 0.0; file: 0.0 | yes |
| `z_range_max` | `userDefinedRangeZMax` | 99.5 -> 99.5 | 100.0; file: 100.0 | yes |
| `z_label_auto` | `graphAutomaticZLabel` | False -> false | True; file: true | yes |
| `z_label` | `graphZLabel` | lab z -> lab z |  | yes |
| `jitter` | `graphJitter` | True -> true | False; file: false | yes |
| `records_limit` | `graphLargeDatasetReadThreshold` | 1234 -> 1234<br>-1 -> 0 | 2000; file: 2000 | yes |
| `if_over_limit` | `graphLargeDatasetReadOption` | PlotSample -> Sample<br>PlotAll -> All<br>PlotBins -> Bin | PlotBins; file: Bin | yes |
| `use_grid` | `graphGridlines` | False -> false | True; file: true | yes |
| `graph_background` | `graphBackgroundColor` | #87CEEB -> -7876885 | -3355444; file: -3355444 | yes |
| `page_background` | `graphPageBackgroundColor` | #112233 -> -15654349 | -1; file: -1 | yes |
| `output_mode` | `outputMode` | File -> file<br>Screen -> screen | Screen; file: screen | yes |
| `output_format` | `graphOutputFileType` | BMP -> bmp<br>JPEG -> jpg<br>PNG -> png<br>HTML -> html<br>Output -> output_object | JPEG; file: jpg | yes |
| `full_filename` | `outputFilePath` | D:/out/g.png -> D:/out/g.png |  | yes |
| `title` | `graphTitle` | My title -> My title |  | yes |
| `caption` | `graphCaption` | My caption -> My caption |  | yes |
| `use_graph_size` | `useGraphSize` | True -> true | False; file: false | yes |
| `graph_width` | `graphWidth` | 640 -> 640 | 575; file: 575 | yes |
| `graph_height` | `graphHeight` | 480 -> 480 | 475; file: 475 | yes |

## Set failed

(none)

## Set succeeded but file unchanged

- `point_type`

## Observations

- Class `ScatterPlotNode`. `three_D` -> `graph3D`; `overlay_type` -> `graphOverlayType` (`None` default | `Smoother` | `Function`, stored as given); `overlay_expression` -> `graphOverlayFunction`; `use_overlay_expr` -> `graphIncludeOverlayFunction`; `style` -> `scatterShowLine` (Line=true, Point=false); `x_mode` -> `scatterXMode` (`Sort` default | `Overlay` | `AsRead`); `{x,y,z}_range_mode` -> `automaticRange{X,Y,Z}` (inverse boolean) with `userDefinedRange{X,Y,Z}{Min,Max}`; `jitter` -> `graphJitter`; `records_limit` -> `graphLargeDatasetReadThreshold` (default 2000; -1 clamps to 0); `if_over_limit` -> `graphLargeDatasetReadOption`: PlotBins=`Bin` (default), PlotSample=`Sample`, PlotAll=`All`. `size_field` -> `graphSizeField`, `shape_field` -> `graphShapeField`, `transp_field` -> `graphTransparencyField`, `z_field` -> `graphZField`.
- `point_type`: all 21 values are accepted and read back, but the saved file does NOT change (not persisted for this node, or stored outside `properties`; unresolved).
- Graph common properties: `title` `graphTitle`, `caption` `graphCaption`, `use_graph_size` `useGraphSize` (default false), `graph_width` `graphWidth` (default 575), `graph_height` `graphHeight` (default 475), `output_mode` `outputMode` (`screen` | `file`), `output_format` `graphOutputFileType` (default `jpg`; BMP=`bmp`, JPEG=`jpg`, PNG=`png`, HTML=`html`, Output=`output_object`), `full_filename` `outputFilePath`, `use_output_name`/`output_name` `autoOutputName` (inverse) / `outputName`.
- Colors: `graph_background`/`page_background` accept `#RRGGBB` strings (named colors such as `red` are rejected) and are stored as a signed 32-bit ARGB integer: `#87CEEB` -> `-7876885` (0xFF87CEEB), `#112233` -> `-15654349`; defaults `-3355444` (0xFFCCCCCC) and `-1` (white).
- Axis/overlay fields (`x_field`, `color_field`, `panel_field`, `animation_field`, ...) are plain strings `graphXField`, `graphColorField`, `graphPanelField`, `graphAnimationField`; empty means off. Labels: `x_label_auto` `graphAutomaticXLabel` (default true) + `x_label` `graphXLabel`; same for Y/Z; `use_grid` `graphGridlines` (default true); `normalize` `graphNormalizeData`.
