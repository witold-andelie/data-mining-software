# `select` node (`SelectNode`)

Probe `xtask/oracle/probes/settings_select.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `mode` | `selectMode` | Discard -> discard<br>Include -> include | Include; file: include | yes |
| `condition` | `selectCondition` | Age < 18 -> Age < 18<br>Drug = "drugY" and Na > 0.7 -> Drug = "drugY" and Na > 0.7<br> -> <br>数据 = 1 -> 数据 = 1<br>xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx... -> xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx... |  | yes |

## Set failed

(none)

## Set succeeded but file unchanged

(none)

## Observations

- Class `SelectNode`. `mode` -> `selectMode` (`include` default | `discard`); `condition` -> `selectCondition`, stored verbatim (empty string ok, Chinese ok, 400 chars ok; the CLEM text is not parsed at set time).
