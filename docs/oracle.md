# 标准答案（oracle）

> 更新：2026-10-16

IBM SPSS Modeler 只用作测试对照：用它跑同一个流，导出结果，Rust 版的输出逐项比对。Modeler 不被链接、打包或分发；它的输出只存在本地 `oracle/`（gitignored）。

## 用法（仅 Windows）

```sh
# 一次性：Modeler 安装目录（含 bin\clemb.exe）
setx DMS_MODELER_HOME "D:\Program Files\spss"
# 可选：额外的数据目录，用 ; 分隔
setx DMS_ORACLE_DATA_DIRS "D:\some\data;D:\other\data"

cargo xtask corpus sync ibm-demos "D:\Program Files\spss\Demos\streams"
cargo xtask oracle ibm-demos druglearn.str [--timeout 600]
```

## 工作方式

1. 从 `corpus/<label>/` 取流，先按 `xtask/corpus.tsv` 校验 sha256。
2. 用 `xtask/oracle/modeler_oracle.py` 生成脚本（Jython 2.7，只用 IBM 公开的 Python 脚本接口），通过 `clemb -script … -execute` 在本地批处理模式运行（不需要 Modeler Server，也不弹界面）。
3. 脚本：
   - 源节点引用的文件不存在时，按文件名在 Modeler 的 `Demos`、`corpus/<label>/`、`DMS_ORACLE_DATA_DIRS` 中查找并替换；流自带的导出节点改写到 `exports/`。
   - 逐个运行终端节点（一个分支失败不影响其他分支，结果记在 `runs`），把生成的模型导出为 PMML（`models/`），把输出节点生成的 `DocumentOutput` 导出到 `outputs/NN_<type>_<id>.<ext>`（记在 `manifest.json` 的 `"outputs"` 下，含 `id`、`type`、`label`、`file`、`format`）。
   - 在每个终端节点的上游接一个 Flat File 导出（UTF-8、逗号、带表头），写到 `data/`。
   - `manifest.json` 记录导出、重定位、缺失的数据文件和所有错误。
4. xtask 写 `provenance.json`：流的 sha256、Modeler 版本、数据目录、耗时、每个输出文件的大小和 sha256。

## 输出节点（`outputs/`）导出格式

`taskrunner.exportDocumentToFile(output, path, fmt)` 对不同输出节点支持的 `FileFormat` 不同（不支持的格式会写出 0 字节文件）。脚本按优先级尝试 `COMMA_DELIMITED`（`.csv`）→ `JSON`（`.json`）→ `PLAIN_TEXT`（`.txt`）→ `HTML`（`.html`），取第一个非空结果：

