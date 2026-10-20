# Decision List training validation — 2026-10-18

`BuildDecisionListNode` now trains through `dms-algo::decision_list` and `decision_list`
is registered as implemented, outside `SCORING_ONLY`. MVP parity is **69/85**.
No Modeler execution, installation, oracle/probe command, shared corpus/oracle writes,
or git state changes were used.

## Implementation and boundaries

The implementation ports the verified clean-room `dl-lab/dl.py` prototype. Sources and
case-specific rules are recorded in the module docs and `crates/algo/README.md`:
Modeler 19 Algorithms Guide pp.105–111, PMML 4.3 RuleSetModel. Rust uses the existing
regularized incomplete beta function, bounded inverse bisection and exact boundary-shape
forms for F quantiles. Python 3.13 / SciPy 1.14.1 was used only to calculate test values.
Five F quantiles use 1e-12 relative tolerance; nine Clopper–Pearson intervals use 1e-12
absolute tolerance, including n=1,000,000.

Always read `minGroupSize` and `maxListDepth`. With `expertMode=false`, use confidence 95%,
rule width/depth and list width 5, equalFreq/10 bins, growth 2. Expert mode reads
`confidenceLevel`, `maxRuleWidth`, `maxRuleDepth`, `maxListWidth`, `binningMethod`,
`binCount`, `segmentGrowth`. Resolve custom target/inputs when automatic field selection
is disabled; the default response is the target flag's true value, overridden by
`modelTargetValue`. Filter training partitions through the existing builder path.

`train_xml` writes rule ids, response score, correct/record counts, equal categorical
predicates, six-decimal numeric bounds, mining fields in first-use order, `RuleSet0`,
`isDisabled`, `searchDirection=up`, and the disabled True `remainder` label.
`train` parses this into the existing scoring IR. Label and search-direction extensions
remain available in XML; the existing IR retains disabled state.

Explicit rejection: down search, reuseFields=false, frequency/weight use, missing
input/target values, events, interactive states, Split fields, unknown/non-scalar storage.
Finite settings and data indices are checked before use. Limits: beam widths 1..64,
depths 1..32, bins 2..256, 256 categories per input, 128 inputs, 1,000,000 rows,
8,000,000 input cells, 32 MiB input text, 10,000 candidates per level and 200,000,000
coverage checks. Exceeding these returns an error.

## Direct training test output

Command: `cargo test -p dms-exec --features corpus --test decision_list -- --nocapture`.
This evaluates the builder itself, independently of saved-nugget fallback, and compares
the full RuleSet IR, including header/schema and ordered predicates/counts.

```text
train_next/dl_default: 6 rules equal (including remainder)
train_next/dl_max_rules2: 3 rules equal (including remainder)
train_next/dl_max_rules4: 5 rules equal (including remainder)
train_next/dl_search: 6 rules equal (including remainder)
train_next/dl_min_group: 4 rules equal (including remainder)
train_next/dl_confidence80: 6 rules equal (including remainder)
train_next/dl_numeric: 5 rules equal (including remainder)
models_next2/dl_default: 4 rules equal (including remainder)
models_next2/dl_min_group: 2 rules equal (including remainder)
models_next2/dl_max_rules: documented GAP (equal-rate list tie: north+web vs web+gold)
test trained_lists_match_modeler_nine_cases_and_document_tie_gap ... ok
test unverified_search_and_weight_settings_are_rejected ... ok
```

Result: **2 passed; 0 failed**. Nine matching models contain 41 rules including remainders.
Four algorithm unit tests also pass: SciPy numerical values, strict group-size acceptance,
bin boundaries/six-decimal PMML, XML escaping/extensions, and invalid settings/indices/missing data.

## Every requested stream comparison

Each line below is the exact summary from `cargo xtask compare <label> <file.str>`.
Every run reported `3 exported, 0 failed (0 root causes)`.

```text
xtask compare: train_next/dl_confidence80.str: 3/3 terminal inputs match Modeler
xtask compare: train_next/dl_default.str: 3/3 terminal inputs match Modeler
xtask compare: train_next/dl_max_rules2.str: 3/3 terminal inputs match Modeler
xtask compare: train_next/dl_max_rules4.str: 3/3 terminal inputs match Modeler
xtask compare: train_next/dl_min_group.str: 3/3 terminal inputs match Modeler
xtask compare: train_next/dl_numeric.str: 3/3 terminal inputs match Modeler
xtask compare: train_next/dl_search.str: 3/3 terminal inputs match Modeler
xtask compare: models_next2/dl_default.str: 3/3 terminal inputs match Modeler
  mismatch       id12L11N87JTTL   applydecisionlist row 1 field $D-respond: Modeler $null$, ours "Y"
  mismatch       id1SV11TBBJDGH   applydecisionlist row 1 field $D-respond: Modeler $null$, ours "Y"
xtask compare: models_next2/dl_max_rules.str: 1/3 terminal inputs match Modeler
xtask compare: models_next2/dl_min_group.str: 3/3 terminal inputs match Modeler
xtask compare: ibm-demos/pm_decisionlist.str: 3/3 terminal inputs match Modeler
```

