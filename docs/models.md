# 建模节点设计（P3）

> 2026-10-16 · 依据：72 个语料流里模型块的附件、oracle 导出的模型（`oracle/<label>/<流>/models/*.xml`）和评分字段、`xtask/oracle/modeler_oracle.py` 的执行顺序。

## 1. 观察到的事实

**模型块（nugget）的存储。** 流里的模型块是 `ApplyXxxNode` 节点（如 `ApplyCartRuleNode`），`modelBuildInfo@modelBuilderId` 写明对应的建模节点（`BuildCartRuleNode`）。模型本身在 `.str` 的附件 `data/NNNN.dat` 里，格式有三种：

| 附件格式 | 出现位置 | 处理 |
|---|---|---|
| PMML 4.3（DMG 公开标准） | 团队流的 CART、CHAID（TreeModel）、K-Means、TwoStep（ClusteringModel）；示例流的广义线性、Cox 等（GeneralRegressionModel）、RuleSet、NaiveBayes、SVM、KNN | 读入后直接评分 |
| `Parameters`（IBM 扩展 XML，`xml.spss.com/pasw/extension`） | 与 PMML 成对出现，存建模设置和查看器信息 | 暂不读 |
| `SCF\x01` 二进制、内嵌 ZIP | 团队流的 Logistic、Linear、神经网络等 | 不解码（私有格式）；这类模型块必须由建模节点重新训练 |

**oracle 的执行顺序。** `modeler_oracle.py` 先逐个运行所有终端节点（建模节点也是终端节点；运行后 Modeler 用新模型替换流里与之相连的模型块），再导出每个终端节点的输入。所以模型块下游的 oracle 数据用的是**本次训练出的模型**，即同目录 `models/NN_<标签>.xml`（Modeler 导出的 PMML）。

**评分字段。** 模型块在输入字段后追加字段，前缀随算法而定（oracle 表头）：

| 模型 | 字段 |
|---|---|
| C&R Tree（CART） | `$R-<目标>`、`$RC-<目标>` |
| CHAID | `$R-`、`$RC-`、`$RRP-` |
| Logistic | `$L-`、`$LP-`、`$LRP-`；每个目标类别一个 `$LP-<类别>`（`appendProbabilities` 为真，或二项过程时总是输出） |
| Linear | `$L-` |
| K-Means | `$KM-<模型名>` |
| TwoStep | `$T-<模型名>` |
| Apriori | `$A-<模型名>-k` 后件、`$AC-<模型名>-k` 置信度、`$A-Rule_ID-k` 规则号（k = 1..=`associationMaxPredictions`，三个样例都是 3）。前件都在且后件不在篮里才命中；同一后件只留置信度最高的一条。事务数据写在每个连续 id 的最后一行（`apriori_transactional` 篮子 14） |
| Time Series | `$FutureFlag`（0/1，插在周期字段后）；每个目标 `$TS-`、`$TSLCI-`、`$TSUCI-`（`ts_arima`、`ts_expert`、`ts_simple`、`ts_holt`）。训练是 GAP，模型块用流里保存的 ARIMA 或指数平滑评分 |

同一流上串联多个同目标模型时，后加的带序号：`$R1-`、`$RC1-`、`$RRP1-`（`model_comparison`）。距离、概率等可选字段由模型块设置决定，逐个算法用 oracle 确认。

## 2. 架构

| crate | 层 | 内容 |
|---|---|---|
| `dms-pmml` | L0（无工作区依赖） | PMML 4.3 读写；模型中间表示（`DataDictionary`、`MiningSchema`、`LocalTransformations` 和各模型元素）；**评分**：`TreeModel`、`GeneralRegressionModel`、`RegressionModel`、`ClusteringModel`、`NeuralNetwork`、`AssociationModel`（规则列表，不是单值 `Prediction`），之后 `NaiveBayesModel`、`RuleSetModel`、`SupportVectorMachineModel`、`NearestNeighborModel`、`MiningModel`（集成）。自带一个小的值类型，不依赖 `dms-types`。 |
| `dms-algo` | L2 | 训练算法：CART、CHAID、K-Means、Logistic、Linear、TwoStep、C5.0、Apriori、神经网络……输出 `dms-pmml` 的模型表示。公式依据 IBM 公开的 Algorithms Guide。线性代数用 `faer`（计划已定）。 |
| `dms-exec` | L3 | 建模节点（训练）和模型块节点（评分）。 |

模型的内部表示直接采用 PMML 的结构：训练结果天然可以导出成 PMML，也可以和 oracle 导出的 PMML 逐项比较；读入的模型和自己训练的模型走同一条评分路径。