| 节点类型 | 导出格式（`format`） | 文件内容与表结构 | 数值与特殊值打印规则 |
|---|---|---|---|
| `dataaudit`（Data Audit） | `csv`（`.csv`） | 两张逗号分隔表，中间隔一个空行：<br>1. **Audit 统计表**：默认表头 `Field,Measurement,Min,Max,Mean,Std. Dev,Skewness,Unique,Valid`；开启 `advanced_stats`/`median_stats` 时表头扩展为 `Field,Measurement,Min,Max,Sum,Range,Mean,Mean Std. Err.,Std. Dev,Variance,Skewness,Skewness Std. Err.,Kurtosis,Kurtosis Std. Err.,Median,Mode,Unique,Valid`。<br>2. **Quality 质量表**：表头 `Field,Measurement,Outliers,Extremes,Action,Impute Missing,Method,% Complete,Valid Records,Null Value,Empty String,White Space,Blank Value`。 | 连续型统计量（Mean、Std. Dev、Skewness、Kurtosis 等）受流属性 `decimal_places = 15` 控制（15 位小数）；整数统计量（Count、Unique、Valid、整数字段的 Min/Max/Sum/Range/Median）不带小数；`% Complete` 固定 3 位小数（如 `91.667`）；不适用项输出 `--`；存在并列众数时 `Mode` 末尾附上角标 `¹`（UTF-8 `0xC2 0xB9`，取最小众数）。 |
| `analysis`（Analysis） | `json`（`.json`） | 单行 JSON 树 `{"data": [...]}`。每个节点含 `"title"`（如 `"Results for output field <target>"`、`"Comparing $R-<target> with <target>"`、分组标题 `"<by_field> = <val>"`）和嵌套 `"data"` 或表格 `"table"`（二维字符串数组，配 `"format"`）：<br>- 准确率表（`FORMAT_ROW_HEADER` 或带分区列的 `FORMAT_ROW_AND_COLUMN_HEADER`）：行 `["Correct", count, pct]`、`["Wrong", count, pct]`、`["Total", count, ""]`。<br>- 混淆矩阵（`coincidence = true`，`FORMAT_MATRIX`）：首行 `["" 或分区说明, pred_cat1, ...]`，后续每行 `[actual_cat, n1, n2, ...]`。<br>- 置信度表（`confidence = true`，`FORMAT_ROW_HEADER`）：`Range`、`Mean Correct`、`Mean Incorrect`、`Always Correct Above`、`Always Incorrect Below`、`100% Accuracy Above`、`N Fold Correct Above`。<br>- 评估指标（`Evaluation Metrics`）：`["Model", "AUC", "Gini"]`。 | 计数为整数字符串（千位以上带逗号，如 `"1,660"`）；百分比最多 2 位小数并去掉尾随零（如 `"66.67%"`、`"80.5%"`、`"75%"`）；置信度与 AUC/Gini 固定最多 3 位小数且去尾零（不受 `decimal_places = 15` 影响）。 |
| `matrix`（Matrix） | `csv`（`.csv`） | 单张逗号分隔交叉表：<br>- **仅计数或数值函数**（单行/单元格）：表头 `<row_field>,<col_cat1>,<col_cat2>,...[,Total]`；每行 `<row_val>,<v1>,<v2>,...`；开启 `include_totals` 时末列/末行为 `Total`（`cell_contents = Function` 时不输出 `Total` 行列）。<br>- **勾选百分比/期望值/残差**（多子行）：第二列插入统计量标签列（表头为空 `<row_field>,,<col_cat1>,...,Total`），每个 `<row_val>` 展开为 `Count`、`Expected`、`Residual`、`Row %`、`Column %`、`Total %` 子行（仅首子行带 `<row_val>`，其余子行首列为空）。 | 计数为整数；百分比、期望值、残差及单元格聚合函数（`Mean`/`Sum`/`Min`/`Max`/`SDev`）固定 3 位小数（如 `75.000`、`2.917`），整百或空桶输出整数 `100` 或 `0`（不受 `decimal_places = 15` 影响）；包含缺失值时字符串空串对应空列头/行头，系统缺失值 `$null$` 输出 `$null$`。 |
| `statistics`（Statistics） | `json`（`.json`） | 单行 JSON 树 `{"data": [...]}`：<br>1. 每个 `examine` 字段一个分支（`"title": "<field>"` → `"title": "Statistics"`），含 `FORMAT_ROW_HEADER` 表 `[[stat_name, value], ...]`（`Count`、`Mean`、`Sum`、`Min`、`Max`、`Range`、`Variance`、`Standard Deviation`、`Standard Error of Mean`、`Median`、`Mode`），多众数时附说明节点 `"*Multiple modes exist.  The smallest value is shown."`。<br>2. 相关性表（`"title": "Pearson Correlations"`，`FORMAT_ROW_AND_COLUMN_HEADER`）：首行 `["", f1, f2, ...]`，每行 `[fi, "r/Label", ...]`（JSON 中 `/` 转义为 `\/`）。 | 实数统计量与相关系数 `r` 按 `decimal_places = 15` 输出 15 位小数；`Count` 及整数输入字段的 `Sum`/`Min`/`Max`/`Range`/`Median` 输出整数；多众数时 `Mode` 值末尾带 `*`；开启 `label_correlations` 时单元格格式为 `<r>/<Strength>`（如 `"-0.513557827673264/Weak"`）。 |
| `means`（Means） | `csv`（`.csv`） | 单张逗号分隔表：<br>- `BetweenGroups`：表头 `Field,Mean (<g1>),Std. Dev. (<g1>),Std. Err. (<g1>),Count (<g1>),...,F-Test,df1,df2[,Importance]`（`output_view = Advanced` 时包含 `F-Test,df1,df2,Importance`，`Simple` 视图仅含分组统计与 `Importance`）。每行对应一个 `test_fields` 字段。 | `Mean`、`Std. Dev.`、`Std. Err.`、`F-Test`、`Importance` 均按 `decimal_places = 15` 输出 15 位小数；`Count`、`df1`、`df2` 为整数；样本量为 1 的组 `Std. Dev.` 输出 `$null$`，`Std. Err.` 输出 `--`。 |
| `table`（Table） | `csv`（`.csv`） | 逗号分隔表：首行为字段名表头，后续每行一条记录。 | 实数按 `decimal_places = 15` 输出 15 位小数，整数原样输出；数值型与符号型系统缺失值输出 `$null$`，空字符串输出空字段（注意：这与 `outputfile` 导出到 `data/` 时把 `$null$` 写成空字段不同）。 |
| `report` / 图表节点 | `txt` / `html` | `report` 导出为纯文本（`.txt`）；图形节点（`distribution`、`histogram`、`plot`）仅支持 `HTML`（`.html`，内嵌 base64 PNG）。 | — |

