# Results and verification

What was measured, against what, and how to reproduce it.

## Key results

| Question | Finding | Where |
|---|---|---|
| Does it read Modeler's streams? | Every `.str` in the corpus decodes; unknown nodes and settings are preserved and shown dashed. | `crates/strfmt` |
| Do results match Modeler? | A regression suite of **1,601 streams** is compared with Modeler's own batch output: **1,469 match exactly, 91 match to statistical parity, 41 are documented known gaps and 0 regress** (Windows, 2026-11-23, version 0.6.0). | `xtask/corpus.tsv`, `xtask/known-gaps.tsv` |
| Which models are exact? | C5.0, CHAID, C&R Tree and QUEST (without an overfit-prevention set), Tree-AS, Linear, Linear-AS, Regression, Logistic, Decision List, K-Means, TwoStep, Apriori, CARMA, PCA/Factor, Anomaly, Feature Selection, RBF. | `docs/statistical-parity.md` |
| Which are statistical? | MLP neural network, Kohonen, LSVM, Random Trees, Time Series, Sequence, Auto Cluster, Auto Classifier, Auto Numeric, and C&R Tree / QUEST with an overfit-prevention set: our result is compared with the spread of repeated runs of our own seedable implementation. | `docs/statistical-parity.md` |
| Same answers on Windows and macOS? | Yes. On 2026-11-23 the full regression on an Apple-silicon Mac classified all 1,601 streams exactly as on Windows (1,469 match, 91 statistical, 41 known gaps, 0 regressions), and the stream-by-stream comparison with the Windows baseline shows no difference. On 2026-11-09 the earlier 1,596-stream comparison on an Apple M1 showed no floating-point differences beyond the comparison tolerance (printed precision, or relative 1e-9). | `docs/mac/windows-baseline.tsv`, `docs/mac/compare_baseline.py` |
| Time-series Expert Modeler? | 92 of 93 recorded ARIMA Expert selections and 26 of 26 univariate selections are reproduced; estimation follows the Guide's Marquardt procedure and matches Modeler to 11–14 digits for single-term ARMA and airline models. For ARMA models with both an AR and an MA term Modeler's Marquardt iteration stops earlier than ours: the estimates agree to 2–4 digits, and in all 12 recorded probes our estimates have the lower (better) exact likelihood objective. | [Known gaps](known-gaps.md#known-gaps-and-their-impact) |
| Does Bayes Net work, if Modeler cannot run it? | Yes, from the public algorithm (TAN and Markov blanket). Two demo streams embed Bayes Net models Modeler trained earlier, so the learner is checked against them: it reproduces their structure and every probability table (to 1e-12) for TAN on two data sets and Markov blanket on one. That check found four places where the Guide's text alone gave a different network; they are fixed and listed in the module. Scores written by the nugget have no Modeler run to compare with. | `crates/algo/src/bayes_net.rs`, `crates/algo/tests/bayes_net_oracle.rs` |
| Do Cox and Generalized Linear work, if Modeler cannot run them? | Yes, from the public algorithms. Demo streams embed models Modeler trained, so the trainers are checked against them: Cox reproduces 12 coefficients (to 4e-15) and the 64-point baseline hazard (to 4e-11), and its stepwise selection picks exactly the saved terms; Generalized Linear reproduces the coefficients and covariance of three saved models (binomial with a complementary log-log link, and gamma with an inverse link and case weights; to 1e-15) and the goodness-of-fit and Type III tables Modeler printed. Scores written by the nuggets have no Modeler run to compare with. Cox strata and custom terms, Tweedie and multinomial targets are not implemented. | `crates/algo/src/cox.rs`, `crates/algo/src/genlin.rs`, `crates/exec/tests/cox.rs`, `crates/exec/tests/genlin.rs` |
| Do SVM and KNN work, if Modeler cannot run them? | Yes, from the public algorithms. SVM reproduces the two models saved in `svm_cancer`: the same support vectors for an RBF and a polynomial kernel, the polynomial model's coefficients, constant and Platt parameters to 1e-14 (the RBF model's coefficients agree to the solver tolerance), and every training record's class. KNN reproduces the neighbours and distances of every case in the model saved in `car_sales_knn` (462 distances to 4e-15). Scores the nuggets write have no Modeler run to compare with; KNN does not score a target or save neighbour details yet. | `crates/algo/src/svm.rs`, `crates/algo/src/knn.rs`, `crates/exec/tests/svm.rs`, `crates/algo/tests/knn_oracle.rs` |
| Do GLMM and Temporal Causal Models work, if Modeler cannot run them? | Yes, from the public algorithms, checked against models saved in `testscores_glmm` and `tcm_kpi`. The GLMM trainer reproduces the saved normal-identity model with two nested random intercepts: fixed effects, standard errors, variance components, F tests and information criteria to 1e-9 or better (including the standard errors of the variance components). The TCM trainer reproduces all 30 saved series models: the same predictors in the same order, and every intercept, lag coefficient, standard error and F test to 1e-12. The nuggets write the fixed-part prediction (GLMM) and one-step-ahead fitted values (TCM), which have no Modeler run to compare with. Other GLMM distributions, random slopes, TCM forecasts and outlier analysis are not implemented. | `crates/algo/src/glmm.rs`, `crates/algo/src/tcm.rs`, `crates/exec/tests/glmm.rs`, `crates/exec/tests/tcm.rs` |
| Do Discriminant and SLRM work, if Modeler cannot run them? | Yes, from the public algorithms. Discriminant reproduces every table of the output Modeler saved with `telco_custcat_discriminant` (the stepwise path, canonical functions, structure matrix, centroids and classification table) and the nugget's 24 stored numbers to 1e-13; predictor importance is a Monte-Carlo estimate in the Guide and agrees to 3e-3. SLRM reproduces every count (616 pair counts, four products) and 590 cut points of the model saved in `pm_selflearn` from the three training files. Continuing the training of a saved SLRM model, a randomisation above 0 and separate-groups discriminant scoring are not implemented. | `crates/algo/src/discriminant.rs`, `crates/algo/src/slrm.rs`, `crates/exec/tests/discriminant.rs`, `crates/exec/tests/slrm.rs` |
| How big is it? | Windows x64 (`cargo xtask bundle-windows`, measured 2026-11-23): the download `Data-Mining-0.6.0-windows-x64.zip` is **18.1 MB**; unpacked, `dms.exe` is **32.8 MB** (31.2 MiB) and `dms-cli.exe` **16.0 MB**, each one file that links the C runtime in and imports only Windows system libraries (`odbc32`, `opengl32`, `dwrite`, ...), so no Visual C++ redistributable is needed. For comparison, the Modeler 19 installation folder on the machine where the oracles were recorded is **5.1 GB** in 35,520 files (including its own Java runtime, Spark and Python), about 160 times the size of `dms.exe`. A freshly started `dms.exe` on a generated 24-node stream held about 300 MB of working memory in three runs (301, 303 and 302 MB after 8 seconds; not broken down further). macOS (Apple silicon, release 0.6.0, measured 2026-11-23): the disk image is **19.43 MB**, `Data Mining.app` **44.73 MB** on disk, of which `dms` is 29.47 MB, `dms-cli` 13.98 MB and the two bundled ODBC libraries 0.48 MB; a freshly started app on the same generated stream had 127.8, 127.8 and 127.1 MB resident after 8 seconds (`ps` RSS; not the same measure as the Windows working set). | `target/release/` |
| Is the editor fast enough? | On an Apple M1 (release build) a canvas with 200 labelled nodes pans at 60 fps with vertical sync and 176 fps without; 1,000 nodes 177 fps; first frame about 155 ms after launch. `cargo xtask perf` enforces a p95 frame budget of 4 ms. | `xtask/src/perf.rs` |
| What is not covered? | 41 documented known gaps and a handful of missing nodes and features, explained below. | [Known gaps](known-gaps.md#known-gaps-and-their-impact) |

Node coverage, generated from the node catalog by `cargo xtask parity` (`docs/parity-checklist.md`):

| Scope | Implemented | Total | Notes |
|---|---|---|---|
| MVP | 85 | 85 | The standard single-machine palettes |
| Later | 24 | 24 | Python palette, extension nodes, Cox, GLMM, Temporal Causal Model and Streaming TS done |
| Licence-blocked | 6 | 6 | Standard nodes the oracle's Modeler licence greys out (SVM, KNN, Bayes Net, Discriminant, GenLin, SLRM), so they cannot be verified. All six are implemented from the public algorithms and checked against models Modeler saved in demo streams (see the questions above) |
| Out of scope | 0 | 34 | IBM ecosystem, big data, in-database mining, Statistics integration |

## Data

The raw data are not in the repository. `xtask/corpus.tsv` pins every corpus file by label, relative path and SHA-256
(**1,722 files in 83 labels**); the files themselves live in the git-ignored `corpus/` and
`oracle/` folders.

| Data | What it is | Handling | Used by |
|---|---|---|---|
| IBM sample streams and demo data | Streams and data shipped with Modeler 19 | Never committed; found through `DMS_MODELER_HOME` (`Demos/`) | `ibm-demos` label |
| Team streams | Seven analysis streams and their data | Never committed | `team` label |
| Probe streams | Streams and data our probe scripts generate from fixed seeds (`xtask/oracle/probes/`) | Recorded under `corpus/`, pinned by SHA-256 | all other labels |
| Oracle | Modeler's exported models (PMML) and each terminal node's input (CSV), recorded in batch mode with Modeler 19.0.0.0.26 on Windows | Under `oracle/<label>/<stream>/`; only `cargo xtask oracle` writes it | `regress`, `compare` |

The oracle is about 250 MB, of which `oracle/probes/` is 80 MB. **To run the regression on another machine** copy
`corpus/` and the rest of `oracle/` (175 MB) and the
Modeler `Demos` folder, and set `DMS_MODELER_HOME` to the folder that contains it. `cargo xtask test-corpus` also needs
`oracle/probes/clem_semantics` and `oracle/probes/sav_to_csv` (4.5 MB).

Modeler is used only to produce expected answers; it is never linked, bundled or distributed. Recording oracles is
Windows-only and needs Modeler 19 and `DMS_MODELER_HOME`: `cargo xtask oracle <label> <file>.str` runs one corpus stream
in batch mode and writes each terminal node's input and each trained model, and `cargo xtask probe <name>` runs a
probe script (`xtask/oracle/probes/<name>.py`) to observe a file format or a node's behaviour.

## Reproducing the verification

Run these from the repository root. The regression and corpus checks need `corpus/` and `oracle/` in place (see
[Data](#data)); the others need nothing.

| Check | Command | Needs | Measured time |
|---|---|---|---|
| All build gates | `cargo xtask ci` | nothing | about 2.5 min on an M1 (warm build) |
| Corpus integrity | `cargo xtask corpus verify` | `corpus/` | seconds |
| Corpus-backed tests | `cargo xtask test-corpus` | `corpus/`, `oracle/probes/` | about 2 min (M1) |
| Full regression | the command below | `corpus/`, `oracle/` (without `probes/`) | about 7 min (M1, `--jobs 4`) |
| One stream against its oracle | `cargo xtask compare <label> <file>.str` (file name only) | `corpus/`, `oracle/` | seconds |
| Compare two platforms | `python3 docs/mac/compare_baseline.py docs/mac/windows-baseline.tsv <mac.tsv>` | two regress `--out` files | seconds |
| Canvas frame budget | `cargo xtask perf` | nothing | seconds |

```sh
labels=$(awk -F'\t' 'NR>1 && $1!~/^#/ {print $1}' xtask/corpus.tsv | sort -u \
  | grep -vx -e database -e probes | sed 's/^/--label /' | tr '\n' ' ')
cargo xtask regress $labels --jobs 4 --out target/regress.tsv
```

Do not run `regress` without labels, or with the `database` label, on a machine with the Access ODBC driver: it opens
a modal dialog and hangs. Do not edit source files while a regression runs; later streams fail to build and show up as
false regressions.

## Verification

- **Oracle comparison.** For every stream with an oracle, each terminal node's input is compared with Modeler's export
  cell by cell: numbers within printed precision or relative 1e-9, text and missing values exactly. A stream is
  `match`, `statistical` (held to the statistical-parity method), `known-gap` (listed with a reason in
  `xtask/known-gaps.tsv`) or `regression`. The command fails on a regression and on a known-gap entry that now matches,
  so fixed gaps are deleted instead of lingering.
- **A change is adopted only if more oracle streams match and none regress.** Hypotheses that fit some cases and break
  others are measured, reported and not shipped.
- **Statistical parity** is tested by training with at least 10 seeds of our own generator: Modeler's value must lie
  inside our seeds' range widened by 10 % of its width, and must also rank near the middle of our distribution across
  the oracle family, so a wide band cannot hide a systematic bias. Documented outliers are asserted to stay outliers
  (`docs/statistical-parity.md`).
- **Platform parity.** The same regression runs on Windows and on Apple silicon and is compared stream by stream
  (`docs/mac/`).
- **Interface tests.** Off-screen screenshot tests (egui_kittest on wgpu) render synthetic streams only, and
  interaction tests send real pointer and keyboard events (connecting, menus, editing, undo, settings, translations).
  Reference images are updated only after review.
- **Never crash.** Non-test code may not use `unwrap`, `expect`, `panic!`, `unreachable!`, `todo!` or
  `unimplemented!` (workspace lints), and `unsafe` is forbidden outside `dms-macos-open`. Numbers from files and
  arguments are treated as hostile; every crash fix comes with a test that panicked before the fix, and parsers have
  mutation fuzz tests.
- **Agent-assisted work is checked, not trusted.** Parts of the code were written with AI coding agents. Each result is
  merged only after the coordinator reruns `cargo xtask ci` and the regression itself.

## Limitations

- **One reference version.** The expected answers come from Modeler 19.0.0.0.26, recorded on one Windows machine and
  its licence tier. Behaviour of other versions, and of nodes that licence blocks, is not checked.
- **Comparison at terminal nodes.** Results are compared where a stream ends (each terminal node's input) and for saved
  models, not at every intermediate node.
- **Windows** builds, passes `cargo xtask ci` and the full regression, but the desktop app has not yet had a full manual
  pass there.
- The gaps, missing nodes and their effects are listed under [Known gaps and their impact](known-gaps.md#known-gaps-and-their-impact).
