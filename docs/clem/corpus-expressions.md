# CLEM in our corpus

Generated from `dms-cli xml` output of every `.str` under `corpus/` (labels `team`, `ibm-demos`, `probes`): 72 streams (7 team, 41 ibm-demos, 24 probes). The probe streams contain no CLEM at all. Only the 7 team streams reflect real-world use; the IBM demos show what Modeler ships.

## What was scanned

Only settings whose content is CLEM were taken. For Derive nodes only the settings that belong to the active `deriveType` were read, because the XML keeps stale values for inactive modes. Nodes are de-duplicated by `uid` per stream; supernode contents are included.

| Node | Setting element(s) |
|---|---|
| DeriveNode (formula) | `deriveFormulaExpression` |
| DeriveNode (flag) | `deriveFlagFieldCondition` |
| DeriveNode (conditional) | `deriveConditionalIfCondition`, `deriveConditionalThenExpression`, `deriveConditionalElseExpression` |
| DeriveNode (count) | `deriveCountIncrementCondition`, `deriveCountResetCondition`, `deriveCountIncrementByExpression` |
| DeriveNode (state) | `deriveStateOnCondition`, `deriveStateOffCondition` |
| DeriveNode (set) | `deriveSetTable/rows/row/condition` (the row `name` is the output value, not scanned) |
| SelectNode | `selectCondition` |
| FillerNode | `fillerCondition`, `fillerReplaceExpression` |
| BalanceNode | `balanceDirectives/entries/entry/condition` |
| MergeNode | `mergeRankConditions/rows/row/{mergeCondition,rankExpression}` (spatial merge) |
| AnalysisNode | `analysisUserIf/Then/Else`, only when `analysisUserDefined` is true (never in the corpus) |

Not scanned because they are not CLEM: `condition` elements inside model rule trees (structured data), `aggregateExpressionsTable`, Reclassify old/new value tables (literal mappings), Extension (R/Python) node scripts, stream scripts. Field references such as `graphXField` = `@INDEX` in a Time Plot are field names, not expression settings.

Unknown: whether Derive set-row names and Reclassify values are evaluated as CLEM by Modeler (they are plain literals in this corpus).

Expression instances found (including repeats): team 34, ibm-demos 71, probes 0.

## Distinct expressions

Streams column lists the stream file name (without `.str`); `xN` marks repeats within the same stream. Whitespace is preserved as stored (some expressions contain double spaces); a literal newline is shown as `\n`.

### Team streams

| Node | Setting | Expression | Streams |
|---|---|---|---|
| BalanceNode | `balanceDirectives.condition` | `'heart disease' = 1` | decision_trees_heart.str |
| BalanceNode | `balanceDirectives.condition` | `'heart disease' = 2` | decision_trees_heart.str |
| BalanceNode | `balanceDirectives.condition` | `default = 0.0` | logistic_regression.str |
| BalanceNode | `balanceDirectives.condition` | `default = 1.0` | logistic_regression.str |
| DeriveNode | `deriveFlagFieldCondition` | `No_accounts /= undef` | stream_2_M.str |
| DeriveNode | `deriveFlagFieldCondition` | `equipmon > 0` | stream_1_M.str |
| DeriveNode | `deriveFlagFieldCondition` | `tollmon > 0` | stream_1_M.str |
| DeriveNode | `deriveFormulaExpression` | `(After - Before) / Before * 100` | linear_regression.str |
| DeriveNode | `deriveFormulaExpression` | `(CARDDEBT+OTHERDEBT)/INCOME*100` | cluster_analysis.str |
| DeriveNode | `deriveFormulaExpression` | `YEARSEMPLOYED/AGE` | cluster_analysis.str |
| DeriveNode | `deriveFormulaExpression` | `sqrt(AGE)` | cluster_analysis.str |
| DeriveNode | `deriveFormulaExpression` | `sqrt(DEBTtoINCOMEratio)` | cluster_analysis.str |
| DeriveNode | `deriveSetTable.condition` | `('income' >= 29.0) and ('income' < 46.0)` | stream_1_M.str |
| DeriveNode | `deriveSetTable.condition` | `('income' >= 46.0) and ('income' < 83.0)` | stream_1_M.str |
| DeriveNode | `deriveSetTable.condition` | `('income' >= 83.0) and ('income' <= 1131.0)` | stream_1_M.str |
| DeriveNode | `deriveSetTable.condition` | `('income' >= 9.0) and ('income' < 29.0)` | stream_1_M.str |
| DeriveNode | `deriveSetTable.condition` | `tenure  <=  12` | stream_1_M.str |
| DeriveNode | `deriveSetTable.condition` | `tenure > 12 and tenure <= 36` | stream_1_M.str |
| DeriveNode | `deriveSetTable.condition` | `tenure>36` | stream_1_M.str |
| FillerNode | `fillerCondition` | `@BLANK(@FIELD)` | stream_1_M.str x2 |
| FillerNode | `fillerReplaceExpression` | `11.51` | stream_1_M.str |
| FillerNode | `fillerReplaceExpression` | `75.326` | stream_1_M.str |
| SelectNode | `selectCondition` | `'account YES/NO' = 1` | stream_2_M.str |
| SelectNode | `selectCondition` | `@PARTITION_FIELD = @TESTING_PARTITION` | decision_trees_heart.str, model_comparison.str |
| SelectNode | `selectCondition` | `@PARTITION_FIELD = @TRAINING_PARTITION` | decision_trees_heart.str, linear_regression.str, model_comparison.str |
| SelectNode | `selectCondition` | `@PARTITION_FIELD = @VALIDATION_PARTITION` | model_comparison.str |
| SelectNode | `selectCondition` | `Partition = "2_Testing"` | logistic_regression.str |
| SelectNode | `selectCondition` | `Partition="1_Training"` | logistic_regression.str |
| SelectNode | `selectCondition` | `Record_Count > 1` | stream_2_M.str x2 |