**执行器语义。**
1. 建模节点是终端节点：运行时训练模型，结果按建模节点的 uid 存进本次运行的模型表。
2. 模型块节点评分时按顺序取模型：(a) 本次运行中与它相连的建模节点刚训练出的模型（对应 Modeler 的“替换旧模型”）；(b) 模型块附件里的 PMML；(c) 都没有（私有格式且建模节点未运行）就报错并说明原因。
3. 先运行建模节点，再运行其余终端节点，与 oracle 顺序一致。
4. 模型块与建模节点的对应关系（流里怎么连、用什么标识）待探针确认（GAP）。

## 3. 验证

1. **评分**（不依赖训练）：对 oracle 里每个模型块的导出 `data/NN_applyxxx_*.csv`，用同目录 `models/` 中对应的 PMML 给导出里的输入字段评分，与导出里的评分字段比较。做成 `dms-exec` 的语料测试（`--features corpus`）。
2. **训练**：`xtask compare` 增加模型比较——我们训练出的 PMML 与 oracle 的 PMML 逐项比对（树的分裂字段、分裂值、叶子分布；回归系数；聚类中心），数值按相对误差，结构必须一致。
3. **端到端**：`cargo xtask compare` 的数据比较覆盖模型块下游（现有机制，不用改）。
4. 随机性：K-Means 初始中心、神经网络初始权重、TwoStep 等若依赖 Modeler 的随机数，只能比较质量指标；逐个算法记录在本文件。

## 4. 次序

| 步 | 内容 | 验证 |
|---|---|---|
| M1 | `dms-pmml`：解析；TreeModel、ClusteringModel、GeneralRegressionModel、RegressionModel 评分 | 第 3 节第 1 条，团队 6 个流的 12 个模型 |
| M2 | `dms-exec`：模型块节点（读附件 PMML）、评分字段命名、建模节点占位（未实现的算法报错） | decision_trees_heart、cluster_analysis 的模型块下游（附件是 PMML，不需要训练） |
| M3 | `dms-algo`：CART → CHAID → K-Means → Logistic → Linear → TwoStep，每个算法完成即接入建模节点 | 第 3 节第 2、3 条 |
| M4 | C5.0、神经网络，以及示例流里的其余模型 | 同上 |

## 5. 待确认（GAP）

- 模型块和建模节点在 `.str` 里如何关联；“替换旧模型”等设置的编码。
- 各模型块的可选评分字段（概率、距离、所有类别的概率）及其开关。
- `Parameters` 扩展 XML 是否包含评分需要、而 PMML 里没有的信息。
- 私有格式（`SCF\x01`）的模型块：只能重新训练；如果建模节点不在流里（只有模型块），这类流无法运行，需要提示用户在 Modeler 里把模型导出为 PMML。
- Apriori 已与 `apriori_default`、`apriori_thresholds`、`apriori_transactional` 对齐（规则、支持度、置信度、提升度、评分列）。仍拒绝、没有标准答案的设置：`evaluationMeasure` 不是 `rule`、`rulesWithoutAntecdents`、事件、`onlyTrueValues` 为假、非连续事务 id、`repeatPredictions`、`checkPredictions` 不是 `notInBasket`、`ruleCriteria` 不是 `confidence`、`ignoreUnmatchedItems` 为假、`maxLengthSet` 不是 10、前件集合与后件集合不同。串联第二个模型块的 `$A1-` 下标没有样例。Modeler 的 `Item` / `Itemset` 元素顺序不是 id 顺序，导出按 id 比较，不按文件中的元素顺序。

## 6. Analytic Engine 保存模型评分（2026-10-18）

`models_as` 的 16 个流全部匹配，每例 3/3，共 48/48 个终端输入。核对字段名、顺序、行数和全部单元格；数值按 `xtask compare` 的打印舍入与相对容差比较，不声称浮点逐位一致。训练不在范围内：`random_trees`、`tree_as`、`linear_as`、`lsvm` 都在 `SCORING_ONLY`；builder 返回 `OperatorError::Settings`，消息以 `GAP:` 开头。运行日志中的一个 builder 根因是预期行为，nugget 使用流内保存的 PMML。

| 目录 id | builder 标识 | nugget 标识 |
|---|---|---|
| `random_trees` | `spss.randomTrees!RT` | `spss.randomTrees!applyRandomTrees` |
| `tree_as` | `spss.treeEngine!treeEngine` | `spss.treeEngine!applyTree` |
| `linear_as` | `spss.linearEngine!linearEngine` | `spss.linearEngine!applyLinearEngine` |
| `lsvm` | `spss.linearSVM!linearSVM` | `spss.linearSVM!applyLinearSVM` |

