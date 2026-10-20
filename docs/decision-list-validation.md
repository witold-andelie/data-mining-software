# Decision List training validation — 2026-10-20

The native learner now adopts phase 3's `estimates-01822`: **20/21 full RuleSet IR
matches**, up from 10/21, with none of the ten previously exact cases lost.
`dl_discriminate/dl05_carry` remains the sole counterexample on the frozen bench.
This is an empirical strict improvement, not a recovery of Modeler's proprietary search.

## Mechanism and public sources

The public IBM SPSS Modeler **19.0 Algorithms Guide**, Decision List chapter
(PDF pp.109–115), supplies binning, local sea-level splitting and Clopper–Pearson
confidence tests. The corresponding 18.0 chapter is printed pp.105–111
(PDF pp.125–131); phase 4 found no algorithmic difference between these chapters.
The 19.0 PDF is available from
[IBM](https://www.ibm.com/docs/en/SS3RA7_19.0.0/pdf/AlgorithmsGuide.pdf).
PMML representation follows DMG PMML 4.3 RuleSetModel. No IBM implementation is used.

1. At each rule depth, stably retain the highest response-rate extensions up to
   `maxRuleWidth`. Return distinct terminal rules and the final depth's selected
   rules, rather than returning selected rules from every intermediate level.
2. Extend each list using only its remaining records. De-duplicate lists by ordered
   rules, sorting conditions within each rule for identity only; retain the first
   generated representation. Coverage equality is not list identity.
3. Rank the list beam by the arithmetic **mean of its rules' first-hit response
   rates**, preserving generation order on ties. Carry genuinely completed lists
   into subsequent candidate levels, still subject to beam pruning.
4. Select the final result from the entire last candidate level, before width
   pruning, by union response rate, fewer covered records, fewer rules, descending
   first-hit rate vector, then name-based ordered predicate syntax (including
   `and`/`or` structure). Do not substitute field indices or XML tag order.
5. Before emission, intersect repeated restrictions on a field; retain the seed
   field and order subsequent fields by name. This empirical serialization
   convention removes redundant bounds and supplies exact predicates/mining schema
   without changing the search's identities or traversal.

The **mean beam score is an empirical fit and departs from the guide's union
estimate**. Phase 3's 21,844 configurations and phase 4's 5,832 configurations
found no 21-case fit. Their selected family matched 20/21 semantically and 14/21
in predicate representation; production adds the uniform emission convention
above and verifies full RuleSet equality. Local evidence: `target/dl4/REPORT.md`,
`target/dl5/REPORT.md`, and this adoption's `target/dl6/REPORT.md`.

## Exact training comparisons

`crates/exec/tests/decision_list.rs` evaluates the upstream data, clears saved
builder results, and calls `registry::factory("decision_list")` directly.
Saved-nugget fallback cannot make a training assertion pass. Equality includes
the model header, mining schema, selection method, rule ids, ordered predicate
trees, scores/confidences/weights, counts and disabled state.

| Cases | Exact models | Rules including remainder |
|---|---:|---:|
| `dl_discriminate` 01–04 and 06–12 | 11 | 36 |
| `train_next`: default, max_rules2, max_rules4, search, min_group, confidence80, numeric | 7 | 35 |
| `models_next2/dl_min_group` | 1 | 2 |
| `auto_train/ac_lift`, saved Decision List component settings | 1 | 4 |
| **Frozen bench exact total** | **20** | **77** |

The `ac_lift` comparison extracts the saved Decision List build node and embedded
RuleSet PMML, trains on its actual 360 upstream records, and compares the full IR.
It does not assume Auto Classifier's component settings.

Outside this bench, `models_next2/dl_max_rules` now also matches full IR. The
completed-list support tie in `models_next2/dl_default` remains explicitly refused:
equal union rate, coverage, length and first-hit rate vector but different
per-rule supports. The lexical key is not established for that completed-list
ambiguity. A corpus test asserts both the remaining refusal and the newly
resolved capped-list case. The old blanket rate/coverage tie refusal is removed.

## Remaining counterexample and limits

For `dl05_carry`, the oracle's ordered rules claim 116/160, 80/100 and 78/100
responses/records. The learner claims 118/140, 120/180 and 36/40. Both leave the
disabled remainder at 10/300; equality of their final union does not imply
equality of the trained lists. The exact predicates and counts of this gap are
fixed by a separate corpus test, rather than accepting arbitrary inequality.

Still open: that partition, a source-grounded rationale for the empirical beam
score and final ties, generalization beyond these oracles, and the completed
support tie above. Down search, reuseFields=false, frequency/weight use, missing
inputs/targets, events, interactive state, Split fields and unknown/non-scalar
storage remain explicitly unsupported.

Settings mapping is unchanged: always read `minGroupSize` and `maxListDepth`;
non-expert mode uses confidence 95%, rule width/depth and list width 5,
equalFreq/10 bins and growth 2. Expert mode reads the saved confidence, rule/list
width/depth, binning and growth. Custom target/input selection, response value
and training partition filtering follow the existing builder path.

Limits remain widths 1..64, depths 1..32, bins 2..256, 256 categories/input,
128 inputs, 1,000,000 rows, 8,000,000 input cells, 32 MiB input text,
10,000 candidates/level and 200,000,000 coverage checks. No Python runtime is
needed. Incomplete-beta/F quantiles retain five SciPy numerical references and
nine Clopper–Pearson references, including million-record counts.

## Regression scope and evidence

The required regression labels are `dl_discriminate`, `train_next`,
`models_next2`, `auto_train`, `models_next`. No unfiltered or database regression
is run. Workspace corpus tests skip only `live::database_oracles`.
Final commands, results, per-case before/after build timings and read-only input
integrity checks are recorded in `target/dl6/REPORT.md` and its adjacent logs.

Five resolved `dl_discriminate` known-gap entries are removed; `dl05` remains.
All nine listed `auto_train/ac_*` streams still differ because of the C5.0
component, so their entries remain with corrected reasons. The Decision List
component no longer explains those ensemble differences.
