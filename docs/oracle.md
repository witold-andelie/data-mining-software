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
   - `runAll` 后把每个生成的模型导出为 PMML（`models/`）。
   - 在每个终端节点的上游接一个 Flat File 导出（UTF-8、逗号、带表头），写到 `data/`。
   - `manifest.json` 记录导出、重定位、缺失的数据文件和所有错误。
4. xtask 写 `provenance.json`：流的 sha256、Modeler 版本、数据目录、耗时、每个输出文件的大小和 sha256。

## 可复现性（实测）

`ibm-demos/druglearn.str` 连跑两次：数据 CSV 和 C5.0 PMML 逐字节一致；神经网络 PMML 只有 `<Header><Timestamp>` 不同，权重一致（Modeler 用固定种子）。比对模型时解析 PMML 并忽略头部时间戳。

## 现状

| 流 | 结果 |
|---|---|
| `ibm-demos/druglearn.str` | 2 个模型、1 份数据，0 错误 |
| `team/model_comparison.str` | 4 个模型、6 份数据，`tree_credit.sav` 重定位到 Demos，0 错误 |
| 其余 6 个团队流 | 缺数据文件：`CREDCARD.sav`、`HEART.csv`、`acct97.txt`、`customer.dat`、`customer_bank_data.csv`、`fraud_info.csv`、`goods.dat`、`telco_missing.sav` |

## 已知限制

- 只遍历顶层节点；超节点内部的源节点和终端节点还没处理。
- 输出到屏幕的节点（表格、分析、图形）在批处理模式下没有输出，脚本改为导出它们的输入数据；分析类节点本身的统计结果暂未导出。