本分支原目录没有 Tree-AS、Linear-AS 的独立条目，新增 `tree_as`、`linear_as` 并归入 later；仅评分不改变 MVP 已实现数。

这些流均通过 `contentContainers/containers/container[@name="PMML"]/content@content` 指向附件。附件是 4 字节大端长度加 ZIP，包含 `.meta` 和 `PMML.xml`，复用 `dms-strfmt` 的有界本地 ZIP 头读取器。未读 IBM 实现代码，未运行 Modeler、oracle 或 probe；仅读取保存的流、导出的 PMML/CSV 和现有探针脚本。

| 已验证规则 | 案例 |
|---|---|
| Random Trees 的 `majorityVote` 每棵树一票；`$R-<t>` 为多数类，`$RC-<t>` 为胜者票数/有效票数，与叶置信度无关。使用实际保存的 segment 数，不使用 builder 请求的树数 | `rt_default`、`rt_fewer`、`rt_shallow`、`rt_native`、`rt_conf` |
| `_calculate_conf=false` 省略独立 nugget 的 `$RC-`；`_GenerateSQL=false`（保存的默认值）和 `native` 的数值评分路径相同 | `rt_noconf`、`rt_batch` 的 Grid scores；`rt_native` |
| `average` 是保存树预测的算术平均；连续目标只输出 `$R-y`，不输出置信度，即使 `_calculate_conf=true` | `rt_numeric`、`rt_numeric_batch` |
| Tree-AS 复用 CHAID 的 Laplace 置信度 `(n_class+1)/(n_total+k)` 和 `lastPrediction`；分类输出 `$R-<t>`、`$RC-<t>`、`$RI-<t>`，最后一个为整数节点 id | `treeas_default`、`treeas_depth` |
| Tree-AS 的 `Discretize` 按保存的 Interval 边界分箱，无边界即无穷；如 x1 的分界为 13、22、32、41，左闭右开，末箱无上界。连续目标输出 `$R-y`、`$RI-y` | `treeas_numeric` |
| Linear-AS 复用 `generalLinear` 的 PPMatrix 参数乘积和 ParamMatrix 加权和，只追加 `$L-y` | `linearas_default`，零输入预测约 -1.9e-14 |
| LSVM 二分类复用 `multinomialLogistic`：参考类 Y 的决策值为 0，N 的决策值为参数加权和，softmax 得类别概率。输出 `$L-respond` 和 `$LC-respond`，二分类胜者置信度等于 logistic(abs(decision)) | `lsvm_default`、`lsvm_l1`；首行分别约 0.730410084338641、0.730802317826299 |
| LSVM 连续目标复用 `generalLinear`，仅追加 `$L-y` | `lsvm_numeric` |

**自动替换字段例外（观察与推断分开）。** `rt_noconf`、`rt_batch` 的保存流中，两份 nugget 的 `_calculate_conf` 都为 false；但 oracle 的原始自动回填 nugget 仍有 `$RC-respond`，独立 Grid scores 副本没有。前者保存了 `refreshLinkState=TurnOnReplaceLink` 和非空的 `repalcebuilderNodeId`（原文件拼写如此）。oracle manifest 和 `modeler_oracle.py` 显示 builder 会在导出输入前运行。原因推断为 builder 替换重置了原始 nugget 的评分字段；本实现按这两个保存标记复现观察到的字段，不训练，也不根据案例名、节点名或 uid 分支。无自动替换标记的 nugget 严格遵守 false。其他替换顺序尚未验证。

**GAP。** 四类训练、raw/adjusted propensity（包括 LSVM `_calculateRawProp=true`）未实现，明确拒绝；`_GenerateSQL` 除缺省、`false`、`native` 外的值（例如 `udf`）明确拒绝。非法布尔值同样返回带 `GAP:` 的 Settings 错误。LSVM 多分类、树平票没有 oracle；通用 majorityVote 在平票时采用目标 DataField 类别顺序，此行为有合成测试但未获 IBM 对照确认。Tree-AS `_display_rule_id=false` 的省略字段有合成测试，本批流均为 true。

规范依据为 DMG [PMML 4.3 Transformations](https://dmg.org/pmml/v4-3/Transformations.html) 与 [Multiple Models](https://dmg.org/pmml/v4-3/MultipleModels.html)；Modeler 设置来自公开脚本接口的既有探针记录 `xtask/oracle/probes/models_as_streams.py`。合成测试覆盖投票与叶置信度的区别、均值与溢出、分箱边界/缺失/default/畸形参数、lastPrediction/id、LSVM 决策值、设置拒绝和损坏附件；语料测试 `crates/exec/tests/models_as.rs` 覆盖全部 48 个终端输入。
