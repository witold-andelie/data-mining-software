# Roadmap

> 阶段：**P1 完成，进入 P2** · 更新：2026-10-15

## 核心数字

| 数字 | 值 | 类型 |
|---|---|---|
| MVP 节点覆盖 | 0 / 100 | 实测（`cargo xtask parity`） |
| 以后再做 / 不做 | 13 / 34 | 实测 |
| `.str` 语料解码成功率 | 72 / 72（团队 7、IBM 示例 41、探针 24） | 实测（`cargo xtask test-corpus`） |
| 语料用到的节点 | 团队 7 个流：31 种，全在 MVP 内；全部 48 个真实流：57 MVP、5 以后、9 不做，0 未映射 | 实测（`dms-cli coverage`） |
| 标准答案（oracle） | 8 个流可生成：团队 7 个 + druglearn；共 16 个模型、41 份数据 | 实测（`cargo xtask oracle`） |
| 语料 | 81 个文件（团队 16 含数据、IBM 示例 41、探针 24） | 实测（`xtask/corpus.tsv`） |

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

- 2026-10-15：**P1 完成**。`.str` 流文档是 Fast Infoset（ITU-T X.891）编码的 XML；按公开规范实现解码器 `dms-strfmt`（不 panic、无递归、迭代式析构），流模型 `dms-stream`（节点、连线、超节点、两种扩展标识），`dms-cli xml/inspect/coverage`；差分探针 `cargo xtask probe`；格式规格 `docs/formats/str.md`。
- 2026-10-15：`cargo xtask oracle` 打通：Modeler 本地批处理（`clemb`）无需 Server 许可、不弹界面；导出 PMML 和终端节点输入 CSV；跨机器数据路径按文件名重定位；druglearn 两次运行结果可复现（仅 PMML 时间戳不同）。
- 2026-10-15：建立仓库骨架、工作区 lints、`xtask`（ci / layers / parity / corpus）、146 个节点的目录（MVP 100 个）、语料清单 49 个文件。
