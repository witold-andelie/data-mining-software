# `restructure` node (`RestructureNode`)

Probe `xtask/oracle/probes/settings_restructure.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `change: instantiate fields by running a temporary Table node on the source` | (none) |  -> (no change) |  | no |
| `fields_from[field]` | `createRestructuredFields`<br>also: `createRestructuredFields/persistentObjectListMember[0]/srcFi...`, `createRestructuredFields/persistentObjectListMember[1]/srcFi...`, `createRestructuredFields/persistentObjectListMember[2]/srcFi...` | ['drugA', 'drugX'] -> (removed)<br>['all'] -> createRestructuredFields/persistentObjectListMembe...=true<br>['drugA'] -> (no change) | [] | yes |
| `include_field_name` | `restructureIncludeFieldNames`<br>also: `createRestructuredFields/persistentObjectListMember[0]/inclu...`, `createRestructuredFields/persistentObjectListMember[1]/inclu...`, `createRestructuredFields/persistentObjectListMember[2]/inclu...` | False -> false | True; file: true | yes |
| `value_mode` | `restructureValuesMode` | OtherFields -> (no change)<br>Flags -> Flags | OtherFields | yes |
| `value_fields` | `restructureValueFields`<br>also: `restructureValueFields/field[0]`, `restructureValueFields/field[1]` | ['Age', 'Na'] -> (removed) | [] | yes |

## Set failed

(none)

## Set succeeded but file unchanged

- `change: instantiate fields by running a temporary Table node on the source`

## Observations

- Class `RestructureNode`. `fields_from[field]` is KEYED by source field and takes a list of category values: every (field, value) pair becomes one `createRestructuredFields/persistentObjectListMember[i]/{srcFieldName,srcFieldValue,includeFieldNames}` in creation order. Re-setting an existing key with a subset did NOT remove members (no change).
- `include_field_name` -> `restructureIncludeFieldNames` and is copied into every member's `includeFieldNames`. `value_mode` -> `restructureValuesMode` (`OtherFields` default | `Flags`); `value_fields` -> `restructureValueFields/field[n]`.