## 可复现性（实测）

`ibm-demos/druglearn.str` 连跑两次：数据 CSV 和 C5.0 PMML 逐字节一致；神经网络 PMML 只有 `<Header><Timestamp>` 不同，权重一致（Modeler 用固定种子）。比对模型时解析 PMML 并忽略头部时间戳。

## 现状

| 流 | 结果 |
|---|---|
| `ibm-demos/druglearn.str` | 2 个模型、1 份数据，0 错误 |
| `team/cluster_analysis.str` | 3 个模型、8 份输出（2 `dataaudit`、1 `statistics`、3 `matrix`、1 `table`、1 `transform`）、7 份数据；1 个散点图节点失败：它引用的 `$KM-K-Means` 在该分支上不存在（流本身的问题，Modeler 界面里同样报错） |
| `team/decision_trees_heart.str` | 2 个模型、4 份输出（1 `dataaudit`、2 `analysis`、1 `distribution`）、4 份数据，0 错误 |
| `team/linear_regression.str` | 3 个模型、3 份输出（1 `dataaudit`、2 `plot`）、3 份数据，0 错误 |
| `team/logistic_regression.str` | 2 个模型、7 份输出（1 `dataaudit`、1 `analysis`、1 `table`、2 `distribution`、2 `histogram`）、6 份数据，0 错误 |
| `team/model_comparison.str` | 4 个模型、5 份输出（1 `dataaudit`、2 `analysis`、1 `table`、1 `histogram`）、6 份数据，0 错误（`tree_credit.sav` 用 Modeler Demos 里的） |
| `team/stream_1_M.str` | 13 份输出（3 `dataaudit`、6 `distribution`、4 `histogram`）、9 份数据，0 错误 |
| `team/stream_2_M.str` | 5 份输出（1 `dataaudit`、4 `table`）、5 份数据，0 错误 |
| `outputs/*.str`（10 个） | 输出节点的探针流（`xtask/oracle/probes/output_streams.py` 生成：2 `dataaudit`、2 `analysis`、3 `matrix`、1 `statistics`、1 `means`、1 `table`），全部 1 份输出、1 份数据、0 错误 |

团队流的数据文件已加入 `team` 语料（`str,csv,sav,dat,txt`）。

2026-10-16 起导出用 `decimal_places = 15`（实数按 15 位小数导出），上表各流已全部重跑。另外：

