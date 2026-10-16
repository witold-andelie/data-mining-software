# `binning` node (`BinningNode`)

Probe `xtask/oracle/probes/settings_binning.py`; each row is derived from a differential save (one property set per saved file, compared with the previous file). Paths are relative to the node's `properties` element unless prefixed with `..` (relative to the `node` element) or `@Class` (another node). Keyed properties are per field: `row[$field]` is the row whose `name` child equals the field name. `(no change)` means the set succeeded but the saved file did not change. Generated tables are followed by hand-written observations.

| scripting property | file path | encoding (set value -> new file value) | default (API; file) | verified |
|---|---|---|---|---|
| `fields` | `binningFields`<br>also: `../@SettingsChanged`, `binningFields/field[0]`, `binningFields/field[1]` | ['Na', 'K'] -> (removed) | [] | yes |
| `method` | `binningMethod`<br>also: `../@RecomputeNecessary` | EqualCount -> equalFreq<br>Rank -> rank<br>SDev -> aroundMean<br>Optimal -> optimal<br>FixedWidth -> fixedWidth | FixedWidth; file: fixedWidth | yes |
| `recalculate_bins` | `recalculateBins` | Always -> (no change)<br>IfNecessary -> ifnecessary | Always | yes |
| `fixed_width_name_extension` | `fixedBinExtensionName` | _bin -> _bin | _BIN; file: _BIN | yes |
| `fixed_width_add_as` | `fixedBinExtensionPosition` | Prefix -> prefix<br>Suffix -> suffix | Suffix; file: suffix | yes |
| `fixed_bin_method` | `fixedBinMethod` | Count -> count<br>Width -> width | Width; file: width | yes |
| `fixed_bin_count` | `fixedBinCount` | 7 -> 7 | 10; file: 10 | yes |
| `fixed_bin_width` | `fixedBinWidth` | 3.5 -> 3.5 | 10.0; file: 10.0 | yes |
| `equal_count_name_extension` | `binTileExtensionName` | _tl -> _tl | _TILE; file: _TILE | yes |
| `equal_count_add_as` | `binTileExtensionPosition` | Prefix -> prefix<br>Suffix -> suffix | Suffix; file: suffix | yes |
| `tile4` | `binTile4` | True -> true | False; file: false | yes |
| `tile5` | `binTile5` | True -> true | False; file: false | yes |
| `tile10` | (none) | True -> (no change) | True | no |
| `tile20` | `binTile20` | True -> true | False; file: false | yes |
| `tile100` | `binTile100` | True -> true | False; file: false | yes |
| `use_custom_tile` | `useBinTileN` | True -> true | False; file: false | yes |
| `custom_tile_name_extension` | `binTileNExtensionName` | _ct -> _ct | _TILEN; file: _TILEN | yes |
| `custom_tile_add_as` | `binTileNExtensionPosition` | Prefix -> prefix<br>Suffix -> suffix | Suffix; file: suffix | yes |
| `custom_tile` | (none) | 8 -> (no change) | 8 | no |
| `equal_count_method` | `tilingMethod` | ValueSum -> sum<br>RecordCount -> count | RecordCount; file: count | yes |
| `tied_values_method` | `tiedValuesMethod` | Current -> low<br>Random -> random<br>Next -> high | Next; file: high | yes |
| `rank_order` | `reverseRank` | Descending -> true<br>Ascending -> false | Ascending; file: false | yes |
| `rank_add_as` | `binRankExtensionPositions` | Prefix -> prefix<br>Suffix -> suffix | Suffix; file: suffix | yes |
| `rank` | `binByRank` | False -> false | True; file: true | yes |
| `rank_name_extension` | `binRankExtensionName` | _rk -> _rk | _RANK; file: _RANK | yes |
| `rank_fractional` | (none) | True -> (no change) | True | no |
| `rank_fractional_name_extension` | `binFrRankExtensionName` | _frk -> _frk | _F_RANK; file: _F_RANK | yes |
| `rank_pct` | (none) | True -> (no change) | True | no |
| `rank_pct_name_extension` | `binPcRankExtensionName` | _prk -> _prk | _P_RANK; file: _P_RANK | yes |
| `sdev_name_extension` | `binMeanStdDevExtensionName` | _sd -> _sd | _SDBIN; file: _SDBIN | yes |
| `sdev_add_as` | `binMeanStdDevExtensionPosition` | Prefix -> prefix<br>Suffix -> suffix | Suffix; file: suffix | yes |
| `sdev_count` | `binMeanStdDevCount` | One -> (no change)<br>Two -> 2<br>Three -> 3 | One | yes |
| `optimal_name_extension` | `binOptimalExtensionName` | _op -> _op | _OPTIMAL; file: _OPTIMAL | yes |
| `optimal_add_as` | `binOptimalExtensionPosition` | Prefix -> prefix<br>Suffix -> suffix | Suffix; file: suffix | yes |
| `optimal_supervisor_field` | `binOptimalSupervisorField` | Drug -> Drug |  | yes |
| `optimal_merge_bins` | (none) | False -> (no change) | False | no |
| `optimal_small_bin_threshold` | `binOptimalsmallBinDetectionThreshold` | 20 -> 1.0 | 0.2; file: 0.2 | yes |
| `optimal_pre_bin` | `binOptimalPreBin` | False -> false | True; file: true | yes |
| `optimal_max_bins` | `binOptimalMaxBins` | 9 -> 9 | 1000; file: 1000 | yes |
| `optimal_lower_end_point` | `binEndpointsOpenClosed` | Exclusive -> lowerExclusive<br>Inclusive -> lowerInclusive | Inclusive; file: lowerInclusive | yes |
| `optimal_first_bin` | `binEndpointsLowestBounding` | Bounded -> bounded<br>Unbounded -> unbounded | Unbounded; file: unbounded | yes |
| `optimal_last_bin` | `binEndpointsHighestBounding` | Bounded -> bounded<br>Unbounded -> unbounded | Unbounded; file: unbounded | yes |

