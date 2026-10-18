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
| Time Series | `$FutureFlag`（0/1，插在周期字段后）；每个目标 `$TS-`、`$TSLCI-`、`$TSUCI-`（`ts_arima`、`ts_expert`）。训练是 GAP，模型块用流里保存的 ARIMA 评分 |

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
