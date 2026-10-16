# `reclassify` node (`ReclassifyNode`)

Probe `xtask/oracle/probes/settings_reclassify.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `change: instantiate fields by running a temporary Table node on the source` | (none) |  -> (no change) |  | no |
| `mode` | `reclassifyMode` | Multiple -> multiple<br>Single -> single | Single; file: single | yes |
| `replace_field` | `reclassifyReplaceField` | True -> true | False; file: false | yes |
| `field` | `singleFieldSelection` | Drug -> Drug |  | yes |
| `new_name` | `singleFieldName` | Chemical -> Chemical |  | yes |
| `fields` | `multipleFieldList`<br>also: `multipleFieldList/field[0]`, `multipleFieldList/field[1]` | ['Drug', 'BP'] -> (removed) | [] | yes |
| `name_extension` | `multipleExtensionName` | _reclass -> _reclass |  | yes |
| `add_as` | `multipleExtensionPosition` | Prefix -> prefix<br>Suffix -> suffix | Suffix; file: suffix | yes |
| `reclassify[field]` | `reclassifyTable/rows/row[$field]/newValue`<br>also: `reclassifyTable/rows/row/newValue`, `reclassifyTable/rows/row/oldValue`, `reclassifyTable/rows/row[$field]/oldValue` | BrandA -> reclassifyTable/rows=(removed)<br>BrandB -> BrandB<br>BrandC -> BrandC |  | yes |
| `use_default` | `useDefault` | True -> true<br>False -> false | False; file: false | yes |
| `default` | `defaultValue` | BrandX -> BrandX | undef; file: undef | yes |
| `pick_list` | `reclassifyPickList`<br>also: `reclassifyPickList/value[0]`, `reclassifyPickList/value[1]`, `reclassifyPickList/value[2]` | ['BrandX', 'Placebo', 'Generic'] -> (removed) | [] | yes |

## Set failed

(none)

## Set succeeded but file unchanged

- `change: instantiate fields by running a temporary Table node on the source`

## Observations

- Class `ReclassifyNode`. `mode` -> `reclassifyMode` (`single` default | `multiple`); `replace_field` -> `reclassifyReplaceField`; `field` -> `singleFieldSelection`; `new_name` -> `singleFieldName`; `fields` -> `multipleFieldList/field[n]`; `name_extension` -> `multipleExtensionName` (default empty); `add_as` -> `multipleExtensionPosition` (`suffix` default | `prefix`); `use_default` -> `useDefault`; `default` -> `defaultValue` (default `undef`); `pick_list` -> `reclassifyPickList/value[n]`.
- `reclassify` is KEYED by old value: `reclassifyTable/rows/row[n]/{oldValue,newValue}` in insertion order (a single row has no index; indices appear from 2 rows). Re-setting an existing key updates that row's `newValue` in place.
