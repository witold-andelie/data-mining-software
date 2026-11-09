# Mac 应用实施方案（阶段 A 之后）

负责人决定（2026-11-09）：**当务之急是在 Mac 上做出一个能运行的软件版本**；Windows 的适配放到最后。本文件取代 `DESIGN.md` 第 12 节的实施顺序；`GUI_BRIEF.md` 和 `DESIGN.md` 的其他约束仍然有效，以下另有说明的除外。

## 1. 思路：先做只读的 v0.1

执行器已经能完整运行导入的 `.str` 流（`dms-cli run`），全量回归 1465 个流与 Modeler 一致。所以最快能用的版本不需要先做编辑：**打开 `.str` → 运行 → 看结果**，打包成 `.app`。它已经能在 Mac 上替代 Modeler 来运行现有的流。编辑、保存、属性表单在后面的版本里逐步加。

| 版本 | 目标 | 主要内容 |
|---|---|---|
| **v0.1** 能运行 | 打开现有流，运行，看结果 | 后台任务（`JobManager`，`catch_unwind`，进度，失败时定位到节点）；运行整个流或选中的终端节点；结果区：表格（虚拟滚动，百万行不卡）、统计和文本输出、模型块概要（树、系数、规则的文字视图）；`.app` 和 `.dmg`，在 Finder 里双击 `.str` 能打开 |
| **v0.2** 能编辑 | 修改流并保存 | `DESIGN.md` 的 B1（`StreamDoc`、可逆 `Edit`、TOML 原生格式 `.dms`、原子保存、`.str` 导入往返测试）和 B2（Session 撤销/重做、自动保存和恢复）；画布编辑：从节点面板添加、连线、移动、删除、复制粘贴、框选 |
| **v0.3** 能设置 | 改节点参数 | 按设置模式生成属性表单（先做团队流里用到的约 15 个节点）；英文、中文翻译表 |
| **v0.4** 打磨 | 体验完整 | 执行器取消（`ExecContext` 的 `CancelToken`）；图表（`egui_plot`）；画布的两个已知问题（竖直连线穿过标签；建模节点和模型块图标相同）；性能预算 |
| 最后 | Windows | 在 Windows 上完整走一遍；窗口内菜单的实际操作；Windows 冷启动（首次 2.7 s，再次 0.6 s） |

每个版本结束：推送分支、`.dmg` 放在 `target/dist/`（不提交），向负责人汇报并停下，等负责人在 Mac 上试用后再开始下一个版本。

### Windows 放到最后，意味着什么

- 不在 Windows 上做运行测试和界面调试；不为 Windows 写专门的代码。
- 代码仍要保持可移植：不用只有 macOS 才有的 API，非用不可时用 `cfg` 隔开。每个版本结束时在 Mac 上跑一次 `cargo check --target x86_64-pc-windows-msvc -p dms -p dms-ui -p dms-engine --all-targets`，只确认能编译。
- 负责人在 Windows 端审查合并时会跑完整的 ci；Windows 上发现的问题由 Windows 端处理。

## 2. 阶段 A 待决定问题：先按建议执行

负责人可以随时推翻，未推翻前按 `DESIGN.md` 第 11 节的建议执行：

- 界面语言：英文和中文，所有界面文字走翻译表；
- 视觉风格：egui 加我们自己的类 macOS 主题，菜单、文件对话框、窗口外框、深色模式用系统原生的；
- 节点图标自己画（矢量，形状加颜色），工具栏通用图标可以用 Lucide（ISC）或 Phosphor（MIT）；
- 应用只用系统字体，不打包 egui 自带字体；egui 字体只用在测试里；
- 允许的许可证增加 BSL-1.0 和 Unicode-3.0；MPL-2.0 只允许用于不随应用分发的测试依赖；
- 原生格式 TOML，扩展名 `.dms`；超节点 v1 只显示；
- 签名：团队内部使用，`.app` 用临时签名（`codesign --force --deep -s -`），不做公证；首次打开需要在 Finder 里右键选“打开”，写进使用说明。

## 3. 协调者和子代理

Mac 上的 Claude Code 是协调者，负责：架构决定、拆任务、写任务说明、审查子代理的改动、整合、验证、与负责人沟通、界面体验的判断。两个子代理可以同时运行：

| 子代理 | 模型 | 适合的任务 | 不适合 |
|---|---|---|---|
| opencode | `opencode/glm-5.3-flash-free` | **简单、边界清晰**的任务：运行固定的验证命令并原样汇报；按明确清单做机械改动（翻译表条目、按给定字段表填数据、重命名）；为已定好接口的纯函数补测试；文档整理 | 任何需要设计判断、跨模块改动或数值正确性的任务 |
| Codex | `gpt-6.1-sol` | **复杂、边界清晰**的任务：`JobManager` 和运行集成；结果表格的虚拟滚动；`StreamDoc`/`Edit`/TOML 格式；撤销/重做；执行器取消；属性表单框架 | 边界不清、需要和负责人商量的事；界面整体风格 |