Total: **31/33**. Known GAP: equal response-rate lists in `models_next2/dl_max_rules`.
The prototype/Rust choose north (80/80) then web (60/60), remainder 55/379.
Modeler chooses web (60/60) then gold (55/55), remainder 80/404.
Both selected lists have response rate 1.0. Stable prototype generation order is preserved;
the oracle's additional tie rule is unknown. The regression test asserts both precise rule
predicates and counts; it does not treat arbitrary inequality as the expected GAP.

## Quality gates

| Command | Final result |
|---|---|
| `cargo test --workspace` | exit 0; 418 passed, 0 failed, 0 ignored (sum of test summaries) |
| `cargo clippy --workspace --all-targets --features dms-exec/corpus -- -D warnings` | exit 0, no warnings |
| `cargo fmt --all --check` | exit 0, no output |
| `cargo xtask ci` | exit 0, final output below |

```text
xtask layers: 13 packages ok
xtask parity: docs/parity-checklist.md up to date
xtask ci: all gates passed
```

Full local logs: `target/decision-list-validation/` (`workspace-tests.log`,
`clippy-corpus.log`, `fmt.log`, `ci.log`, `trained-lists.log`, and one `compare-*.log`
per stream). These are generated process files, not committed artifacts.

## Discriminating experiments (2026-10-20, open)

`auto_train/ac_lift` trains a different list from Modeler's (third rule `region = north` 35/57
instead of `age > 54.5` 36/59). `xtask/oracle/probes/dl_discriminate_streams.py` builds twelve
small nominal-input streams (label `dl_discriminate`), each designed to separate one candidate
search policy: rule-result de-duplication (original, dedup, final, prefix), list identity
(none, syntax, canonical, coverage), completed-list handling (deepest, finished, carry,
pre-pruning detection) and five tie orders. Against Modeler's twelve lists, all
80 × 5 = 400 policy/tie combinations die (also 2,000 with separate rule and list ties):
`dl09_prefix_canonical` kills the last eight survivors of cases 1-8, and `dl04_coverage` with
`dl10_tie_sizes` alone is contradictory. The learner is unchanged; the eight exact learners
stay exact; the unchanged builder trains only `dl01` and `dl03` exactly and refuses three as
unresolved ties. Clues for the missing mechanism: `dl08_completed` keeps a one-rule list
(`b = v1`, 120/280) over deeper lists; `dl09` splits the same 220/280 union into different
rules; `dl11`/`dl12` choose the same ordered rules whatever the physical column order.

Phase 3 (same day) re-derived the search from the Algorithms Guide chapter (the local PDF's
cover says Modeler 18.0) and ran 21,844 configurations over all 21 cases (the twelve above,
the eight exact learners, `ac_lift`): segment generation and growth, conditional populations,
rule and list widths, mixed-length frontiers, list estimates and intervals, and final-result
ordering separate from the beam. None fits all 21. The best fit 20 of 21 by ranking the beam
with the mean of the rules' first-hit rates (not the guide's union estimate), missing either
`train_next/dl_max_rules4` or `dl_discriminate/dl05_carry`. It keeps a completed list
(`ac_lift`'s three-rule `age > 54.5` list, `dl08`'s one-rule list) competing with deeper
lists, and orders final results by names, not column order (`dl11`/`dl12`). No mechanism is
grounded well enough to implement; the learner is unchanged.

Phase 4 (same day) compared the Decision List chapter of the official 19.0 Algorithms Guide
(downloaded from IBM's 19.0 documentation page) page by page with the 18.0 copy: no formula,
pseudocode step or stopping test differs. It then tested 5,832 configurations drawn from the
public Data Surveyor / CWI literature (Holsheimer & Siebes CS-R9406; Kersten & Holsheimer
CS-R9521: response-probability rule beam, n best extensions per level; Siebes 1994:
non-overlapping confidence intervals; CN2's best-complex memory; gain/WRAcc quality;
cover-identity duplicates; bounded lookahead). None fits all 21 cases; the best stays 20 of 21
and still needs the mean-of-rule-rates beam score (19 of 21 without it). The learner is
unchanged.
