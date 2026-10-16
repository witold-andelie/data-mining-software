# `derive` node (`DeriveNode`)

Probe `xtask/oracle/probes/settings_derive.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `use_custom_name` | `autoLabel` | True -> false | False; file: true | yes |
| `new_name` | `deriveSingleFieldName` | NaToK -> NaToK<br> -> <br>Na_to_K_ratio -> Na_to_K_ratio |  | yes |
| `mode` | `deriveMode` | Multiple -> multiple<br>Single -> single | Single; file: single | yes |
| `fields` | `deriveMultipleFieldList`<br>also: `deriveMultipleFieldList/field[0]`, `deriveMultipleFieldList/field[1]` | ['Na', 'K'] -> (removed) | [] | yes |
| `name_extension` | `deriveMultipleExtensionName` | _new -> _new | _Derive; file: _Derive | yes |
| `add_as` | `deriveMultipleExtensionPosition` | Prefix -> prefix<br>Suffix -> suffix | Suffix; file: suffix | yes |
| `result_type` | `deriveType` | Formula -> (no change)<br>Flag -> flag<br>Set -> set<br>State -> state<br>Count -> count<br>Conditional -> conditional<br>Formula -> formula<br>Flag -> flag<br>Set -> set<br>State -> state | Formula | yes |
| `formula_expr` | `deriveFormulaExpression` | Na / K -> Na / K |  | yes |
| `formula_measure_type` | `deriveFormulaFieldMeasure` | Range -> range<br>Flag -> flag<br>Set -> set<br>OrderedSet -> orderedSet<br>Typeless -> typeless<br>Discrete -> discrete<br>Collection -> collection<br>Range -> range<br>Collection -> collection<br>Geospatial -> geospatial | Default; file: auto | yes |
| `flag_expr` | `deriveFlagFieldCondition` | Drug = 'drugX' -> Drug = 'drugX' |  | yes |
| `flag_true` | `deriveFlagTrueValue` | yes -> yes | T; file: T | yes |
| `flag_false` | `deriveFlagFalseValue` | no -> no | F; file: F | yes |
| `set_default` | `deriveSetDefaultValue` | other -> other | default; file: default | yes |
| `set_value_cond[field]` | `deriveSetTable/rows`<br>also: `deriveSetTable/rows/row[$field]/name`, `deriveSetTable/rows/row[$field]/index`, `deriveSetTable/rows/row[$field]/condition` | Na > 0.8 -> (removed) |  | yes |
| `state_on_val` | `deriveStateOnValue` | 1 -> 1 | On; file: On | yes |
| `state_off_val` | `deriveStateOffValue` | 0 -> 0 | Off; file: Off | yes |
| `state_on_expression` | `deriveStateOnCondition` | Na > 0.8 -> Na > 0.8 |  | yes |
| `state_off_expression` | `deriveStateOffCondition` | Na < 0.5 -> Na < 0.5 |  | yes |
| `state_initial` | `deriveStateInitialState` | On -> on<br>Off -> off | Off; file: off | yes |
| `count_initial_val` | `deriveCountInitialValue` | 5 -> 5 | 0; file: 0 | yes |
| `count_inc_condition` | `deriveCountIncrementCondition` | Na > 0.7 -> Na > 0.7 |  | yes |
| `count_inc_expression` | `deriveCountIncrementByExpression` | @INDEX -> @INDEX | 1; file: 1 | yes |
| `count_reset_condition` | `deriveCountResetCondition` | Na < 0.6 -> Na < 0.6 |  | yes |
| `cond_if_cond` | `deriveConditionalIfCondition` | Age > 40 -> Age > 40 |  | yes |
| `cond_then_expr` | `deriveConditionalThenExpression` | Na * 2 -> Na * 2 |  | yes |
| `cond_else_expr` | `deriveConditionalElseExpression` | Na -> Na |  | yes |
| `collection_measure` | `deriveFormulaCollectionMeasure/CollectionType/@valueMeasure` | Geospatial -> geospatial<br>Range -> range<br>Flag -> flag<br>Set -> set<br>OrderedSet -> orderedSet<br>Typeless -> typeless | Typeless; file: typeless | yes |
| `geo_type` | `deriveFormulaGeoMeasure/Geo/@geoType` | Point -> (no change)<br>MultiPoint -> MultiPoint<br>LineString -> LineString<br>MultiLineString -> MultiLineString<br>Polygon -> Polygon<br>MultiPolygon -> MultiPolygon | Point | yes |
| `has_coordinate_system` | (none) | True -> (no change) | False | no |
| `coordinate_system` | `deriveFormulaGeoMeasure/Geo/@geoCsname` | ETRS_1989_EPSG_Arctic_zone_5-47 -> ETRS_1989_EPSG_Arctic_zone_5-47 |  | yes |

## Set failed

- `set_value_cond` = 'drugA' Drug = 'drugA': The value ''drugA' Drug = 'drugA'' is not valid for the property 'set_value_cond'
- `geo_type` = Typeless: The value 'Typeless' is not valid for the property 'geo_type'

## Set succeeded but file unchanged

- `has_coordinate_system`

## Observations

- Class `DeriveNode`. All result types keep their own settings simultaneously; `result_type` -> `deriveType` (`formula` default | `flag` | `set` | `state` | `count` | `conditional`), and the per-type properties are stored whether or not that type is selected.
- Property -> element: `new_name` `deriveSingleFieldName`; `mode` `deriveMode` (`single` | `multiple`); `fields` `deriveMultipleFieldList/field[n]`; `name_extension` `deriveMultipleExtensionName` (default `_Derive`); `add_as` `deriveMultipleExtensionPosition` (`suffix` default | `prefix`); `formula_expr` `deriveFormulaExpression`; `formula_measure_type` `deriveFormulaFieldMeasure` (`auto` default for Default, `range`, `flag`, `set`, `orderedSet`, `typeless`, `discrete`, `collection`, `geospatial`); `flag_expr` `deriveFlagFieldCondition`; `flag_true`/`flag_false` `deriveFlagTrueValue`/`FalseValue` (defaults `T`/`F`); `set_default` `deriveSetDefaultValue` (default `default`); `state_*` `deriveState{OnValue,OffValue,OnCondition,OffCondition,InitialState}` (defaults `On`, `Off`, empty, empty, `off`); `count_*` `deriveCount{InitialValue,IncrementCondition,IncrementByExpression,ResetCondition}` (defaults `0`, empty, `1`, empty); `cond_*` `deriveConditional{If,Then,Else}...`.
- `set_value_cond` is a KEYED property (key = set value name, value = condition): it is stored as `deriveSetTable/rows/row[name]/{name,condition,index}` (index 0-based, in creation order). Setting it un-keyed fails ("not valid").
- `collection_measure` -> `deriveFormulaCollectionMeasure/CollectionType/@valueMeasure` (default `typeless`); `geo_type` -> `deriveFormulaGeoMeasure/Geo/@geoType` (default `Point`, `Typeless` rejected); `coordinate_system` -> `.../Geo/@geoCsname`. `has_coordinate_system=True` read back False and changed nothing in the file when set before a coordinate system existed.
- Expression strings are stored verbatim (no CLEM parsing at set time); `new_name` empty string is accepted.