### IBM demo streams

| Node | Setting | Expression | Streams |
|---|---|---|---|
| DeriveNode | `deriveConditionalElseExpression` | `0` | telco_coxreg.str |
| DeriveNode | `deriveConditionalElseExpression` | `1-'$GP-result'` | ulcer_genlin.str |
| DeriveNode | `deriveConditionalElseExpression` | `1-'$GP-result2'` | ulcer_genlin.str |
| DeriveNode | `deriveConditionalElseExpression` | `@AVE(PowerInc, 5)` | condlearn.str |
| DeriveNode | `deriveConditionalElseExpression` | `@AVE(TempInc, 5)` | condlearn.str |
| DeriveNode | `deriveConditionalIfCondition` | `'$G-result'` | ulcer_genlin.str |
| DeriveNode | `deriveConditionalIfCondition` | `'$G-result2'` | ulcer_genlin.str |
| DeriveNode | `deriveConditionalIfCondition` | `@FIELD>0.248` | telco_coxreg.str |
| DeriveNode | `deriveConditionalIfCondition` | `Time < 5` | condlearn.str x2 |
| DeriveNode | `deriveConditionalThenExpression` | `'$GP-result'` | ulcer_genlin.str |
| DeriveNode | `deriveConditionalThenExpression` | `'$GP-result2'` | ulcer_genlin.str |
| DeriveNode | `deriveConditionalThenExpression` | `0.0` | condlearn.str x2 |
| DeriveNode | `deriveConditionalThenExpression` | `1` | telco_coxreg.str |
| DeriveNode | `deriveCountIncrementByExpression` | `1` | condlearn.str |
| DeriveNode | `deriveCountIncrementCondition` | `Pressure /== 0` | condlearn.str |
| DeriveNode | `deriveCountResetCondition` | `Time == 0` | condlearn.str |
| DeriveNode | `deriveFlagFieldCondition` | `Time > 0 and PowerInc * @OFFSET(PowerInc, 1) < 0` | condlearn.str |
| DeriveNode | `deriveFlagFieldCondition` | `frozenmeal == "T" and beer == "T" and cannedveg == "T"` | baskrule.str |
| DeriveNode | `deriveFlagFieldCondition` | `fruitveg = "T" and fish = "T"` | baskrule.str |
| DeriveNode | `deriveFlagFieldCondition` | `wine = "T" and confectionery = "T"` | baskrule.str |
| DeriveNode | `deriveFormulaExpression` | `"ALERT: More business expected in this area: http://maps.google.com/maps?z=18&q="  \n >< TAXIS_NEEDED_HERE  >< " from time " ><  TAXIS_NEEDED_WHEN` | taxi_availabity_stb.str |
| DeriveNode | `deriveFormulaExpression` | `(100 * ('Upper Estimate' - 'Lower Estimate')) / 'Lower Estimate'` | telco_coxreg.str |
| DeriveNode | `deriveFormulaExpression` | `(After - Before) / Before * 100.0` | cplex_goodslearn.str, goodslearn.str, goodsplot.str |
| DeriveNode | `deriveFormulaExpression` | `@DIFF1(Power, Time)` | condlearn.str |
| DeriveNode | `deriveFormulaExpression` | `@DIFF1(Temperature,Time)` | condlearn.str |
| DeriveNode | `deriveFormulaExpression` | `Na/K` | druglearn.str, drugplot.str |
| DeriveNode | `deriveFormulaExpression` | `TAXI_DENSITY / PEOPLE_DENSITY` | taxi_availabity_stb.str |
| DeriveNode | `deriveFormulaExpression` | `[Longitude,Latitude]` | geospatial_association.str |
| DeriveNode | `deriveFormulaExpression` | `[X,Y]` | stp_server_demo.str x2 |
| DeriveNode | `deriveFormulaExpression` | `stb_centroid_latitude(substring(1,5,STB_GH5_15MINS)) >< ', ' >< stb_centroid_longitude(substring(1,5,STB_GH5_15MINS))` | taxi_availabity_stb.str |
| DeriveNode | `deriveFormulaExpression` | `substring(18,5,STB_GH5_15MINS)` | taxi_availabity_stb.str |
| DeriveNode | `deriveStateOffCondition` | `@SINCE(PowerFlux == 'T') > 5 or Time == 0` | condlearn.str |
| DeriveNode | `deriveStateOnCondition` | `PowerFlux == 'T' and @OFFSET(PowerFlux,1) == 'T'` | condlearn.str |
| FillerNode | `fillerCondition` | `@BLANK(@FIELD)` | broadband_apply_models.str, broadband_create_models.str, condlearn.str x2, cplex_pm_selflearn.str, pm_selflearn.str, telco_churn.str, telco_coxreg.str, telco_dataaudit.str x2 |
| FillerNode | `fillerReplaceExpression` | `'$R-equipmon'` | telco_dataaudit.str |
| FillerNode | `fillerReplaceExpression` | `'$R-loglong'` | telco_dataaudit.str |
| FillerNode | `fillerReplaceExpression` | `1` | telco_coxreg.str |
| FillerNode | `fillerReplaceExpression` | `3.240` | telco_churn.str |
| FillerNode | `fillerReplaceExpression` | `to_date(DATE_)` | broadband_apply_models.str, broadband_create_models.str |
| FillerNode | `fillerReplaceExpression` | `to_number(@FIELD)` | condlearn.str |
| FillerNode | `fillerReplaceExpression` | `to_string(@FIELD)` | condlearn.str |
| FillerNode | `fillerReplaceExpression` | `to_string(campaign)` | cplex_pm_selflearn.str, pm_selflearn.str |
| MergeNode | `mergeRankConditions.mergeCondition` | `within('POINT', 'MULTIPOLYGON')` | geospatial_association.str |
| MergeNode | `mergeRankConditions.rankExpression` | `area('MULTIPOLYGON')` | geospatial_association.str |
| SelectNode | `selectCondition` | `'$STP-Time'="10:00:00"` | stp_server_demo.str |
| SelectNode | `selectCondition` | `Outcome == 101` | condplot.str |
| SelectNode | `selectCondition` | `Outcome == 202` | condplot.str |
| SelectNode | `selectCondition` | `Outcome == 303` | condplot.str |
| SelectNode | `selectCondition` | `TAXI_TO_PEOPLE_RATIO_TILE20 = 1` | taxi_availabity_stb.str |
| SelectNode | `selectCondition` | `Time == 0 or Outcome /== @OFFSET(Outcome, 1)` | condlearn.str |
| SelectNode | `selectCondition` | `campaign = 2` | pm_binaryclassifier.str |
| SelectNode | `selectCondition` | `campaign=2` | pm_decisionlist.str |
| SelectNode | `selectCondition` | `churn = 0` | telco_coxreg.str |
| SelectNode | `selectCondition` | `default = '$null$'` | bayes_bankloan.str |

