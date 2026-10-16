# `filter` node (`FilterNode`)

Probe `xtask/oracle/probes/settings_filter.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `change: instantiate fields by running a temporary Table node on the source` | (none) |  -> (no change) |  | no |
| `default_include` | `filterTable/defaultDrop`<br>also: `filterTable/rows/row[$field]/drop` | False -> true<br>True -> false<br>False -> true | True; file: false | yes |
| `include[field]` | `filterTable/rows/row[$field]/drop`<br>also: `filterTable/rows/row[$field]/name`, `filterTable/rows` | False -> true<br>False -> true<br>True -> (removed)<br>True -> false<br>False -> (no change) | True; file: absent | yes |
| `new_name[field]` | `filterTable/rows/row[$field]/rename`<br>also: `filterTable/rows/row[$field]/name` | Chemical -> Chemical<br>Sodium -> Sodium<br> -> (no change)<br>Gender -> Gender<br>x -> x | Drug; file: absent | yes |

## Set failed

(none)

## Set succeeded but file unchanged

- `change: instantiate fields by running a temporary Table node on the source`

## Observations

- Class `FilterNode`; everything is in `filterTable`: `defaultDrop` (inverse of `default_include`; default false) and `rows/row[name]/{name,drop,rename}`.
- `include[f]=False` writes `row/drop=true`; `include[f]=True` on a dropped field REMOVES the row again (default state is no row). `include[f]` with default_include flipped later rewrites existing rows' `drop` (s011: row Drug drop true -> false when default_include False) while rows are only created for explicitly touched fields.
- `new_name[f]` writes `row/rename` (and `name`); setting it to the empty string is accepted but leaves `rename` unchanged (reads back the previous name; there is no way to clear a rename via empty string).
- Keys are not validated: `new_name` for a non-existent field creates a row for it; `include[...]=False` on a non-existent field changes nothing.
- Order of rows in the file follows creation order of the touches, not the field order.

## Read-only observations

- `include_Drug`: `True`
- `new_name_Drug`: `Chemical`
- `new_name_K`: `K`
