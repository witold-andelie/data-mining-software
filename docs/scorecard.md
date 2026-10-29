# 记分卡（P5）

> 测量：2026-10-29，提交 3f99a7e，Windows 10，release 构建。复现：`cargo build --release -p dms-cli`，
> `python xtask/scorecard/run_all.py`，`cargo xtask regress --out regress.tsv`，
> `python xtask/scorecard/summarize.py regress.tsv`。

## 导入率：语料里的流能不能跑

语料登记 1257 个流，其中 23 个是只验证设置读写的探针流（没有终端节点），不计入。

| 口径 | 结果 |
|---|---|
| 1234 个有终端节点的流，所有终端都产出结果 | 1162 个（94.2 %） |
| 加上与 Modeler 以同样方式报错的流（回归判定一致，例如 XML 源没有扫描字段、全局值未定义、存储无法确定） | 1191 个（96.5 %） |
| 部分终端产出 / 一个也没有 | 22 / 50（含上面 29 个与 Modeler 同样报错的） |

剩下 43 个流里，与 Modeler 行为不同或 Modeler 这里也跑不了的：

- 本机 Modeler 许可证不含或未安装的节点：Bayes Net、SVM、Discriminant、GenLin、Cox（7 个流）。
- 不做的节点：CPLEX、Simulation Generate、Space-Time-Boxes；以后再做：SLRM、GLMM、Streaming TS、STP、地理空间导入
  （一个流可能同时用到几种未实现的节点或设置，这里只列节点）。
- Excel 夹具里没有单元格的空行（15 个，Modeler 自身读取不一致，已知 GAP）。
- **IBM 演示流里可修的缺口（没有 oracle，回归测不到）**：
  - 可变文件源 `multiBlankDelimiters`：`drugplot`、`goodslearn`、`goodsplot`、`cplex_goodslearn`；
  - SAV 读取过严：`broadband_1/2.sav`（`F w.0` 格式的变量含非整数值）、`employee_data.sav`（格式代码 4 无小数）；
  - Time Series 使用 Type 节点角色（`autoModelFieldSelection`）：`catalog_forecast`、`catalog_streaming_timeseries`；
  - Statistics 文件源 `readValues=labelsAsData`：`modelingintro`；可变文件源 `treatSquareBracketsAsLists`：`stp_server_demo`；
  - `ADP_basic_demo`（模型块需要的字段 `callid` 不在输入里）、`telco_dataaudit`（模型块没有保存模型）、旧版 Auto 模型块 2 个。

## 速度

| | 每个流 |
|---|---|
| 本软件（release） | 中位数 0.010 s，90% 分位 0.021 s，最慢 0.285 s；1234 个流合计 18.7 s |
| Modeler（`clemb` 批处理跑 oracle） | 中位数 7.22 s，最少 6.01 s |

Modeler 的时间包含批处理启动（约 6 s，取最小值估计）和 oracle 脚本的导出工作，所以总时间之比
（中位数约 700 倍）主要是启动开销。扣除 6.01 s 后，Modeler 仍需 2 s 以上的 272 个流里，
Modeler / 本软件的中位数是 251 倍，最小 10 倍。

注意：语料里的数据都很小（几十到几千行），这些数字说明的是交互式小流的响应速度，不是大数据量的吞吐量。
大数据量的性能预算尚未测量。