| 流 | 结果 |
|---|---|
| `ibm-demos/condlearn.str` | 2 个模型、1 份数据，0 错误 |
| `ibm-demos/condplot.str` | 3 份数据，0 错误 |
| `ibm-demos/pm_binaryclassifier.str` | 1 个模型、2 份数据，0 错误（约 4 分钟） |
| `ibm-demos/pm_decisionlist.str` | 3 份数据，0 错误 |
| `ibm-demos/bayes_bankloan.str`、`bayes_churn_retrain.str` | 部分导出；本机 Modeler 不认识 Bayes Net 节点（`unknownterminal`），其下游的评估、分析节点随之失败 |
| `ibm-demos/telco_coxreg.str` | 部分导出；Cox 节点未知，两个图节点没有可用字段 |
| `records/*.str`（30 个） | 记录操作节点的探针流（`xtask/oracle/probes/record_streams.py` 生成），全部 1 份数据、0 错误 |
| `models/*.str`（23 个） | 确定性模型训练与评分的探针流（`xtask/oracle/probes/model_streams.py` 生成），全部 1 个模型、2 份数据、1 份输出、0 错误 |

Modeler 导出 CSV 时不转义字段里的双引号（值 `a"b` 写成 `"a"b"`），`xtask compare` 按这种方言读取 oracle 文件。

## Model probes（`corpus/models/` 与 `oracle/models/`）

团队语料流在训练节点前接了未设种子的 `Partition` / `Balance` 节点，每次运行到达建模节点的数据都会变化。`xtask/oracle/probes/model_streams.py` 构造了一组完全确定性的训练与评分探针流（标签 `models`）：

- **数据文件**（拷贝到 `corpus/models/` 并固定在 `xtask/corpus.tsv`）：
  - `DRUG1n`（200 行，字段：`Age` `Range`、`Sex` `Flag`、`BP` `Set`、`Cholesterol` `Flag`、`Na` `Range`、`K` `Range`、`Drug` `Set`）
  - `tree_credit.sav`（2,464 行，字段：`Credit_rating` `Set`、`Age` `Range`、`Income` `OrderedSet`、`Credit_cards` `Set`、`Education` `Set`、`Car_loans` `Set`）
  - `HEART.csv`（270 行，字段：`age`、`resting blood pressure`、`max heart rate` 等，目标 `heart disease` `Flag`）
- **流拓扑**：`source -> Type`（显式设置每个字段的测量级别与角色：单目标 `Target`、特征 `Input`、其余字段 `None`）后分两支：
  1. `Type -> build_node`（终端建模节点，模型名显式设为 `<case>`，支持随机种子的节点统一设 `12345`，C&R Tree 设 `train_pct = 0` 关闭内部 30% 防过拟合随机留出集以使用全部训练记录）；
  2. `Type -> apply<xxx> -> Table`（由建模节点生成的模型块接在 `Type` 之后并连到 `Table` 终端节点）。
  流中不含 `Partition`、`Sample`、`Balance` 节点，`decimal_places = 15`。
- **每个 `oracle/models/<case>/` 目录包含**：
  - `models/01_<case>.xml`：建模节点导出的 PMML 模型；
  - `data/01_apply<xxx>_<id>.csv`：模型块评分后到达 `Table` 节点的记录（含 `$R-*` / `$C-*` / `$L-*` / `$E-*` / `$$kmeans` / `$T-*` 等评分与概率/距离字段）；
  - `data/02_type_<id>.csv`：到达建模节点的精确训练输入记录；
  - `outputs/01_table_<id>.csv`：`Table` 终端节点的 `DocumentOutput` 导出；
  - `manifest.json` 与 `provenance.json`。

