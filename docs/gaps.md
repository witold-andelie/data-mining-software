# 待办（按优先级排序）

> 更新：2026-10-18

1. **P2：类型系统和 CLEM**：存储类型、测量级别、缺失值、日期基准（流属性 `dateBaseline`、`date2DigitBaseline`）；CLEM 解析与求值的子集。
2. **P2：节点设置映射**：每个节点的脚本属性名 ↔ 文件元素名 ↔ 取值形式，用探针逐个确定（`docs/formats/str.md` 第 5 节），按团队流的使用频率排序（Type、Statistics File、Table、Var. File、Derive、Filter、Select、Data Audit……）。
3. **P2：执行器和第一批数据节点**：参考执行器 + Polars 执行器；`dms-cli run` 跑流，结果与 `oracle/` 标准答案比对。
4. **附件格式**：PMML（标准，P3 用）、StatXML、PSMData、扩展节点设置附件、`SCF\x01` 二进制、长度前缀格式。
5. **节点目录里仍未映射的标识**：没有样例的节点（如 Random Trees、GLE、LSVM、Extension Output）。遇到新样例时用 `dms-cli coverage` 发现并补上。
6. **在 Mac 上跑通 `cargo xtask ci`**（P0 完成标准，用户安排在后面）。
7. **Python 桥设计**（Python 节点已移到“以后”，暂缓）：Python 环境检测、Arrow 交换格式、模型持久化、错误和取消。
8. **CI**：私有仓库用 GitHub Actions 要消耗分钟数（macOS 机器按 10 倍计）。目前做法是本地 Windows 和 Mac 各跑 `cargo xtask ci`。
9. **oracle 覆盖面**：超节点内部的节点；分析、数据审核、统计等输出节点本身的结果（目前只导出它们的输入数据）。
10. **Apriori 仍拒绝的设置**（`apriori_default` / `apriori_thresholds` / `apriori_transactional` 的训练和评分已对齐）：非 `rule` 的评价度量、无前件规则、事件、`onlyTrueValues` 为假、非连续事务 id、重复预测、非 `notInBasket`、非 `confidence` 准则、`ignoreUnmatchedItems` 为假、`maxLengthSet` 不是 10、前件与后件集合不同。串联第二个模型块的 `$A1-` 没有样例。PMML 的 `Item` / `Itemset` 元素顺序未复现，比较按 id。
10. **Auto Data Prep 其余步骤**（节点已能匹配 `adp_balanced` / `adp_rescale` / `adp_dates`）：监督合并、监督分箱、特征选择与构造、时间字段、最小-最大缩放、目标 Box-Cox。见 `crates/exec/src/nodes/auto_data_prep.rs` 的 `GAP:`。
