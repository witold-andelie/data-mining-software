# data-mining-software：给代理和协作者的说明

用 Rust 重写的轻量版数据挖掘工具，对标 IBM SPSS Modeler 19（单机功能）。团队内部使用，主要平台为 Windows 和 Apple 芯片 macOS。界面用 egui，不用 Tauri、Electron 或 WebView，不做 WebAssembly。

## 每次会话从这里开始

1. 读 `plan/STATUS.md`（当前阶段、下一步、本机路径），再读 `plan/execution-plan.md` 里对应阶段。`plan/` 和 `log/` 只在本地，不进 git；缺失时从 `ROADMAP.md` 和 `docs/gaps.md` 开始。
2. 结束前在 `log/devlog.md` 追加一条：做了什么、数字、还剩什么。保持每一步都能编译通过。
3. 只有真正需要负责人决定的事才停下来问（许可、发布、推送到新的远端、密钥）。

## Never crash（优先级高于功能）

用户的流和模型可能是几小时的工作。损坏的文件、错误的命令参数、磁盘满都只能产生可处理的错误，不能崩溃、不能丢数据。

- 非测试代码禁止 `unwrap()`、`expect()`、`panic!`、`unreachable!`、`todo!`、`unimplemented!`（工作区 lints 强制，`clippy.toml` 只在测试里放开）；`unsafe_code = "forbid"`。
- 错误用 `Result` 和 `?` 传递并补充上下文；未完成的功能返回 "unsupported" 错误。
- 来自文件、命令参数、MCP 的数字都视为恶意：用 `get()` 取下标，字符串只在字符边界切，长度和偏移用 `checked_*`/`saturating_*`，防止除零和 NaN 转整数，按输入分配内存前先设上限。
- 递归和循环要有深度上限或已访问集合（超节点可以嵌套）。
- 建模、执行流、Python 子进程等后台任务失败时报告错误、可以取消，不能让界面卡在“运行中”。
- 每个崩溃修复都带一个修复前会 panic 的回归测试；解析器（`.str`、`.sav`、CLEM、PMML）要有变异模糊测试。

## Clean room

- IBM 程序内部：不反编译、不反汇编 jar、DLL、exe，只看文件名清单。
- 依据公开资料实现：Modeler 用户手册、CLEM 参考、Python Scripting and Automation Guide（节点属性）、Algorithms Guide（算法公式）、PMML（DMG）、`.sav` 格式公开说明。所用资料的版本记在对应 crate 的 README。
- `.str` 格式靠差分观察：在 Modeler 里只改一个参数保存两份，对比字节，写进 `docs/formats/str.md`；解码器只按这份文档实现。
- 不读、不抄 GPL/LGPL/AGPL 代码（PSPP 等）。宽松许可的第三方实现只用来生成测试文件。
- IBM 和团队的文件（示例流、数据、Modeler 输出）只放在 `corpus/` 和 `oracle/`（gitignored），绝不提交。不使用 IBM 的图标、文案和配色。
- Modeler 只作为测试对照（oracle）：生成标准答案，不链接、不打包、不分发。

## 架构

| 层 | crate | 职责 | 状态 |
|---|---|---|---|
| L4 | `dms-engine` | Session、节点注册表、Modeler 节点目录（对标用） | 已建：目录 |
| tools | `xtask` | `cargo xtask ci / layers / parity / corpus` | 已建 |

计划中的各层见 `docs/architecture.md`。规则：

- **分层强制**：`cargo xtask layers`。一个 crate 只能依赖更低层；L6 以下不能用 egui、eframe、winit、rfd。新 crate 必须登记在 `xtask/src/layers.rs`。
- **一切都是命令**：用户可见的操作都是 engine 里带 id 的命令。界面、CLI、控制通道、MCP 都按 id 调用；程序调用不弹对话框。
- **对标可度量**：`crates/engine/src/catalog.rs` 列出 Modeler 的每个节点及其在 `.str` 里的标识。`cargo xtask parity` 生成 `docs/parity-checklist.md`。节点实现后加入 `crates/engine/src/nodes.rs`，并上调 `MVP_FLOOR`（只升不降）。
- **数据类型是运行时数据**：存储类型、测量级别、角色、缺失值、日期基准都不能写死。
- **Python 节点**：通过通用 Python 桥在用户自己的 Python 环境里运行（子进程，数据用 Arrow 交换）。不链接 libpython，不内置 Python。缺少 Python 或依赖包时给出可操作的错误。

## 质量闸门

每次提交前跑 `cargo xtask ci`（fmt、clippy `-D warnings`、测试、分层、对标清单）。提交信息用 Conventional Commits（`feat(strfmt): …`）。

## 语料和标准答案

- `xtask/corpus.tsv` 按标签、相对路径、SHA-256 固定语料。本机源目录记在 `plan/STATUS.md`。
- `cargo xtask corpus sync <label> <dir>` 把文件拷到 `corpus/<label>/` 并校验；`cargo xtask corpus verify` 校验现有语料；新增文件用 `cargo xtask corpus record`。
- Modeler 产生的标准答案放 `oracle/`，只在装有 Modeler 的 Windows 机器上生成。

## 环境

- Windows：过程文件、编译产物、缓存一律放 D 盘，不放 C 盘。并行代理各用一个 D 盘上的 `CARGO_TARGET_DIR`（每个约 10 GB，用完删除）。
- macOS：只支持 `aarch64-apple-darwin`。合并前在 Mac 上跑一遍 `cargo xtask ci`。

## 进度文档

`ROADMAP.md`（阶段、核心数字、进度日志）、`docs/gaps.md`（排好序的待办）、`docs/architecture.md`、`docs/parity-checklist.md`（自动生成）。工作改变了某个数字或关闭了某个缺口时，同步更新这些文档。