| 用例（`.str`） | 建模节点 / 模型块 | 数据集 | 目标（Target） | 输入（Inputs） | 关键设置 |
|---|---|---|---|---|---|
| `cart_default` | `cart` / `applycart` | `DRUG1n` | `Drug` (`Set`) | `Age, Sex, BP, Cholesterol, Na, K` | 默认（`impurity_measure=Gini`, `use_max_depth=Default` [5], `prune_tree=True`, `train_pct=0`, `seed=12345`） |
| `cart_depth3` | `cart` / `applycart` | `DRUG1n` | `Drug` (`Set`) | `Age, Sex, BP, Cholesterol, Na, K` | `use_max_depth=Custom`, `max_depth=3`, `impurity_measure=Gini`, `train_pct=0`, `seed=12345` |
| `cart_unpruned` | `cart` / `applycart` | `DRUG1n` | `Drug` (`Set`) | `Age, Sex, BP, Cholesterol, Na, K` | `prune_tree=False`, `impurity_measure=Gini`, `train_pct=0`, `seed=12345` |
| `cart_min_records` | `cart` / `applycart` | `DRUG1n` | `Drug` (`Set`) | `Age, Sex, BP, Cholesterol, Na, K` | `prune_tree=False`, `use_percentage=False`, `min_parent_records_abs=30`, `min_child_records_abs=15`, `train_pct=0`, `seed=12345` |
| `cart_regression` | `cart` / `applycart` | `DRUG1n` | `Na` (`Range`) | `Age, Sex, BP, Cholesterol, K` | 连续目标回归树（LSD），`prune_tree=False`, `use_max_depth=Custom`, `max_depth=4`, `train_pct=0`, `seed=12345` |
| `cart_surrogates` | `cart` / `applycart` | `tree_credit.sav` | `Credit_rating` (`Set`) | `Age, Income, Credit_cards, Education, Car_loans` | `max_surrogates=2`, `prune_tree=False`, `use_max_depth=Custom`, `max_depth=3`, `train_pct=0`, `seed=12345` |
| `cart_no_surrogates` | `cart` / `applycart` | `DRUG1n` | `Drug` (`Set`) | `Age, Sex, BP, Cholesterol, Na, K` | `max_surrogates=0`（禁用代理分裂）, `prune_tree=False`, `train_pct=0`, `seed=12345` |
| `cart_twoing` | `cart` / `applycart` | `DRUG1n` | `Drug` (`Set`) | `Age, Sex, BP, Cholesterol, Na, K` | `impurity_measure=Twoing`, `prune_tree=False`, `train_pct=0`, `seed=12345` |
| `chaid_default` | `chaid` / `applychaid` | `DRUG1n` | `Drug` (`Set`) | `Age, Sex, BP, Cholesterol, Na, K` | 默认（`method=Chaid`, `use_max_depth=Default` [5], `split_alpha=0.05`, `merge_alpha=0.05`, `seed=12345`） |
| `chaid_exhaustive` | `chaid` / `applychaid` | `tree_credit.sav` | `Credit_rating` (`Set`) | `Age, Income, Credit_cards, Education, Car_loans` | `method=ExhaustiveChaid`, `seed=12345` |
| `chaid_depth2` | `chaid` / `applychaid` | `DRUG1n` | `Drug` (`Set`) | `Age, Sex, BP, Cholesterol, Na, K` | `method=Chaid`, `use_max_depth=Custom`, `max_depth=2`, `seed=12345` |
| `chaid_alpha` | `chaid` / `applychaid` | `tree_credit.sav` | `Credit_rating` (`Set`) | `Age, Income, Credit_cards, Education, Car_loans` | `method=Chaid`, `split_alpha=0.01`, `merge_alpha=0.10`, `bonferroni_adjustment=False`, `seed=12345` |
| `chaid_regression` | `chaid` / `applychaid` | `tree_credit.sav` | `Age` (`Range`) | `Credit_rating, Income, Credit_cards, Education, Car_loans` | 连续目标（F 检验），`method=Chaid`, `use_max_depth=Custom`, `max_depth=3`, `seed=12345` |
| `kmeans_3clusters` | `kmeans` / `applykmeans` | `DRUG1n` | — | `Age, Na, K` | 连续输入，`num_clusters=3`, `gen_distance=True` |
| `kmeans_5clusters` | `kmeans` / `applykmeans` | `DRUG1n` | — | `Age, Na, K` | 连续输入，`num_clusters=5`, `gen_distance=True` |
| `kmeans_categorical` | `kmeans` / `applykmeans` | `DRUG1n` | — | `Age, Sex, BP, Cholesterol, Na, K` | 含分类输入（集合编码），`num_clusters=4`, `gen_distance=True` |
| `logreg_binomial` | `logreg` / `applylogreg` | `HEART.csv` | `heart disease` (`Flag`) | `age, resting blood pressure, max heart rate` | `logistic_procedure=Binomial`, `binomial_method=Enter`, `include_constant=True` |
| `logreg_multinomial` | `logreg` / `applylogreg` | `DRUG1n` | `Drug` (`Set`) | `Age, Sex, BP, Cholesterol, Na, K` | `logistic_procedure=Multinomial`, `method=Enter`, `include_constant=True`, `all_probabilities=True` |
| `linear_enter` | `linear` / `applylinear` | `DRUG1n` | `Na` (`Range`) | `Age, K` | `model_selection=None`（Enter），`use_auto_data_preparation=False`, `use_random_seed=True`, `random_seed=12345` |
| `regression_enter` | `regression` / `applyregression` | `DRUG1n` | `Na` (`Range`) | `Age, K` | `method=Enter`, `include_constant=True` |
| `c50_default` | `c50` / `applyc50` | `DRUG1n` | `Drug` (`Set`) | `Age, Sex, BP, Cholesterol, Na, K` | `output_type=DecisionTree` |
| `neuralnet_default` | `neuralnetwork` / `applyneuralnetwork` | `DRUG1n` | `Drug` (`Set`) | `Age, Sex, BP, Cholesterol, Na, K` | 默认 MLP，`use_random_seed=True`, `random_seed=12345` |
| `twostep_default` | `twostep` / `applytwostep` | `DRUG1n` | — | `Age, Sex, BP, Cholesterol, Na, K` | `standardize=True`, `cluster_num_auto=True` (2..15), `distance_measure=Loglikelihood`, `clustering_criterion=BIC` |

