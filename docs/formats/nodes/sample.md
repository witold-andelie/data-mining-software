# `sample` node (`SampleNode`)

Probe `xtask/oracle/probes/settings_sample.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `change: instantiate fields by running a temporary Table node on the source` | (none) |  -> (no change) |  | no |
| `mode` | `sampleMode` | Discard -> discard<br>Include -> pass | Include; file: pass | yes |
| `method` | `sampleProcedure` | Complex -> complex<br>Simple -> simple<br>Complex -> complex | Simple; file: simple | yes |
| `sample_type` | `sampleMethod` | OneInN -> oneInN<br>RandomPct -> random<br>First -> firstN | First; file: firstN | yes |
| `first_n` | `sampleFirstN` | 500 -> 500<br>-3 -> 0 | 10000; file: 10000 | yes |
| `one_in_n` | `sampleOneInN` | 7 -> 7<br>0 -> 1 | 2; file: 2 | yes |
| `rand_pct` | `sampleRandom` | 12.5 -> 12.5<br>250 -> 100.0 | 50.0; file: 50.0 | yes |
| `use_max_size` | `sampleUseMaxSize` | True -> true | False; file: false | yes |
| `maximum_size` | `sampleMaxSize` | 1000 -> 1000 | 10000; file: 10000 | yes |
| `set_random_seed` | `sampleUseRandomSeed` | True -> true | False; file: false | yes |
| `random_seed` | `sampleRandomSeed` | 4242 -> 4242<br>-1 -> 0 | 1217380; file: 1217380 | yes |
| `complex_sample_type` | `sampleComplexType` | Systematic -> systematic<br>Random -> random | Random; file: random | yes |
| `sample_units` | `sampleUnits` | Counts -> counts<br>Proportions -> proportions<br>Proportions -> (no change)<br>Counts -> counts | Proportions; file: proportions | yes |
| `sample_size_proportions` | `sampleSizeProportions` | Fixed -> (no change)<br>Custom -> custom<br>Variable -> variable<br>Custom -> custom | Fixed | yes |
| `sample_size_counts` | `sampleSizeCounts` | Fixed -> (no change)<br>Custom -> custom<br>Variable -> variable<br>Custom -> custom | Fixed | yes |
| `fixed_proportions` | `sampleFixedProportions` | 0.4 -> 0.4 | 0.5; file: 0.5 | yes |
| `fixed_counts` | `sampleFixedCounts` | 77 -> 77 | 10000; file: 10000 | yes |
| `variable_proportions` | `sampleVariableProportions` | Age -> Age |  | yes |
| `variable_counts` | `sampleVariableCounts` | Age -> Age |  | yes |
| `use_min_stratum_size` | `sampleUseMinStratumSize` | True -> true | False; file: false | yes |
| `minimum_stratum_size` | `sampleMinimumStratumSize` | 5 -> 5 | 100; file: 100 | yes |
| `use_max_stratum_size` | `sampleUseMaxStratumSize` | True -> true | False; file: false | yes |
| `maximum_stratum_size` | `sampleMaximumStratumSize` | 50 -> 50 | 10000; file: 10000 | yes |
| `clusters` | `sampleClusters` | Drug -> Drug |  | yes |
| `stratify_by` | `sampleStratifyBy`<br>also: `sampleStratifyBy/field[0]`, `sampleStratifyBy/field[1]`, `sampleCountsReadStrataFieldList` | ['Sex', 'BP'] -> (removed) | [] | yes |
| `specify_input_weight` | `sampleSpecifyInputWeight` | True -> true | False; file: false | yes |
| `input_weight` | `sampleInputWeight` | Age -> Age |  | yes |
| `new_output_weight` | `sampleNewOutputWeight` | NewWeight -> NewWeight | SampleWeight; file: SampleWeight | yes |
| `sizes_proportions` | `sampleProportionsSizes`<br>also: `sampleProportionsSizes/persistentObjectListMember[0]/SizeVal...`, `sampleProportionsSizes/persistentObjectListMember[0]/Strata[...`, `sampleProportionsSizes/persistentObjectListMember[0]/Strata[...` | [['M', 'HIGH', 'Default'], ['F', 'HIGH',... -> (removed) | [] | yes |
| `default_proportion` | `sampleDefaultProportion` | 0.2 -> 0.2 | 0.5; file: 0.5 | yes |
| `sizes_counts` | `sampleCountsSizes`<br>also: `sampleCountsSizes/persistentObjectListMember[0]/SizeValue`, `sampleCountsSizes/persistentObjectListMember[0]/Strata[0]`, `sampleCountsSizes/persistentObjectListMember[0]/Strata[1]` | [['M', 'HIGH', 10], ['F', 'HIGH', 20]] -> (removed) | [] | yes |
| `default_count` | `sampleDefaultCount` | 15 -> 15 | 10000; file: 10000 | yes |

## Set failed

(none)

## Set succeeded but file unchanged

- `change: instantiate fields by running a temporary Table node on the source`

## Observations

- Class `SampleNode`. `method` -> `sampleProcedure` (`simple` default | `complex`); `mode` -> `sampleMode` (`pass` = Include, default | `discard`); `sample_type` -> `sampleMethod` (`firstN` default | `oneInN` | `random`); `first_n` -> `sampleFirstN` (default 10000; -3 clamps to 0); `one_in_n` -> `sampleOneInN` (default 2; 0 clamps to 1); `rand_pct` -> `sampleRandom` (default 50.0; 250 clamps to 100.0); `use_max_size`/`maximum_size` -> `sampleUseMaxSize`/`sampleMaxSize` (10000); `set_random_seed`/`random_seed` -> `sampleUseRandomSeed`/`sampleRandomSeed` (default seed is generated per node, e.g. 1217380; -1 clamps to 0).
- Complex sampling: `complex_sample_type` -> `sampleComplexType` (`random` | `systematic`); `sample_units` -> `sampleUnits` (`proportions` default | `counts`); `sample_size_proportions`/`sample_size_counts` -> `sampleSizeProportions`/`sampleSizeCounts` (`fixed` | `custom` | `variable`); `fixed_proportions` -> `sampleFixedProportions` (0.5); `fixed_counts` -> `sampleFixedCounts` (10000); `variable_proportions`/`variable_counts` -> `sampleVariableProportions`/`sampleVariableCounts`; `use_min_stratum_size`/`minimum_stratum_size` -> `sampleUseMinStratumSize`/`sampleMinimumStratumSize` (100); `use_max_stratum_size`/`maximum_stratum_size` -> `sampleUseMaxStratumSize`/`sampleMaximumStratumSize` (10000); `clusters` -> `sampleClusters`; `specify_input_weight`/`input_weight`/`new_output_weight` -> `sampleSpecifyInputWeight`/`sampleInputWeight`/`sampleNewOutputWeight` (default `SampleWeight`); `default_proportion` -> `sampleDefaultProportion` (0.5); `default_count` -> `sampleDefaultCount` (10000).
- `stratify_by` is written TWICE: `sampleCountsReadStrataFieldList` and `sampleProportionsReadStrataFieldList`, each with `persistentObjectListMember[i]/ReadFieldName`. `sizes_proportions` / `sizes_counts` -> `sampleProportionsSizes` / `sampleCountsSizes` / `persistentObjectListMember[i]/{SizeValue, Strata[j]/@Stratum}` (SizeValue is `Default` or a number).
