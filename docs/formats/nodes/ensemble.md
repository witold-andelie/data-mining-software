# `ensemble` node (`EnsembleNode`)

Scripting properties are Table 83 of the Modeler 19 Python Scripting and Automation Guide
(`ensemblenode`). File element names and the values below were read from
`corpus/models_next2/ens_*.str` (both the train and grid Ensemble nodes). This is not a
one-property differential probe: `xtask/oracle/probes/models_next2_streams.py` writes the
whole node in one save. Do not add rows to `docs/formats/node-settings.tsv` from this note.

Paths are children of the node's `properties` element.

| scripting property | file element | values read from `ens_*.str` |
|---|---|---|
| `ensemble_target_field` | `ensembleTargetField` | `respond` |
| `filter_individual_model_output` | `filterEnsembledFields` | `false` on every `ens_*` stream |
| `flag_ensemble_method` | `ensembleFlagsMethod` | `voting` (`ens_voting`); `confidenceWeightedVoting` (`ens_confidence`); `rawPropensityWeightedVoting` (`ens_raw_vote`); `highestConfidence` (`ens_highest`); `averageRawPropensity` (`ens_avg_raw`); `adjustedPropensityWeightedVoting` (`ens_adjusted_vote`); `averageAdjustedPropensity` (`ens_avg_adjusted`) |
| `flag_voting_tie_selection` | `ensembleFlagTiesMethod` | `highestConfidence` on the three voting methods; `random` on `ens_highest`, `ens_avg_raw`, and the adjusted streams |
| `set_ensemble_method` | `ensembleSetsMethod` | `confidenceWeightedVoting` (nominal default; the `ens_*` targets are flags) |
| `set_voting_tie_selection` | `ensembleSetTiesMethod` | `random` |
| `calculate_standard_error` | `calculateStandardError` | `true` (no standard-error field is written for the flag target) |

The same nodes also store the common elements in `common.md` (`keywords`, `lastSavedAs`,
`enabled`, `fieldMappingEssentialFields`, `repositoryPath`, `repositoryVersionMarker`,
`doReadValues`, `toolTipText`, `annotation`, `lastSavedBy`, `lastSavedDate`, `label`,
`creationDate`, `repositoryLastModified`, `cacheEnabled`, `scriptExecutionModeChange`,
`autoLabel`, `createdBy`, `name`, `position`).

Score fields the node reads and writes are documented on `crates/exec/src/nodes/ensemble.rs`.
Adjusted-propensity names `$CAP-` / `$RAP-` / `$LAP-` / `$EAP-` and the ensemble adjusted
field `$XFAP-` are inferred from the raw-propensity pattern. No `ens_*` oracle contains them.
