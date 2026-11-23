# Known gaps and their impact

Every place where our result differs from Modeler's, or where a Modeler feature is missing.

## Known gaps and their impact

A **gap** is a place where our result differs from Modeler's, or where a Modeler feature is missing. None is hidden:
`cargo xtask regress` lists every known gap with its reason in `xtask/known-gaps.tsv`, fails on a new difference, and
fails on a listed gap that has since been fixed, so the list cannot go stale.

**What it means in practice.** None of the gaps stops the core workflow (open, edit, run and save a stream): 1,560 of
1,601 recorded streams agree with Modeler exactly (1,469) or statistically (91). What you may notice:

1. A stream that uses a node we do not have runs up to that node and reports it; the rest of the stream is unaffected.
2. Models that depend on random numbers or on Modeler's optimizer stopping rule give **equivalent but not identical**
   numbers: do not expect the same weights, cluster layout or last digits.
3. Time-series models with both AR and MA terms give estimates that agree with Modeler to about 2–4 significant digits
   (ours have the better likelihood); a few badly conditioned fits differ more (see
   [Statistical parity](#2-statistical-parity-91-streams-not-failures)).
4. Nodes that sample without a seed (Balance) differ from run to run, in Modeler as well.

### 1. Documented known gaps: 41 of 1,601 streams

| Group | Streams | What differs | Impact |
|---|---|---|---|
| Nodes not implemented | 1 | CPLEX Optimization (`cplex_goodslearn`), which is out of scope | The node does not run. Its branch of a stream stops there; palette entries are greyed out with the reason |
| Streaming Time Series demo stream | 1 | `catalog_streaming_timeseries`: the node runs and picks the same model family as Modeler (Winters additive smoothing), but our smoothing parameters differ (our fit has the smaller squared error), so forecasts differ by up to 4%; the saved nugget in the same stream holds an older ARIMA model that Modeler's own run replaced by the retrained one | Values of that stream differ; Modeler's estimate is the less accurate one |
| Discriminant demo stream | 1 | `telco_custcat_discriminant`: Modeler 19 cannot run the Discriminant nugget (licence), so the oracle's nuggets add no fields, while ours scores them from the model saved in the stream | The scores have no Modeler output to compare with; the build node reproduces the saved model (see above) |
| SVM demo stream | 1 | `svm_cancer`: Modeler 19 cannot run the SVM nugget (licence), so the oracle's nuggets add no fields, while ours scores them from the models saved in the stream | The scores have no Modeler output to compare with; the build nodes are checked against the two saved models (see above) |
| Cox demo stream | 1 | `telco_coxreg`: the oracle's Modeler install has no Cox node, so its nuggets add no fields, while ours scores them from the model saved in the stream | The scores have no Modeler output to compare with; the build node is checked against the saved model (see above) |
| Bayes Net demo streams | 2 | `bayes_bankloan` and `bayes_churn_retrain`: Modeler 19 cannot run the Bayes Net nugget (licence), so the oracle's downstream filter carries no Bayes scores, while ours scores them from the models saved in the streams | The scores have no Modeler output to compare with; the learner itself is checked against the saved models (see above) |
| Demo streams with unsupported settings or an incomplete oracle | 3 | `modelingintro` (a Statistics File option, and Modeler's batch run finds nothing to execute), `stp_server_demo` (STP node; Modeler's own run fails), `pm_binaryclassifier` (the oracle retrains a replacement-linked Auto model, which is out of scope) | Affects those demo streams only |
| Time Series scoring of one demo | 1 | `broadband_create_models`: all four transfer-function targets match; for two series the first 63 rows differ because Modeler scores `Market_3` with its auxiliary Holt model instead of the saved primary ARIMA, and `Total`'s oracle model is not saved | Values in rows 1–63 of those two series differ; the other targets are exact |
| Unseeded Balance | 4 | The Balance node has no seed, so Modeler's own runs keep different records (row counts 1155 vs 1156, 356 vs 379, 206 vs 210, 7774 vs 7797) | No single Modeler run can be matched. Models trained downstream (for example the logistic regression) vary with the sample, as they do in Modeler |
| Excel test-fixture artifacts | 15 | Our generated test sheets contain cell-less `<row>` elements. Excel never writes them, and Modeler reads such sheets inconsistently, so we refuse them | None for files saved by Excel; 65 of 71 `sources` streams and 12 of 21 `excel_more` streams match |
| GLE: separated data and chaotic paths | 9 | Binomial identity/log fits where the data are separated, or where fitted means leave (0, 1) so the iteration is chaotic or Modeler's estimate is not a root of the score. `gle_binom_factor` is only a rounding difference of about 2e-7 relative | Affects ill-posed fits only: 40 of 49 GLE streams match exactly |
| Decision List | 1 | `dl05_carry`: a tie in the rule search picks a different first rule (equal total coverage, different partition) | One edge case; 11 of 12 Decision List streams match |
| Linear-AS | 1 | `x_shift`: forward selection treats one numerically degenerate term differently | Only when a candidate term is numerically degenerate; 20 of 21 Linear-AS streams match |

### 2. Statistical parity: 91 streams (not failures)

For these models Modeler's training cannot be reproduced bit for bit (an undocumented random number generator or
optimizer stopping rule), so the stream is accepted when Modeler's value lies inside the spread of our own seeded runs
(method in `docs/statistical-parity.md`). By model family: Time Series 33, Kohonen 17, MLP neural network 13, Sequence 12,
Random Trees 9, LSVM 3, QUEST 3, C&R Tree 1.

**Impact:** the models are equivalent in distribution but not numerically identical. Do not use them where a result must
match a Modeler run digit for digit. For the 33 Time Series streams, the first mismatching cell each one reports differs
from Modeler's by a median of 3e-6 (relative) and by less than 1e-4 in 29 of them. The other four are synthetic
no-constant ARMA fits on a series with a large mean, where the AR root is close to 1: the cell is a confidence limit and
the difference is 3e-4 to 0.38 (see below).

### 3. Open algorithm differences

- **Time-series estimation.** Modeler's optimizer follows the Guide's Marquardt procedure and we reproduce it to 11–14
  digits for single-term AR or MA models and for the airline model. For models with both AR and MA terms its starting
  values are not reproduced by any reading of the Guide we tried (about 20). The objective function is identical at
  Modeler's own saved estimates, and in all 12 recorded probes ours is the lower one, so the difference is that
  Modeler's iteration stops earlier; we do not copy its stopping point.
- **No-constant ARMA near a unit root.** On three synthetic probe streams (ARMA(1,1) without a constant, on a series
  whose mean is about 50) our estimate of the error variance differs from Modeler's by about 3 %, so the confidence
  limits differ much more than elsewhere (in the first row by tens of percent). The same fits with a constant match to
  about the usual 2–4 digits.
- **Time-series Expert Modeler.** One of 93 recorded ARIMA Expert selections differs (Modeler picks white noise where we
  keep an MA(2) at a partial-autocorrelation statistic of 2.2), and one of 23 transfer-function selections differs in
  its noise ARMA term.

### 4. Missing nodes and features

| Missing | Impact |
|---|---|
| Graphics output and R syntax in Extension nodes | The options are absent |
| Licence-blocked (6) and out-of-scope (34) nodes | Listed in the catalog and greyed out in the palette, with the reason |
| Native models are not turned into nuggets after training in the app (C5.0, CHAID, ...), which needs a PMML writer | Train, then score by running the stream; nuggets imported in `.str` files score normally, and Python models do generate nuggets |
| Excel named ranges whose sheet name needs quotes (`'My Sheet'!A1:B2`) | Refused with a `GAP:` error. Modeler 19 itself crashes its server process on such names ([reported here](https://github.com/witold-andelie/data-mining-software/issues/1)), so there is no answer to compare with; names on sheets without spaces are read and match Modeler (`excel_named_*`) |
| Python Extension nodes from Modeler `.str` files | Kept, but usually do not run: how Modeler stores their settings has not been observed, and its Python extension API is Spark-based |
| TwoStep clustering on data whose CF tree needs a rebuild (more than three levels) | The node stops with a "not supported yet" error instead of producing a result. Seen on the team's `cluster_analysis` stream; the TwoStep streams in the corpus that fit the supported depth match |
| Supernodes | Shown and run, but cannot be edited |
| Settings forms cover the most-used nodes | Other nodes keep their imported settings and show them read-only |
| Python nodes have no Modeler oracle | Their score field names follow Modeler's naming but are inferred |
