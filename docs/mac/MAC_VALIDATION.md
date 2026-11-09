# Mac 验证任务（P0：Apple 芯片上的 CI 和全量回归）

给在负责人 Mac（Apple Silicon M1，`aarch64-apple-darwin`）上运行的 Claude Code。目标是确认两件事：

1. 仓库在 macOS 上能编译，并通过 `cargo xtask ci`（P0 的完成标准之一）。
2. 引擎在 Mac 上的结果和 Windows 一致：用 Windows 录制的 Modeler 标准答案（oracle）跑全量回归，逐流对照 Windows 基线。macOS 的数学库（`exp`、`ln`、`powf` 等）在最后一位可能和 Windows 不同，迭代算法会放大这种差异。这只有拿 oracle 跑回归才能发现。

开始前先读仓库根目录的 `AGENTS.md`。Mac 上没有 `plan/` 和 `log/`，按它的说明从 `ROADMAP.md` 和 `docs/gaps.md` 开始。

## 分工

| 步骤 | 谁做 | 内容 |
|---|---|---|
| 0 | Claude Code | 环境：Rust 工具链、opencode、分支 |
| 2 | Claude Code | 放置数据（涉及 `~/Downloads` 和家目录） |
| 1、3、4 | **opencode Muse 子代理** | 只运行 `docs/mac/muse-steps.sh` 的五个子命令，不改文件 |
| 核查 | Claude Code | 自己读 `target/mac/` 下的日志和 TSV，核对 Muse 的汇报 |
| 5、交付 | Claude Code | 分析差异、平台修复、报告、推送 |

Muse 的已知问题：它会把真实的失败说成"上游的怪癖"，声称检查已通过而实际没有，用错参数后把失败报告成成功，还曾按绝对路径改动别的目录。所以它只拿到一个固定脚本的执行权，没有编辑权；它的汇报只作参考，**结论一律以 Claude Code 自己读到的日志和 TSV 为准**。

## 规则（除 AGENTS.md 外，额外适用）

- 只在分支 `mac-validation` 上提交并推送。不要提交到 `main`，不要合并，不要改写历史。
- `corpus/`、`oracle/`、`modeler-home/` 是 IBM 和团队的数据，绝不提交，也不上传到任何地方。`corpus/` 和 `oracle/` 已在 `.gitignore` 里；`modeler-home/` 放在仓库外面。`target/mac/` 下的日志含有 Modeler 的输出值，也不提交。
- Mac 上没有 Modeler。不要运行 `cargo xtask oracle`、`cargo xtask probe`、`cargo xtask corpus record`/`sync`，也不要运行 `database` 标签（它需要 Windows 的 ODBC）。
- 不要为了让结果一致去放宽比较容差、修改 `xtask/known-gaps.tsv` 或 oracle 文件。数值差异只记录、分析，然后报告。
- 只修明确属于平台问题的代码（路径分隔符、换行、大小写、文件锁、编译错误）。修复要通用，不能按平台写死特例；每个修复都要在 Windows 上同样成立。
- 提交信息用 Conventional Commits，结尾加一行 `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`。文件用 LF 换行。
- 只有需要负责人决定的事才停下来问。需要负责人在终端里交互操作时（比如登录），告诉他具体命令，让他在 Claude Code 里用 `! <命令>` 运行。

## 第 0 步：环境（Claude Code）

```sh
xcode-select --install          # 已装会提示 already installed；否则会弹出安装窗口，请负责人确认
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none
source "$HOME/.cargo/env"
cd ~/dev/data-mining-software   # 仓库位置以实际 clone 的为准
rustup show                     # 按 rust-toolchain.toml 自动安装 1.95.0 + rustfmt + clippy
git switch -c mac-validation
```

安装 opencode（按 opencode 官方说明；常见方式是 `npm install -g opencode-ai`，或 `curl -fsSL https://opencode.ai/install | bash`）。然后请负责人登录他的 opencode go 会员：`! opencode auth login`。确认模型可用：

```sh
opencode models --standalone | grep -i muse     # 应能看到 opencode-go/muse-spark-1.3-contributor
```

opencode 的注意事项（来自 Windows 上的经验）：

- **每一条 `opencode` 命令都必须带 `--standalone`。** 不带的话，它会启动后台服务，把以前挂起的会话自动恢复并继续消耗额度。
- `opencode run --help` 先看一下当前版本的参数；Windows 上用的是 v2.2.5，变体写在模型名后面（`#xhigh`）。

把记录写进报告：`rustc -vV`、`sw_vers`、`uname -m`（应为 `arm64`）、`opencode --version`。

## 第 2 步：放置数据（Claude Code）

负责人会把压缩包 `dms-data-d1d2836.tar.gz` 和它的 `.sha256` 文件放在 `~/Downloads/`。包里有三部分：`corpus/`、`oracle/`（不含回归不用的 `probes/` 和 4.6 GB 的 `sources/excel_named_range`），以及 `modeler-home/Demos`（Modeler 自带的示例数据，ibm-demos 的流要用）。

```sh
cd ~/Downloads && shasum -a 256 -c dms-data-d1d2836.tar.gz.sha256
cd ~/dev/data-mining-software
tar -xzf ~/Downloads/dms-data-d1d2836.tar.gz corpus oracle
tar -xzf ~/Downloads/dms-data-d1d2836.tar.gz -C "$HOME" modeler-home
cargo xtask corpus verify
```