“边界清晰”指任务说明里写明：要改哪些文件（只能改这些）、接口签名、验收标准（哪些测试必须通过、哪些命令的输出必须怎样）、禁止事项。说明写不清楚的任务，协调者自己做或先拆小。

### 3.1 并行的安全规则

- 每个子代理用自己的仓库副本：`git clone --local ~/dev/data-mining-software ~/dev/agents/<任务名>`，切到当前工作分支；`corpus`、`oracle` 用符号链接指到主仓库（只读使用）。不用 git worktree（Codex 的沙箱只允许写它自己的目录）。
- 每个副本用自己的 `CARGO_TARGET_DIR`（在副本的 `target/` 下即可），`CARGO_BUILD_JOBS=3`。同时最多一个 Codex 和一个 opencode 子代理。M1 只有 16 GB 内存，开始前看一下内存余量。
- 两个同时运行的任务改的文件不能重叠。每份任务说明列出“只能改”的文件，协调者分配时检查。
- 子代理不提交、不推送、不切分支、不安装软件、不联网（Codex 的 workspace-write 沙箱默认如此；opencode 用权限配置）。改动留在副本里，由协调者用 `git diff` 取出，审查后应用到工作分支。
- 子代理的汇报只作参考。协调者自己跑测试和检查，结论以自己看到的结果为准（`docs/mac/MAC_REPORT.md` 里对子代理的核查做法）。

### 3.2 Codex（gpt-6.1-sol）

- 安装：`npm install -g @openai/codex`（或按官方说明），负责人登录：`! codex login`（ChatGPT 账号）。
- 运行（在后台，输出存到 `~/dev/agents/<任务名>.out/`）：

  ```sh
  codex exec -m gpt-6.1-sol -c model_reasoning_effort="xhigh" -s workspace-write \
    -C ~/dev/agents/<任务名> --json -o ~/dev/agents/<任务名>.out/last-message.md - \
    < ~/dev/agents/briefs/<任务名>.md > ~/dev/agents/<任务名>.out/events.jsonl
  ```

  先用 `codex exec --help` 确认参数；模型名以 `codex` 实际接受的为准。
- 遇到额度用完（`usage limit`）：告诉负责人，不要自行换模型；未完成的改动留在副本里。

### 3.3 opencode（glm-5.3-flash-free）

- 每条 opencode 命令都带 `--standalone`；结束后删除会话（`opencode session delete --standalone <id>`）。
- 副本根目录放 `opencode.json`（加进副本的 `.git/info/exclude`）：`edit` 允许（只在副本内）、`external_directory` 和 `webfetch` 拒绝；`bash` 默认拒绝，只放行这个任务需要的确切命令（例如 `cargo test -p dms-ui*`、`cargo clippy*`、`cargo fmt*`）。
- 运行：`opencode run --standalone --auto -m 'opencode/glm-5.3-flash-free' --format json --title <任务名> "Read <说明文件> and carry out exactly what it says."`
- 免费模型限流或卡住超过 30 分钟：结束它，告诉负责人，不要换付费模型。

### 3.4 任务说明模板（每个子代理任务一份，放 `~/dev/agents/briefs/`，不提交）

1. 背景（两三句）和要读的文件；
2. 目标；
3. **只能改**的文件列表；不能改的文件（特别是 `corpus/`、`oracle/`、`xtask/known-gaps.tsv`、比较容差）；
4. 接口签名或数据格式（协调者定好的）；
5. 验收：必须通过的测试和命令（写出确切命令），新增测试的要求；
6. 禁止：提交、推送、联网、安装、`unwrap`/`panic`/`unsafe`（工作区 lints 已强制）、放宽测试；
7. 汇报格式：改了哪些文件、跑了哪些命令和结果（原样）、未完成的部分。

## 4. 验证关口

- 每次整合后：`cargo xtask ci` 在 Mac 上通过。
- 改动涉及 `crates/stream`、`crates/exec`、`crates/io`、`crates/algo`、`crates/clem` 时：再跑全量回归并对照基线（`bash docs/mac/mac-steps.sh regress`，然后 `compare`）。结果必须与 `docs/mac/mac-regress.tsv` 逐流相同；任何变化都要先查清原因，不能接受未解释的变化。
- 截图基准图只在协调者看过新图之后更新，并在提交信息里说明为什么变。
- 每个版本结束时，在 Mac 上从 Finder 双击 `.app` 实际走一遍目标流程（用 `corpus/team/` 的流），把结果写进汇报。

## 5. 分支和提交

- 工作分支 `mac-app`（从包含阶段 A 的 `main` 新建）。只在这个分支提交和推送，不碰 `main`；负责人在 Windows 端审查后合并。
- 提交信息用 Conventional Commits，结尾加 `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`；子代理写的代码在提交信息里注明（例如“implemented by Codex gpt-6.1-sol, reviewed by …”）。
- 文件用 LF；`corpus/`、`oracle/`、`modeler-home/`、`target/` 和任何含团队数据的截图都不提交。
