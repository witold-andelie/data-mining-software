# Roadmap

> 阶段：**P0 骨架** · 更新：2026-10-15

## 核心数字

| 数字 | 值 | 类型 |
|---|---|---|
| MVP 节点覆盖 | 0 / 100 | 实测（`cargo xtask parity`） |
| 以后再做 / 不做 | 12 / 34 | 实测 |
| `.str` 语料解码成功率 | 未测（P1 解码器之后） | — |
| 标准答案（oracle） | 2 个流可生成（druglearn、model_comparison）；6 个团队流缺数据 | 实测（`cargo xtask oracle`） |
| 语料 | 49 个文件（团队 8 个、IBM 示例 41 个） | 实测（`xtask/corpus.tsv`） |

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

- 2026-10-15：`cargo xtask oracle` 打通：Modeler 本地批处理（`clemb`）无需 Server 许可、不弹界面；导出 PMML 和终端节点输入 CSV；跨机器数据路径按文件名重定位；druglearn 两次运行结果可复现（仅 PMML 时间戳不同）。
- 2026-10-15：建立仓库骨架、工作区 lints、`xtask`（ci / layers / parity / corpus）、146 个节点的目录（MVP 100 个）、语料清单 49 个文件。
