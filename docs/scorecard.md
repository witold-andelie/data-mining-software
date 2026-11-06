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
- **IBM 演示流里发现的缺口**（2026-10-29 补录 11 个演示流的 oracle 后处理）：
  - 已修：可变文件源 `multiBlankDelimiters` 在没有空白分隔符时不起作用（`drugplot`、`goodsplot`、`goodslearn` 一致）；
    SAV 的 `F w.0` 与 `DOLLAR w.0` 变量按整数读入并四舍五入（`broadband_apply_models`、`graphboard` 一致）；
    Auto Data Prep 只重编码字符串分类输入，数值型名义/flag 输入原样保留（`ADP_basic_demo` 4/4 一致）。
  - 已修：流日期格式 `format27`（MON YYYY）的 `to_date` 与 Type 导出（`broadband_create_models` 的 Type 终端一致）；
    Clementine 11 内联 `RuleTreeModel` 模型块（`telco_dataaudit` 2/2 一致）。
  - 仍缺：Time Series 的季节/变换传递函数（`broadband_create_models` 仍缺 applyts 终端）；Time Series 的 `_isOutputOthers` 与 `_calculatePI` 已确认不影响打分输出（`catalog_forecast` 一致）；Statistics 文件源
    `labelsAsData`（`modelingintro`，Modeler 批处理本身也找不到可执行节点）；STP 与 `treatSquareBracketsAsLists`
    （`stp_server_demo`）；旧版 Auto 模型块 2 个。

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