## Set failed

(none)

## Set succeeded but file unchanged

- `tile10`
- `custom_tile`
- `rank_fractional`
- `rank_pct`
- `optimal_merge_bins`

## Observations

- Class `BinningNode`. The node element carries two attributes outside `properties`: `@SettingsChanged` (false -> true when `fields` is set) and `@RecomputeNecessary` (false -> true on the first method change). `fields` -> `binningFields/field[n]`; `method` -> `binningMethod`: FixedWidth=`fixedWidth` (default), EqualCount=`equalFreq`, Rank=`rank`, SDev=`aroundMean`, Optimal=`optimal`. The guide's `rcalculate_bins` is undefined; the working name is `recalculate_bins` -> `recalculateBins` (`always` default | `ifnecessary`).
- Fixed width: `fixed_width_name_extension` `fixedBinExtensionName` (`_BIN`), `fixed_width_add_as` `fixedBinExtensionPosition` (`suffix`), `fixed_bin_method` `fixedBinMethod` (`width` default | `count`), `fixed_bin_count` `fixedBinCount` (10), `fixed_bin_width` `fixedBinWidth` (10.0).
- Tiles: `equal_count_name_extension`/`equal_count_add_as` -> `binTileExtensionName` (`_TILE`) / `binTileExtensionPosition`; `tile4/5/20/100` -> `binTile4` etc. (default false; `tile10` is already true so no change); `use_custom_tile` -> `useBinTileN`; `custom_tile_*` -> `binTileNExtensionName` (`_TILEN`) / `binTileNExtensionPosition`; `custom_tile = 8` did not change the file (unresolved); `equal_count_method` -> `tilingMethod` (RecordCount=`count` default, ValueSum=`sum`); `tied_values_method` -> `tiedValuesMethod` (Current=`low`, Next=`high` default, Random=`random`).
- Rank: `rank_order` -> `reverseRank` (Descending=true); `rank_add_as` -> `binRankExtensionPositions`; `rank` -> `binByRank`; extension names `binRankExtensionName` (`_RANK`), `binFrRankExtensionName` (`_F_RANK`), `binPcRankExtensionName` (`_P_RANK`); `rank_fractional`/`rank_pct` = True changed nothing (the flag element was not identified).
- SDev: `sdev_name_extension` `binMeanStdDevExtensionName` (`_SDBIN`), `sdev_add_as` `binMeanStdDevExtensionPosition`, `sdev_count` `binMeanStdDevCount` (One=1, Two=2, Three=3).
- Optimal: `optimal_name_extension` `binOptimalExtensionName` (`_OPTIMAL`), `optimal_add_as` `binOptimalExtensionPosition`, `optimal_supervisor_field` `binOptimalSupervisorField`, `optimal_small_bin_threshold` `binOptimalsmallBinDetectionThreshold` (default 0.2; 20 was clamped to 1.0, so it is a proportion), `optimal_pre_bin` `binOptimalPreBin`, `optimal_max_bins` `binOptimalMaxBins` (1000), `optimal_lower_end_point` `binEndpointsOpenClosed` (`lowerInclusive` | `lowerExclusive`), `optimal_first_bin`/`optimal_last_bin` `binEndpointsLowestBounding`/`binEndpointsHighestBounding` (`bounded` | `unbounded`); `optimal_merge_bins = False` changed nothing.