`muse-steps.sh` 默认使用 `$HOME/modeler-home` 作为 `DMS_MODELER_HOME`（回归按文件名在它的 `Demos/` 里找数据）。`.sha256` 文件是 Windows 上生成的（文件名前有 `*`）；如果 `-c` 不认，就用 `shasum -a 256 dms-data-d1d2836.tar.gz` 和文末的哈希值手动对比。

## 第 1、3、4 步：交给 Muse

1. 把权限模板放到仓库根目录，并让 git 忽略它：

   ```sh
   cp docs/mac/opencode.muse.json opencode.json
   echo opencode.json >> .git/info/exclude
   ```

   模板拒绝一切编辑和网络，shell 只允许 `bash docs/mac/muse-steps.sh ci|test-corpus|regress|compare|details` 这五条。

2. 在仓库根目录后台启动 Muse，输出存到 `target/mac/muse/`：

   ```sh
   mkdir -p target/mac/muse
   opencode run --standalone --auto -m 'opencode-go/muse-spark-1.3-contributor#xhigh' \
     --format json --title mac-muse \
     "Read docs/mac/MUSE_BRIEF.md and carry out exactly what it says." \
     > target/mac/muse/out.json 2> target/mac/muse/err.log
   ```

   参数以 `opencode run --help` 为准；如果 `#xhigh` 写法不被接受，就用当前版本指定变体的方式，或者去掉变体。

3. 看护：几分钟内 `out.json` 应该开始增长。如果超过 30 分钟没有增长，但 `target/mac/*.log` 也没有新内容，说明卡住了：结束进程后用同样的命令重启一次（Windows 上遇到过首个请求挂住的情况，重启即好）。`regress` 本身可能跑一个多小时，这期间 `target/mac/regress.log` 会持续增长，属于正常。
4. 结束后清理会话，防止以后被自动恢复：`opencode session list --standalone`，确认 `mac-muse` 会话已结束；必要时 `opencode session delete --standalone <id>`。
5. 删除仓库根目录的 `opencode.json`。

`muse-steps.sh` 各子命令的产物：

| 子命令 | 产物 |
|---|---|
| `ci` | `target/mac/ci.log`（fmt、clippy、`cargo test --workspace`、分层、对标清单） |
| `test-corpus` | `target/mac/test-corpus.log`（校验语料，并运行各 crate `corpus` feature 下的测试） |
| `regress` | `target/mac/regress.log`、`target/mac/mac-regress.tsv`（全部标签，排除 `database` 和 `probes`） |
| `compare` | `target/mac/mac-compare.txt`（逐流对照 Windows 基线） |
| `details` | `target/mac/details/<label>__<stem>.txt`（每个与基线不同的流各跑一次 `cargo xtask compare`） |

每个日志最后一行是 `muse-steps <step>: exit=… elapsed=…`。

## 核查（Claude Code）

不要直接相信 Muse 的汇报。自己读：

- 每个 `target/mac/*.log` 的最后一行（退出码、用时），失败时找到最先出现的错误；
- `target/mac/regress.log` 的汇总行，以及 `target/mac/mac-compare.txt`；
- `target/mac/details/` 是否覆盖了 `mac-compare.txt` 里列出的每个差异流。

缺了哪一步，就自己补跑那一步（直接运行 `bash docs/mac/muse-steps.sh <step>`）。预期有一处不同：`sources/excel_named_range` 的 oracle 没有拷过来，在 Mac 上会显示为 `absent`。

## 第 5 步：分析差异（Claude Code）

对每个和基线不同的流：

1. 从 `target/mac/details/` 看具体哪一行、哪个字段不同，差多少（绝对值、相对值，以及大约差几个 ulp）。
2. 归类：
   - **平台问题**：路径、换行、文件名、编码、排序时对字符串的比较方式等。修复后重跑相关步骤。
   - **浮点数差异**：同样的代码在 Mac 上算出最后几位不同。追到最先出现差异的那一步计算（哪个函数、哪个数学库调用），不要修，记下证据。
   - **其他**：如实记录。
3. 统计每一类有多少个流、涉及哪些算法。

`ci` 或 `test-corpus` 失败时同样归类。

## 交付

在分支 `mac-validation` 上提交：

- `docs/mac/MAC_REPORT.md`：环境、每一步的结果和用时、Muse 的汇报与你核查的结果是否一致、差异流清单及分类、浮点数差异的证据（最先分叉的计算）、平台修复的根因和改动、你建议的下一步（比如是否需要跨平台一致的数学函数实现）。
- `docs/mac/mac-regress.tsv`：用 `cut -f1-4 target/mac/mac-regress.tsv > docs/mac/mac-regress.tsv` 生成。第 5 列含有 Modeler 的输出值，不提交。
- 必要的平台修复。

然后推送：`git push -u origin mac-validation`。负责人会在 Windows 上审查并合并。

## Windows 基线（提交 d1d2836）

见 `docs/mac/windows-baseline.tsv`。汇总行：

`xtask regress: 1597 streams: 1465 match, 91 statistical, 41 known GAPs, 0 regressions`（Windows 10，`--jobs` 默认）。数据包 `dms-data-d1d2836.tar.gz`，56.6 MB，SHA-256 `72089bc762d6ea550b41d2e86cdbfa6607c935d7da201d0d7acfb7b7a5bc8aa9`。