## Frequency of functions and operators

Tokenised with a small throwaway lexer written for this report (not the future parser). Function names are folded to lower case, `@` names to upper case. A bare word that is not followed by `(` and not a keyword counts as a field. `default` is counted as a keyword but is context-dependent (see observations).

#### Functions (called with parentheses)

Cells are occurrences / distinct streams.

| Item | Team | IBM demos |
|---|---|---|
| `sqrt` | 2 / 1 | - |
| `substring` | - | 3 / 1 |
| `to_string` | - | 3 / 3 |
| `to_date` | - | 2 / 2 |
| `area` | - | 1 / 1 |
| `stb_centroid_latitude` | - | 1 / 1 |
| `stb_centroid_longitude` | - | 1 / 1 |
| `to_number` | - | 1 / 1 |
| `within` | - | 1 / 1 |

#### Special `@` functions and fields

Cells are occurrences / distinct streams.

| Item | Team | IBM demos |
|---|---|---|
| `@PARTITION_FIELD` | 6 / 3 | - |
| `@TRAINING_PARTITION` | 3 / 3 | - |
| `@FIELD` | 2 / 1 | 13 / 8 |
| `@BLANK` | 2 / 1 | 10 / 8 |
| `@TESTING_PARTITION` | 2 / 2 | - |
| `@VALIDATION_PARTITION` | 1 / 1 | - |
| `@OFFSET` | - | 3 / 1 |
| `@AVE` | - | 2 / 1 |
| `@DIFF1` | - | 2 / 1 |
| `@SINCE` | - | 1 / 1 |

