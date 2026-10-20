# Statistical parity

Most nodes are held to **exact parity**: our output equals Modeler's oracle (to printed
precision, `cargo xtask compare`), and anything we cannot reproduce fails with `GAP:`.

A few model builders cannot be reproduced bit for bit, because Modeler draws from an
undisclosed random generator or stops an optimizer by an undocumented rule. Since 2026-10-19
(user decision) they are held to **statistical parity** instead: we implement the documented
algorithm with our own seedable generator or a converged optimizer, and accept the result when
Modeler's single run looks like one of ours.

| Builder | Why not exact | Oracles |
|---|---|---|
| Neural Net | random initial weights, random 70/30 split | `models_ml/nn_*` |
| Kohonen | random initial weights and record order | `models_next/kohonen_*`, `kohonen_size/*` |
| Random Trees | bootstrap and predictor draws, seeded by an undocumented hash | `models_as/rt_*`, `models_next/rt_*` |
| LSVM | ADMM stops before the optimum | `models_as/lsvm_*` |
| Time Series | smoothing/ARIMA optimizer stopping point | `models_next/ts_*`, `models_next2/ts_*` |
| Sequence | order of rules tied on every statistic | `train_next/sequence_*`, `models_next/sequence_*` |
| Auto Classifier / Numeric / Cluster | components above (Auto Classifier is exact when every enabled component is: `auto_train/ac_trees_nofs`) | `models_next2/ac_*`, `auto_train/*`, `an_default`, `acluster_default` |
| QUEST, C&R Tree with an overfit prevention set | the set is a random draw | `models_next/quest_*`, `cart_ofs*` |

QUEST and C&R Tree without an overfit prevention set stay exact (`quest_nofs/*`).

## Acceptance rules

1. **Same contract.** The node reads the same settings, writes the same output fields (names,
   storage, order) and a model the nugget can score. Unsupported settings are `GAP:` as usual.
2. **Seed band.** Train with at least 10 seeds of our generator. For each metric below,
   Modeler's value must lie inside our seeds' range widened by 10 % of its width (or inside an
   absolute tolerance stated per metric when the range is degenerate). The test prints the
   range and Modeler's value. With many oracles a few single runs fall outside any finite band
   by chance; such a case may be listed as a known outlier with its numbers in the test and the
   module docs, and the test asserts that it stays outside (Kohonen: 1 of 17, at 100 seeds).
   An outlier list is not a way around a systematic bias: Modeler's values must also rank near
   the middle of our runs across the family.
3. **Optimizers.** Where only the stopping point differs (LSVM, Time Series), solve the
   documented objective to convergence. Accept when our objective is no worse than the value
   of Modeler's own coefficients on the same data (up to 1e-9 relative), predicted classes are
   identical on every scored record, and continuous predictions differ from Modeler's by at most
   1 % of the target's standard deviation. Modeler's early stop can leave coefficients far from
   the optimum (`models_as/lsvm_numeric`: intercept -0.0155 against the optimum's -0.0359),
   so coefficients are reported, not compared.
4. **Ties.** Sequence must produce exactly Modeler's rules and statistics; only the order of
   rules tied on confidence, length and support is free.
5. **Labelled.** Module docs, the builder's notes and the parity checklist say "statistical
   parity" for these builders; never describe them as matching Modeler exactly.

## Metrics per builder

| Builder | Metrics on the oracle data (training and grid/score records) |
|---|---|
| Neural Net | accuracy (categorical) or RMSE (continuous) of `$N-`; the number of hidden units for fixed architectures |
| Kohonen | map size (exact); mean quantization error; number of non-empty units |
| Random Trees | accuracy or RMSE; number of trees kept by the accuracy stop rule |
| Auto models | the same candidate list (exact); the chosen models' ranking metric; ensemble accuracy |
| QUEST / C&R Tree with overfit set | accuracy; tree depth and leaf count |
