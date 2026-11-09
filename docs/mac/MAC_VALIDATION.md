# Mac 验证任务（P0：Apple 芯片上的 CI 和全量回归）

给在负责人 Mac（Apple Silicon M1，`aarch64-apple-darwin`）上运行的 Claude Code。目标是确认两件事：

1. 仓库在 macOS 上能编译，并通过 `cargo xtask ci`（P0 的完成标准之一）。
2. 引擎在 Mac 上的结果和 Windows 一致：用 Windows 录制的 Modeler 标准答案（oracle）跑全量回归，逐流对照 Windows 基线。macOS 的数学库（`exp`、`ln`、`powf` 等）在最后一位可能和 Windows 不同，迭代算法会放大这种差异。这只有拿 oracle 跑回归才能发现。

开始前先读仓库根目录的 `AGENTS.md`。Mac 上没有 `plan/` 和 `log/`，按它的说明从 `ROADMAP.md` 和 `docs/gaps.md` 开始。

## 规则（除 AGENTS.md 外，额外适用）

- 只在分支 `mac-validation` 上提交并推送。不要提交到 `main`，不要合并，不要改写历史。
- `corpus/`、`oracle/`、`modeler-home/` 是 IBM 和团队的数据，绝不提交，也不上传到任何地方。`corpus/` 和 `oracle/` 已在 `.gitignore` 里；`modeler-home/` 放在仓库外面。
- Mac 上没有 Modeler。不要运行 `cargo xtask oracle`、`cargo xtask probe`、`cargo xtask corpus record`/`sync`，也不要运行 `database` 标签（它需要 Windows 的 ODBC）。
- 不要为了让结果一致去放宽比较容差、修改 `xtask/known-gaps.tsv` 或 oracle 文件。数值差异只记录、分析，然后报告。
- 只修明确属于平台问题的代码（路径分隔符、换行、大小写、文件锁、编译错误）。修复要通用，不能按平台写死特例；每个修复都要在 Windows 上同样成立。
- 提交信息用 Conventional Commits，结尾加一行 `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`。文件用 LF 换行。
- 只有需要负责人决定的事才停下来问。

## 第 0 步：环境

```sh
xcode-select --install          # 已装会提示 already installed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none
source "$HOME/.cargo/env"
cd ~/dev/data-mining-software   # 仓库位置以实际 clone 的为准
rustup show                     # 按 rust-toolchain.toml 自动安装 1.95.0 + rustfmt + clippy
git switch -c mac-validation
```

把 `rustc -vV`、`sw_vers` 和 `uname -m`（应为 `arm64`）记进报告。

## 第 1 步：不需要数据的 CI

```sh
cargo xtask ci
```

它会依次跑 fmt、clippy `-D warnings`、`cargo test --workspace`、分层检查和对标清单。记录用时和结果。失败时先判断是不是平台问题；是的话修复，并在报告里写清根因。

## 第 2 步：放置数据

负责人会把一个压缩包 `dms-data-<commit>.tar.gz` 和它的 `.sha256` 文件拷到 Mac 上（U 盘或私人网盘），比如放在 `~/Downloads/`。包里有三部分：`corpus/`、`oracle/`（不含回归不用的 `probes/` 和 4.6 GB 的 `sources/excel_named_range`），以及 `modeler-home/Demos`（Modeler 自带的示例数据，ibm-demos 的流要用）。

```sh
cd ~/Downloads && shasum -a 256 -c dms-data-*.tar.gz.sha256
cd ~/dev/data-mining-software
tar -xzf ~/Downloads/dms-data-*.tar.gz corpus oracle
tar -xzf ~/Downloads/dms-data-*.tar.gz -C "$HOME" modeler-home
export DMS_MODELER_HOME="$HOME/modeler-home"   # 回归按文件名在 $DMS_MODELER_HOME/Demos 里找数据
cargo xtask corpus verify
```

如果 `.sha256` 文件里是 Windows 的路径格式导致 `-c` 失败，就用 `shasum -a 256 dms-data-*.tar.gz` 手动对比哈希值。

## 第 3 步：语料测试

```sh
cargo xtask test-corpus
```

它先校验语料，再运行各 crate `corpus` feature 下的测试，包括时间序列、Expert、C5.0 等冻结测试。记录失败的测试和断言信息。

## 第 4 步：全量回归，逐流对照 Windows 基线

```sh
labels=$(awk -F'\t' 'NR>1 && $1!~/^#/ {print $1}' xtask/corpus.tsv | sort -u \
  | grep -vx -e database -e probes | sed 's/^/--label /' | tr '\n' ' ')
cargo xtask regress $labels --jobs 4 --out target/mac-regress.tsv 2>&1 | tee target/mac-regress.log
python3 docs/mac/compare_baseline.py docs/mac/windows-baseline.tsv target/mac-regress.tsv \
  | tee target/mac-compare.txt
```

`docs/mac/windows-baseline.tsv` 是 Windows 上同一提交的逐流结果，基线数字见文末。预期有一处不同：`sources/excel_named_range` 的 oracle 没有拷过来，在 Mac 上会显示为 `absent`。

## 第 5 步：分析差异

对每个和基线不同的流：

1. 用 `cargo xtask compare <label> <stem>.str` 看具体哪一行、哪个字段不同，差多少（绝对值、相对值，以及大约差几个 ulp）。
2. 归类：
   - **平台问题**：路径、换行、文件名、编码、排序时对字符串的比较方式等。修复并重跑。
   - **浮点数差异**：同样的代码在 Mac 上算出最后几位不同。追到最先出现差异的那一步计算（哪个函数、哪个数学库调用），不要修，记下证据。
   - **其他**：如实记录。
3. 统计每一类有多少个流、涉及哪些算法。

## 交付

在分支 `mac-validation` 上提交：

- `docs/mac/MAC_REPORT.md`：环境、每一步的结果和用时、差异流清单及分类、浮点数差异的证据（最先分叉的计算）、平台修复的根因和改动、你建议的下一步（比如是否需要跨平台一致的数学函数实现）。
- `docs/mac/mac-regress.tsv`：从 `target/mac-regress.tsv` 复制。
- 必要的平台修复。

然后推送：`git push -u origin mac-validation`。负责人会在 Windows 上审查并合并。

## Windows 基线（提交 d1d2836）

见 `docs/mac/windows-baseline.tsv`。汇总行：

`xtask regress: 1597 streams: 1465 match, 91 statistical, 41 known GAPs, 0 regressions`（Windows 10，`--jobs` 默认）。数据包 `dms-data-d1d2836.tar.gz`，56.6 MB，SHA-256 `72089bc762d6ea550b41d2e86cdbfa6607c935d7da201d0d7acfb7b7a5bc8aa9`。