#### Operators

Cells are occurrences / distinct streams.

| Item | Team | IBM demos |
|---|---|---|
| `=` | 13 / 5 | 10 / 7 |
| `>` | 6 / 2 | 3 / 2 |
| `>=` | 4 / 1 | - |
| `/` | 3 / 2 | 7 / 7 |
| `<` | 3 / 1 | 3 / 1 |
| `<=` | 3 / 1 | - |
| `*` | 2 / 2 | 5 / 5 |
| `-` | 1 / 1 | 6 / 5 |
| `+` | 1 / 1 | - |
| `/=` | 1 / 1 | - |
| `==` | - | 12 / 3 |
| `><` | - | 5 / 1 |
| `/==` | - | 2 / 1 |

#### Keywords

Cells are occurrences / distinct streams.

| Item | Team | IBM demos |
|---|---|---|
| `and` | 5 / 1 | 6 / 2 |
| `default` | 2 / 1 | 1 / 1 |
| `undef` | 1 / 1 | - |
| `or` | - | 2 / 1 |

#### Operand forms

| Form | Team | IBM demos |
|---|---|---|
| bare word (field) | 21 | 64 |
| single-quoted token | 11 | 20 |
| double-quoted string | 2 | 10 |
| number literal | 25 | 41 |
| backquote char | 0 | 0 |

Single-quoted tokens are ambiguous in CLEM (field name or string), so the evaluator must resolve them against the stream schema.


## Observations for the first implementation

- Team streams need very little: arithmetic (`+ - * /`), comparisons (`= > >= < <= /=`), `and`, `sqrt`, `undef`, `@BLANK(@FIELD)`, and the partition specials `@PARTITION_FIELD` with `@TRAINING_PARTITION`, `@TESTING_PARTITION`, `@VALIDATION_PARTITION` (the partition specials are by far the most common construct in team Select nodes). No `if/then/else` expression, no string or date functions, no sequence functions occur in team streams.
- IBM demos add: `==` and `/==`, string concatenation `><`, `or`, `to_string`, `to_date`, `to_number`, `substring`, `@OFFSET`, `@SINCE`, `@DIFF1`, list literals `[X,Y]` (note: commas inside the list; the manual only shows space-separated lists), spatial `within` and `area`, and the geohash helpers `stb_centroid_latitude` and `stb_centroid_longitude`.
- `@AVE(PowerInc, 5)` is used in `condlearn.str` but is not in the 19.0 function tables (the manual documents `@MEAN`). It is probably an older alias of `@MEAN`; treat as an unverified alias until an oracle run confirms it.
- `default = 0.0` and `default = 1.0` (Balance directives) use the word `default` as a pseudo-field meaning "all other values"; it is not CLEM. In `bayes_bankloan.str` the Select condition `default = '$null$'` uses `default` as an ordinary field name and `'$null$'` as a single-quoted token (the null literal shown by the UI; the manual does not define it as source syntax).
- Single-quoted field names containing spaces or punctuation are common: `'heart disease'`, `'account YES/NO'`, `'Upper Estimate'`, and generated model fields such as `'$G-result'`, `'$R-equipmon'`, `'$STP-Time'`. The same single-quote form is also used for string comparison values (`PowerFlux == 'T'`), so name resolution needs the schema.
- Numeric literals in derived expressions are stored with plain digits (`100`, `100.0`, `0.248`, `3.240`); no scientific notation occurs.
- One expression spans lines and mixes string literals with `><`; a literal newline appears between operands, so the parser must treat newlines as whitespace.
- Whitespace around operators is irregular (`tenure  <=  12`, `tenure>36`, `Partition="1_Training"`); `=` and `==` are interchangeable.
- Recommended first subset: literals, field references (bare and single-quoted), `+ - * / = == /= /== < <= > >=`, `and or not`, `><`, `undef`, `@BLANK`, `@NULL`, `@FIELD`, partition specials, `sqrt`, `to_string`, `to_number`, `to_date`, `substring`, `if/then/elseif/else/endif`. Next: `@OFFSET`, `@SINCE`, `@DIFF1`, `@INDEX`, list literals, then spatial functions.
