# 架构

> 更新：2026-10-15 · 状态：P1 完成。已建 `dms-strfmt`、`dms-stream`（结构）、`dms-engine`（节点目录与识别）、`dms-cli`、`xtask`

## 分层

一个 crate 只能依赖更低层，由 `cargo xtask layers` 检查。L6 以下不使用任何 GUI 库。

| 层 | crate（计划） | 职责 | 阶段 |
|---|---|---|---|
| L0 | `dms-types` | 存储类型、测量级别、角色、缺失值、日期/时间语义 | P2 |
| L0（独立） | `dms-strfmt` | `.str` 流文件：ZIP 容器 + Fast Infoset（ITU-T X.891）解码，见 `docs/formats/str.md` | P1 已完成 |
| L0（独立） | `dms-sav`、`dms-pmml` | `.sav` 读写；PMML 读写 | P2 / P3 |
| L1 | `dms-stream` | 流模型（节点 DAG、参数、超节点）和原生文本格式（可 git diff） | P1 完成结构；参数与原生格式在 P2 |
| L1 | `dms-clem` | CLEM 表达式解析和求值 | P2 |
| L2 | `dms-io` | CSV、固定宽度、Excel、`.sav`、数据库、JSON、XML、SAS → Arrow/Polars | P2 |
| L2 | `dms-algo` | 原生算法：树族、回归、聚类、关联规则、神经网络等（线性代数用 faer） | P3 |
| L2 | `dms-python` | Python 桥：子进程、Arrow 数据交换、环境检测、模型持久化 | P3 |
| L3 | `dms-exec` | 参考执行器（逐行，作对照）和优化执行器（Polars lazy）；节点缓存、增量重算、任务、取消 | P2 |
| L4 | `dms-engine` | Session、撤销、命令注册表、节点注册表、节点目录 | P0 起 |
| L5 | `dms-mcp` | MCP 服务器（无界面，或连到运行中的应用） | P2 |
| L6 | `dms-ui` | egui 节点画布、数据预览、图表、模型查看器 | P4 |
| apps | `dms`、`dms-cli` | 桌面应用；命令行 `run / inspect / import-str / convert / mcp` | P1 起 |
| tools | `xtask` | `ci / layers / parity / corpus`，以后加 `oracle / scorecard / perf` | P0 起 |

## 关键设计

- **流文件**：原生格式是文本（TOML 或 JSON），可以进 git 和做代码评审。`.str` 只导入；导入时识别不了的节点和参数原样保留，界面上提示。
- **两套执行器**：参考执行器写得简单直接，作为优化执行器的对照；两者在测试里必须给出相同结果。
- **Python 节点**（2026-10-16 起移出 MVP、归入“以后”：团队的流一个都没用到；`dms-python` 随之推后）：Modeler 自带的 Python 3.10 里是 scikit-learn 1.1.1、xgboost 0.82、imbalanced-learn 0.8.0、hdbscan 0.8.29。我们的 Python 节点用同样的库，在用户自己的 Python 环境里以子进程运行。与 Modeler 对比的差分测试在 Windows 上用同版本环境；Apple 芯片 Mac 上没有 xgboost 0.82，用新版本，允许细微差异。
- **跨平台**：Windows x64 和 macOS aarch64。不依赖 BLAS（faer 是纯 Rust），便于在 Mac 上直接编译。
- **AS 模型评分**：Random Trees、Tree-AS、Linear-AS、LSVM 的 builder/nugget 均映射目录 id；`SCORING_ONLY` 让 builder 返回显式 `GAP:`，nugget 继续读取保存的 PMML。`dms-exec` 用有界 ZIP 读取器解开 AS 附件，`dms-pmml` 复用树和通用回归评分，仅增加分箱与集成汇总。未增加分层依赖；16 例、48 个终端输入已对齐。
