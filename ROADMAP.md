# Roadmap

> 阶段：**P1、P2 完成；P3 除 Python 桥外完成；P0 只差 Mac 上的 CI；P4 放到最后** · 更新：2026-10-28

## 核心数字

| 数字 | 值 | 类型 |
|---|---|---|
| MVP 节点覆盖 | 85 / 85（其中 10 个按统计等价验收） | 实测（`cargo xtask parity`，2026-10-20） |
| 以后再做 / 许可证受阻 / 不做 | 已做 3 / 25；0 / 5；0 / 34 | 实测（`cargo xtask parity`） |
| 回归（全部带标准答案的流） | 1085 个流：994 完全一致、60 统计等价、31 已知 GAP、0 回归 | 实测（`cargo xtask regress`，2026-10-28） |
| 语料 | `xtask/corpus.tsv` 登记 1096 个文件，50 个 oracle 标签 | 实测 |

## 阶段

| 阶段 | 内容 | 完成标准 |
|---|---|---|
| P0 | 仓库骨架、分层、xtask、节点目录、语料、oracle 流程打通 | Windows 和 Mac 上 `cargo xtask ci` 通过；oracle 对 1 个示例流产出标准答案 |
| P1 | `.str` 格式规格文档 + `strfmt` 解码器 + 覆盖率报告 | 语料中所有 `.str` 解码无错误 |
| P2 | 类型系统、CLEM、数据节点、两套执行器、命令行跑流 | 选定流端到端结果与标准答案一致 |
| P3 | 建模（原生算法 + Python 桥）+ PMML | 模型指标和打分与标准答案一致 |
| P4 | egui 界面，macOS `.app`/`.dmg` | 在 Mac 上完成完整建模流程 |
| P5 | 打磨：导入率、性能预算、记分卡 | 达到约定目标 |

先做哪些节点按团队语料的使用频率排序；MVP 范围是单机版 Modeler 的标准面板（含 Python 面板）。

## 进度日志

- 2026-10-28：C5.0 剪枝：只让正增益测试进入平均增益门槛、提升时只重生长被选中的分支、提升最大的内部子分支、对重生长后的候选继续做子树提升、全局剪枝在误差增量相同时先坍缩叶子多的子树、并先做信息量检验（切分的比特数低于 log2（正增益检验数）即剪掉，由 90 个探针流反推）；`c50_prune` 6 例与 16 个 Auto Classifier C5.0 组件全部逐节点一致，`c50_presets/c50_wn100_expert` 也已一致，剩 `wn89`（平票）与 `mc3`/`mc5`（生长阶段）。Decision List 判别实验 21 例中 20 例完整一致。输出节点补上 flag/数值矩阵、成对 t 检验、利润/ROI 图。
- 2026-10-20：**MVP 85/85**。Excel 源节点（`excel_more` 证实空 `<row>` 元素是夹具问题、文本逗号读成句点）、XML 源、导入 PMML（回归与分类）。Auto Data Prep 按指南补齐异常值、min-max、目标 Box-Cox/缺失/异常值/重编码、监督与无监督合并（名义与序数）、监督分箱、特征选择、主成分特征构造、时间字段与日期年/自动单位（`adp_next`/`adp_corr`/`adp_more`/`adp_time` 共 43 例全部一致）。Decision List、Linear-AS、Sequence 用判别实验查到公开资料的尽头，结论记入文档；LSVM 停止点与 Cox（本机未装）确认不可复现。
- 2026-10-19：Time Series、Auto Cluster、Random Trees、Auto Classifier/Numeric；统计等价验收层（`docs/statistical-parity.md`）。
- 2026-10-18：PCA/Factor、Anomaly Detection（训练与打分）、Transform 输出、Apriori（表格与事务数据，规则与 oracle PMML 逐条一致）、Auto Data Prep（`misc_next` 三例）；Kohonen 仅打分。MVP 覆盖 65/85（许可证受阻的 5 个模型不计入）。
- 2026-10-15：**P1 完成**。`.str` 流文档是 Fast Infoset（ITU-T X.891）编码的 XML；按公开规范实现解码器 `dms-strfmt`（不 panic、无递归、迭代式析构），流模型 `dms-stream`（节点、连线、超节点、两种扩展标识），`dms-cli xml/inspect/coverage`；差分探针 `cargo xtask probe`；格式规格 `docs/formats/str.md`。
- 2026-10-15：`cargo xtask oracle` 打通：Modeler 本地批处理（`clemb`）无需 Server 许可、不弹界面；导出 PMML 和终端节点输入 CSV；跨机器数据路径按文件名重定位；druglearn 两次运行结果可复现（仅 PMML 时间戳不同）。
- 2026-10-15：建立仓库骨架、工作区 lints、`xtask`（ci / layers / parity / corpus）、146 个节点的目录（MVP 100 个）、语料清单 49 个文件。
