# `append` node (`AppendNode`)

Probe `xtask/oracle/probes/settings_append.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `match_by` | `appendMode` | Name -> (no change)<br>Position -> position | Name | yes |
| `match_case` | `appendMatchCase` | True -> true | False; file: false | yes |
| `include_fields_from` | `appendIncludeAllFields` | All -> true<br>Main -> false | Main; file: false | yes |
| `create_tag_field` | `appendIncludeTag` | True -> true | False; file: false | yes |
| `tag_field_name` | `appendTagField` | Append_Flag -> Append_Flag<br> ->  | Input; file: Input | yes |

## Set failed

(none)

## Set succeeded but file unchanged

(none)

## Observations

- Class `AppendNode` (probed with two inputs). `match_by` -> `appendMode` (`name` default | `position`); `match_case` -> `appendMatchCase`; `include_fields_from` -> `appendIncludeAllFields` (All=true, Main=false, default false); `create_tag_field` -> `appendIncludeTag`; `tag_field_name` -> `appendTagField` (default `Input`; empty string accepted).
