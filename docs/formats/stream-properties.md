# Stream properties (`stream/properties`)

Probe `xtask/oracle/probes/stream_formats.py`: each step sets one stream property (`stream.setPropertyValue`) and saves; every row is the change versus the previous saved file. Paths are children of `clementineStream/stream/properties`.

## Date and time formats

`dateFormat` / `timeFormat` hold an opaque id `formatNN`. Scripting value -> saved id (verified for every value of the guide's Table 42, plus `MM.DD.YY` which the guide omits from the stream table):

| scripting value | saved id |
|---|---|
| **date_format** (`dateFormat`) | |
| `DDMMYY` | `format1` |
| `MMDDYY` | `format2` |
| `YYMMDD` | `format3` |
| `YYYYMMDD` | `format4` |
| `YYYYDDD` | `format24` |
| `DAY` | `format25` |
| `MONTH` | `format26` |
| `DD-MM-YY` | `format9` |
| `DD-MM-YYYY` | `format10` |
| `MM-DD-YY` | `format11` |
| `MM-DD-YYYY` | `format12` |
| `DD-MON-YY` | `format17` |
| `DD-MON-YYYY` | `format20` |
| `YYYY-MM-DD` | `format23` |
| `DD.MM.YY` | `format13` |
| `DD.MM.YYYY` | `format14` |
| `MM.DD.YY` | `format15` |
| `MM.DD.YYYY` | `format16` |
| `DD.MON.YY` | `format19` |
| `DD.MON.YYYY` | `format22` |
| `DD/MM/YY` | `format5` |
| `DD/MM/YYYY` | `format6` |
| `MM/DD/YY` | `format7` |
| `MM/DD/YYYY` | `format8` |
| `DD/MON/YY` | `format18` |
| `DD/MON/YYYY` | `format21` |
| `MON YYYY` | `format27` |
| `q Q YYYY` | `format29` |
| `ww WK YYYY` | `format28` |
| **time_format** (`timeFormat`) | |
| `HHMMSS` | `format1` |
| `HHMM` | `format2` |
| `MMSS` | `format3` |
| `HH:MM:SS` | `format4` |
| `HH:MM` | `format5` |
| `MM:SS` | `format6` |
| `(H)H:(M)M:(S)S` | `format7` |
| `(H)H:(M)M` | `format8` |
| `(M)M:(S)S` | `format9` |
| `HH.MM.SS` | `format10` |
| `HH.MM` | `format11` |
| `MM.SS` | `format12` |
| `(H)H.(M)M.(S)S` | `format13` |
| `(H)H.(M)M` | `format14` |
| `(M)M.(S)S` | `format15` |

Defaults read back through the API: `date_2digit_baseline`=1930, `date_baseline`=1900, `date_format`=YYYY-MM-DD, `decimal_places`=3, `decimal_symbol`=PERIOD, `time_format`=HH:MM:SS. In the file a new stream has `dateFormat` = `format23` (= `YYYY-MM-DD`), `timeFormat` = `format4` (= `HH:MM:SS`), `dateBaseline` 1900, `date2DigitBaseline` 1930, `realDecimalPlaces` 3, `realDecimalSymbol` `period`.

`DD-MM-YY` etc. ids are NOT in the order of the guide list: dates 1-4 = DDMMYY, MMDDYY, YYMMDD, YYYYMMDD; 5-8 = DD/MM/YY, DD/MM/YYYY, MM/DD/YY, MM/DD/YYYY; 9-12 = DD-MM-YY, DD-MM-YYYY, MM-DD-YY, MM-DD-YYYY; 13-16 = DD.MM.YY, DD.MM.YYYY, MM.DD.YY, MM.DD.YYYY; 17-19 = DD-MON-YY, DD/MON/YY, DD.MON.YY; 20-22 = DD-MON-YYYY, DD/MON/YYYY, DD.MON.YYYY; 23 = YYYY-MM-DD; 24-26 = YYYYDDD, DAY, MONTH; 27 = MON YYYY; 28 = ww WK YYYY; 29 = q Q YYYY. Time ids 1-15 follow the guide order exactly (HHMMSS=1 ... (M)M.(S)S=15).

## Other stream properties

| scripting property | element | encoding (set -> file) | default (file) |
|---|---|---|---|
| `date_baseline` | `dateBaseline` | 1950 -> 1950<br>0 -> 1<br>2100 -> 2100 | 1900 |
| `date_2digit_baseline` | `date2DigitBaseline` | 1950 -> 1950<br>1900 -> 1900<br>2099 -> 2099 | 1930 |
| `time_rollover` | `timeRollover` | True -> true | false |
| `import_datetime_as_string` | `importDatetimeAsString` | True -> true | false |
| `decimal_places` | `realDecimalPlaces` | 7 -> 7<br>30 -> 30<br>100 -> 100<br>-1 -> 0 | 3 |
| `decimal_symbol` | `realDecimalSymbol` | Comma -> comma<br>Period -> period<br>Default -> (element removed) | period |
| `angles_in_radians` | (none) | True -> (no change) |  |
| `use_max_set_size` | (none) | True -> (no change) |  |
| `max_set_size` | `typeMaximumSetSize` | 777 -> 777<br>0 -> 0 | 250 |
| `ruleset_evaluation` | `rulesetMode` | FirstHit -> first<br>Voting -> voting | voting |
| `refresh_source_nodes` | `refreshImportSources` | True -> true | false |
| `annotation` | `annotation` | stream note -> stream note |  |
| `script` | `script` | print 1 -> print 1 |  |
| `encoding` | `fileEncoding` | SystemDefault -> (no change)<br>UTF-8 -> UTF-8 | serverDefault |
| `execute_method` | `scriptDefaultExecution` | Script -> true<br>Normal -> false | false |
| `stream_rewriting` | `EnableStreamRewriting` | False -> false<br>True -> true | true |
| `stream_rewriting_maximise_sql` | `OptimizeSQL` | False -> false<br>True -> true | true |
| `stream_rewriting_optimise_clem_execution` | `OptimizeBackendExec` | False -> false<br>True -> true | true |
| `stream_rewriting_optimise_syntax_execution` | `OptimizeSyntaxExec` | False -> false<br>True -> true | true |
| `enable_parallelism` | `EnableParallelism` | False -> false<br>True -> true | true |
| `sql_generation` | `GenerateSQL` | False -> false<br>True -> true | true |
| `database_caching` | `DatabaseCaching` | False -> false<br>True -> true | true |
| `sql_logging` | `LogSQL` | False -> (no change)<br>True -> true | false |
| `sql_generation_logging` | `LogSQLGeneration` | False -> (no change)<br>True -> true | false |
| `sql_log_native` | `LogNativeSQL` | False -> (no change)<br>True -> true | false |
| `sql_log_prettyprint` | `LogPrettyPrintSQL` | False -> (no change)<br>True -> true | false |
| `create_model_applier_for_new_models` | `createApplyNodeForNewModels` | False -> false<br>True -> true | true |
| `create_source_node_from_builders` | `createSourceNodeFromBuilders` | False -> false<br>True -> true | true |
| `has_coordinate_system` | `useCoordinateSystem` | False -> false<br>True -> true | true |
| `record_count_feedback_interval` | `RecordThroughputFreq` | 333 -> 333 | 1000 |
| `create_model_applier_update_links` | `createApplyNodeUpdateLinks` | createDisabled -> createDisabled<br>doNotCreate -> doNotCreate<br>createEnabled -> createEnabled | createEnabled |
| `create_source_node_update_links` | `createSourceNodeUpdateLinks` | createDisabled -> createDisabled<br>doNotCreate -> doNotCreate<br>createEnabled -> createEnabled | createEnabled |
| `coordinate_system` | `coordinateSystem` | ETRS_1989_EPSG_Arctic_zone_5-47 -> ETRS_1989_EPSG_Arctic_zone_5-47 | WGS_1984_World_Mercator |
| `deployment_area` | `scnDeploymentType` | Scoring -> scoring<br>ModelRefresh -> modelRefresh<br>None -> none | none |
| `scoring_terminal_node_id` | `scnScoringTerminalNode` | id123 -> id123 |  |
| `scoring_node_id` | `scnScoringModelNode` | id456 -> id456 |  |
| `model_build_node_id` | `scnModelBuildNode` | id789 -> id789 |  |

No effect on the file (equal to default or not persisted): `angles_in_radians`, `use_max_set_size`

Clamping seen: `date_baseline` 0 -> 1; `decimal_places` -1 -> 0 (100 accepted); `decimal_symbol Default` removes `realDecimalSymbol`.

Surprises: the saved element names differ from the scripting names (`decimal_places` -> `realDecimalPlaces`, `max_set_size` -> `typeMaximumSetSize`, `stream_rewriting_maximise_sql` -> `OptimizeSQL`, `sql_generation` -> `GenerateSQL`, `has_coordinate_system` -> `useCoordinateSystem`, `deployment_area` -> `scnDeploymentType`).