## 全量回归（`cargo xtask regress`）

`cargo xtask compare` 一次只跑一个流。`cargo xtask regress` 跑 `oracle/` 下所有有对应语料流的导出（每个 `oracle/<label>/<stem>/manifest.json`；`<stem>` 是语料路径去掉 `.str`、把 `/` 换成 `__`，找不到语料流的目录如 `oracle/probes/` 跳过），并给每个流一个分类：

| 分类 | 含义 |
|---|---|
| `match` | 所有终端节点的输入都与 Modeler 一致 |
| `statistical` | 不一致，但流里有按统计对等（`docs/statistical-parity.md`，`crates/exec/src/registry.rs` 的 `STATISTICAL`）训练的建模节点；这类训练结果本来就不会逐位一致 |
| `known-gap` | 不一致，且列在 `xtask/known-gaps.tsv`（每行 `label<TAB>stem<TAB>原因`）里；登记的原因优先于 `statistical`（例如 `team/decision_trees_heart` 的差异在 C&R Tree 之前的无种子 Balance 节点就已出现） |
| `regression` | 其余的不一致，或流根本跑不起来 |

```sh
cargo xtask regress                        # 全部语料，默认 2 个并发 dms-cli
cargo xtask regress --label records        # 只跑若干标签（--label 可重复）
cargo xtask regress --jobs 8 --out target/regress.tsv
```

每个不一致的流打印一行（分类、`label/stem`、匹配的终端输入数或错误），以及前几条终端节点差异；最后汇总一行 `N streams: a match, b statistical, c known GAPs, d regressions`。`--out` 把每个流写成一 TSV 行（`label`、`stem`、`class`、`detail`、`problems`，相对路径按仓库根目录解析）。

存在 `regression`，或 `known-gaps.tsv` 里的条目已经不再不一致（缺口修好了、条目过期）时，命令以非零状态退出；修好一个已知缺口后删掉对应的行。`--label` 同时限制过期检查的范围。

## 已知限制

- 只遍历顶层节点；超节点内部的源节点和终端节点还没处理。
- 图形节点（`distribution`、`histogram`、`plot`）的 `DocumentOutput` 只能导出为内嵌 PNG 的 HTML，不能导出数值表，因此仍需结合 `data/` 里的上游输入数据做校验。
