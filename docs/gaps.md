# 待办（按优先级排序）

> 更新：2026-10-18

Decision List 已支持训练（MVP 69/85）。9/10 指定模型的规则顺序、谓词、计数和 mining schema 一致；剩余 GAP：`models_next2/dl_max_rules` 响应率平票时，原型及 Rust 保留 north+web（80+60 个响应），Modeler 选 web+gold（60+55）。该流比较 1/3，其余 10 个流各 3/3。保留原型稳定顺序，未猜测新的平票规则。显式拒绝 down 搜索、`reuseFields=false`、频率/权重、缺失输入/目标、事件、交互状态、Split 字段；未知存储和资源上限返回错误。见 `docs/decision-list-validation.md`。

1. **P2：类型系统和 CLEM**：存储类型、测量级别、缺失值、日期基准（流属性 `dateBaseline`、`date2DigitBaseline`）；CLEM 解析与求值的子集。
2. **P2：节点设置映射**：每个节点的脚本属性名 ↔ 文件元素名 ↔ 取值形式，用探针逐个确定（`docs/formats/str.md` 第 5 节），按团队流的使用频率排序（Type、Statistics File、Table、Var. File、Derive、Filter、Select、Data Audit……）。
3. **P2：执行器和第一批数据节点**：参考执行器 + Polars 执行器；`dms-cli run` 跑流，结果与 `oracle/` 标准答案比对。
4. **附件格式**：PMML（标准，P3 用）、StatXML、PSMData、扩展节点设置附件、`SCF\x01` 二进制、长度前缀格式。
5. **节点目录里仍未映射的标识**：没有样例的节点（如 GLE、Extension Output）。Random Trees、Tree-AS、Linear-AS、LSVM 的 builder 和 nugget 标识已映射（`models_as`）；Tree-AS 已支持默认设置与深度 3/5 的训练，其余三个仍为显式 `GAP:`。
6. **在 Mac 上跑通 `cargo xtask ci`**（P0 完成标准，用户安排在后面）。
7. **Python 桥设计**（Python 节点已移到“以后”，暂缓）：Python 环境检测、Arrow 交换格式、模型持久化、错误和取消。
8. **CI**：私有仓库用 GitHub Actions 要消耗分钟数（macOS 机器按 10 倍计）。目前做法是本地 Windows 和 Mac 各跑 `cargo xtask ci`。
9. **oracle 覆盖面**：输出节点的报表本身已按 Modeler 导出逐项比对（`crates/exec/tests/outputs_*.rs`，覆盖 oracle 中除 1 例许可证受阻外的全部 119 份 Analysis/Data Audit/Statistics/Means/Matrix/Report/Transform/图形导出）。仍缺：超节点内部节点；Matrix 的 flag/数值矩阵、Means 的成对 t 检验、Evaluation 的利润/ROI 图、各输出节点的文件输出模式。
10. **Apriori 仍拒绝的设置**（`apriori_default` / `apriori_thresholds` / `apriori_transactional` 的训练和评分已对齐）：非 `rule` 的评价度量、无前件规则、事件、`onlyTrueValues` 为假、非连续事务 id、重复预测、非 `notInBasket`、非 `confidence` 准则、`ignoreUnmatchedItems` 为假、`maxLengthSet` 不是 10、前件与后件集合不同。串联第二个模型块的 `$A1-` 没有样例。PMML 的 `Item` / `Itemset` 元素顺序未复现，比较按 id。
10. **Auto Data Prep 其余步骤**（2026-10-20：异常值处理、min-max、目标 Box-Cox、监督/无监督合并、监督分箱、特征选择、特征构造已按指南实现，`adp_next` 13 例与 `adp_corr` 5 例全部匹配；`balance` 等预设目标不执行这些可选步骤）：时间字段与日期年单位/自动单位已按 `adp_time` 12 例实现；剩余：当前日期/时间作参考、时长与时间元素同时开启、字符串序数变量、序数目标、目标异常值设为缺失、多个目标步骤组合、权重、带缺失值的名义监督合并/分箱、构造特征与分类/日期字段并存时的字段顺序（序数输入与目标的缺失值、异常值、重编码已按 `adp_more` 10 例实现）。见 `crates/exec/src/nodes/auto_data_prep.rs` 的 `GAP:`。
11. **Time Series 训练**（`spss.ts!ts_builder`）：ARIMA / 指数平滑估计不复现。模型块 `spss.ts!ts_applier` 已按保存的模型评分：ARIMA（`ts_arima` none、`ts_expert` logarithmic）和指数平滑（`ts_simple` 简单、`ts_holt` Holt 线性趋势）。季节指数平滑、阻尼趋势、Winters、Brown、其它变换、季节 ARIMA、传递函数评分时 `GAP:`。
12. **Graphboard 其余图形**（`histogram_amount`、`pie_categories`、`line_score_amount`、`area_score_amount`、`scatter_score_amount`、`scatter_color_panel`、`boxplot_region_amount`、`dotplot_amount`、`heatmap_rows`、`parallel_fields`、`bubble_score_amount` 已对齐）：条形、计数饼、三维、地图、动画、覆盖、透明度、非 `MEAN` 的轴汇总、超过 `graphLargeDatasetReadThreshold` 的抽样或分箱。见 `crates/exec/src/nodes/graphboard.rs` 的 `GAP:`。
13. **Analytic Engine 模型**：16 个 `models_as` 流已对齐 48/48 终端输入。Tree-AS 三例的训练 PMML 已匹配，仍拒绝非默认合并/分裂阈值、非五箱、深度 3/5 以外的值、成本、权重、分区/拆分、缺失训练值和达到百万记录阈值的 effect-size 路径。Random Trees、LSVM、Linear-AS 的训练仍为 GAP。Linear-AS 默认例中，复用 Linear 选择 6 个参数，oracle 为 7 个（额外 x4²，beta=-1.32903806936708e-16），共同系数最大差 2.6832079490248025e-14；公开的相关矩阵 sweep 路径也未复现该冗余项，未保留尝试代码。2026-10-20 新增 `linas_discriminate`（21 例，Codex 设计）：在 24 例中 Modeler 有 13 例多选 `x4²`，按所选项集合比较即无任何候选规则存活（与数值容差无关），学习器未改。raw/adjusted propensity、`_GenerateSQL` 除 `false` 和 `native` 外仍拒绝。其他限制见 `docs/models.md` 第 6、7 节。
