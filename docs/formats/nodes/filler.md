# `filler` node (`FillerNode`)

Probe `xtask/oracle/probes/settings_filler.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `fields` | `fillerSelectedFields`<br>also: `fillerSelectedFields/field`, `fillerSelectedFields/field[0]`, `fillerSelectedFields/field[1]` | ['Age'] -> (removed)<br>['Age', 'Na'] -> fillerSelectedFields/field=(removed)<br>[] -> <br>['Drug'] -> (removed) | [] | yes |
| `replace_mode` | `fillerReplaceMode` | Conditional -> (no change)<br>Blank -> blanks<br>Null -> nulls<br>BlankAndNull -> blanksAndNulls<br>Always -> always | Conditional | yes |
| `condition` | `fillerCondition` | (Age > 60) and (Sex = "M") -> (Age > 60) and (Sex = "M") | @BLANK(@FIELD); file: @BLANK(@FIELD) | yes |
| `replace_with` | `fillerReplaceExpression` | "old man" -> "old man"<br>@FIELD * 2 -> @FIELD * 2<br> ->  | 0; file: 0 | yes |

## Set failed

(none)

## Set succeeded but file unchanged

(none)

## Observations

- Class `FillerNode`. `fields` -> `fillerSelectedFields`: a single field is `<field>name</field>` (no index), two or more are `field[0]`, `field[1]`..., empty list leaves an empty element. `replace_mode` -> `fillerReplaceMode`: Conditional=`conditional` (default), Blank=`blanks`, Null=`nulls`, BlankAndNull=`blanksAndNulls`, Always=`always`. `condition` -> `fillerCondition` (default `@BLANK(@FIELD)`); `replace_with` -> `fillerReplaceExpression` (default `0`).
