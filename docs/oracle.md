# 标准答案（oracle）

> 更新：2026-10-15

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
   - 逐个运行终端节点（一个分支失败不影响其他分支，结果记在 `runs`），把生成的模型导出为 PMML（`models/`）。
   - 在每个终端节点的上游接一个 Flat File 导出（UTF-8、逗号、带表头），写到 `data/`。
   - `manifest.json` 记录导出、重定位、缺失的数据文件和所有错误。
4. xtask 写 `provenance.json`：流的 sha256、Modeler 版本、数据目录、耗时、每个输出文件的大小和 sha256。

## 可复现性（实测）

`ibm-demos/druglearn.str` 连跑两次：数据 CSV 和 C5.0 PMML 逐字节一致；神经网络 PMML 只有 `<Header><Timestamp>` 不同，权重一致（Modeler 用固定种子）。比对模型时解析 PMML 并忽略头部时间戳。

## 现状

| 流 | 结果 |
|---|---|
| `ibm-demos/druglearn.str` | 2 个模型、1 份数据，0 错误 |
| `team/cluster_analysis.str` | 3 个模型、7 份数据；1 个散点图节点失败：它引用的 `$KM-K-Means` 在该分支上不存在（流本身的问题，Modeler 界面里同样报错） |
| `team/decision_trees_heart.str` | 2 个模型、4 份数据，0 错误 |
| `team/linear_regression.str` | 3 个模型、3 份数据，0 错误 |
| `team/logistic_regression.str` | 2 个模型、6 份数据，0 错误 |
| `team/model_comparison.str` | 4 个模型、6 份数据，0 错误（`tree_credit.sav` 用 Modeler Demos 里的） |
| `team/stream_1_M.str` | 9 份数据，0 错误 |
| `team/stream_2_M.str` | 5 份数据，0 错误 |

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

Modeler 导出 CSV 时不转义字段里的双引号（值 `a"b` 写成 `"a"b"`），`xtask compare` 按这种方言读取 oracle 文件。

## 已知限制

- 只遍历顶层节点；超节点内部的源节点和终端节点还没处理。
- 输出到屏幕的节点（表格、分析、图形）在批处理模式下没有输出，脚本改为导出它们的输入数据；分析类节点本身的统计结果暂未导出。
