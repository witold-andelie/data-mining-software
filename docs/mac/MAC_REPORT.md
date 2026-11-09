# Mac 验证报告（Apple Silicon，基于 8a6a046）

执行日期 2026-11-09。协调者：Claude Code（Opus 5.5）。第 1、3、4 步由 opencode 子代理（`opencode/glm-5.3-flash-free`）运行，其余步骤和全部核查由协调者完成。

每个结论所依据的原始输出摘录（错误原文、汇总行、逐流根因、子代理的工具调用记录和汇报原文、time_series 分叉点与 bisect）见 [`MAC_EVIDENCE.md`](MAC_EVIDENCE.md)。

## 结论

- **回归与 Windows 基线一致。** 修复两个平台问题后，Mac 上 1596 个流：1464 match、91 statistical、41 known GAPs、0 regressions。与 Windows 基线（1597 个流：1465 match、91 statistical、41 known GAPs）相比，唯一的差别是 `sources/excel_named_range`。它的 oracle 按约定没有打进数据包，在 Mac 上显示为 `absent`。两边都有的 1596 个流，分类和 `detail` 列（每个流匹配的终端输入数）逐行相同。
- **没有发现浮点数差异。** 第一轮的 13 个 regression 全部是同一个路径问题（Windows 绝对路径指向别的语料标签），修复后全部 match，没有任何流因数值不同而变化。
- **`cargo xtask ci` 在 Mac 上仍未全绿，但剩下的失败与平台无关。** fmt、clippy、layers、parity 都已通过。`cargo test --workspace` 只剩 1 个单元测试失败，它在 Windows 上同样会失败：自提交 `d2a93c5` 起测试期望与代码不同步，见下文“其他”。
- **`test-corpus` 的 4 个失败都不是平台问题：** 数据包不含 `oracle/probes/`（2 个）、Mac 上没有 Windows ODBC DSN（1 个），以及上面那个单元测试（1 个）。

## 环境

| 项 | 值 |
|---|---|
| 机器 | Apple M1，16 GB |
| 系统 | macOS 26.5.1（25F80），`uname -m` = `arm64` |
| Rust | rustc 1.95.0（59807616e 2026-04-14），host `aarch64-apple-darwin`，LLVM 22.1.2；cargo 1.95.0 |
| opencode | v2.2.6；子代理模型 `opencode/glm-5.3-flash-free`（无变体） |
| 其他 | Xcode Command Line Tools；unixODBC 2.3.14（Homebrew，本次新装）；Python 3.11.9 |

## 每一步的结果和用时

| 步骤 | 执行者 | 结果 | 用时 |
|---|---|---|---|
| 0 环境 | 协调者 | rustup 和 1.95.0 工具链已在本机，`rustup show` 直接通过；opencode v2.2.6 已安装且已登录；建分支 `mac-validation`。首次构建 xtask 链接失败：`ld: library 'odbc' not found`，`brew install unixodbc` 后解决（见“平台修复 1”）。 | `rustup show` 0 s，unixODBC 安装 4 s |
| 2 数据 | 协调者 | `shasum -a 256 -c` 直接认 Windows 生成的 `.sha256`，结果 OK（`72089bc7…8aa9`）；包内无绝对路径或 `..`；解压 `corpus/`、`oracle/` 到仓库，`modeler-home/` 到 `$HOME`；`cargo xtask corpus verify`：1717 files ok。 | 校验 1 s，解压 3 s，verify 4 s |
| 1 `ci`（第一轮） | 子代理 | exit=1：`cargo fmt` 无法解析 `crates/exec/tests/corpus/../support/mod.rs`（见“平台修复 2”） | 2 s |
| 3 `test-corpus`（第一轮） | 子代理 | exit=1：同一原因，`nodes_set_globals` 测试编译失败 | 23 s |
| 4 `regress`（第一轮） | 子代理 | exit=1：1596 streams，1451 match、91 statistical、41 known GAPs、13 regressions | 401 s |
| 4 `compare`（第一轮） | 子代理 | exit=1：13 个 match→regression，1 个 match→absent | 0 s |
| 4 `details`（第一轮） | 子代理 | 13 streams compared | 20 s |
| 子代理整体 | 子代理 | 正常退出，err.log 为空 | 519 s |
| `ci`（修复后） | 协调者 | exit=1：fmt、clippy 通过；`cargo test --workspace` 有 1 个失败（见“其他”）。补跑 `--no-fail-fast`：108 个测试二进制，573 通过、1 失败；`cargo xtask layers` 通过（13 packages ok）；`cargo xtask parity --check` 通过 | 142 s；补跑 158 s |
| `test-corpus`（修复后） | 协调者 | exit=1，在第一个失败的目标处停止。补跑 `--no-fail-fast`：675 通过，4 个测试失败（见“test-corpus 的失败”） | 125 s；补跑 588 s |
| `regress`（修复后） | 协调者 | **exit=0：1596 streams，1464 match、91 statistical、41 known GAPs、0 regressions** | 411 s |
| `compare`（修复后） | 协调者 | 只剩 `sources/excel_named_range: match -> absent`（预期） | 0 s |
| `details`（修复后） | 协调者 | 0 streams（无需比对） | 0 s |

`regress` 在 M1 上用 `--jobs 4` 只要约 7 分钟，没有出现文档预计的一小时。

## 子代理的汇报与核查

子代理只拿到 `opencode.subagent.json` 的权限：禁止编辑和网络，shell 只允许 `bash docs/mac/mac-steps.sh` 的五个子命令。它按顺序运行了五条命令，没有尝试其他 shell 命令，也没有写文件。每条命令都传了 `timeout: 0`，所以 `regress` 没有被工具超时打断。读文件用的是 opencode 的 read/grep 工具。

逐项核对（协调者直接读 `target/mac/` 下的日志和 TSV）：

| 子代理汇报的内容 | 核查结果 |
|---|---|
| 五行 `mac-steps …: exit=… elapsed=…` | 与各日志最后一行逐字一致 |
| `ci`、`test-corpus` 的首个错误 | 一致，确为 `nodes_set_globals.rs:586` 的 `mod support` |
| `regress` 汇总行（1451/91/41/13） | 一致（regress.log 第 378–379 行） |
| `mac-compare.txt` 全部 21 行 | 一致 |
| `details` 13 个流 | 一致；`details/` 覆盖了 `mac-compare.txt` 里全部 13 个非 absent 的差异流 |
| `target/mac/` 文件清单 | 一致 |

**结论：子代理的汇报与核查结果完全一致。** 没有出现虚报、改写错误或把失败说成可以忽略的情况。它的汇报只做原样复制，没有下结论，符合 `SUBAGENT_BRIEF.md` 的要求。

收尾：`mac-agent` 会话已用 `opencode session delete --standalone` 删除，`opencode session list --standalone` 为空；仓库根目录的 `opencode.json` 已删除。本机另有一个 `opencode serve --service`，12:41 由 launchd 启动，早于本次工作，不是本次启动的，未处理。所有 opencode 命令都带了 `--standalone`。

## 差异流清单及分类

第一轮共 14 个流与基线不同：

| 类别 | 流数 | 流 | 说明 |
|---|---|---|---|
| 平台问题（路径） | 13 | `xml_scan/` 下 11 个：`xml_default`、`xml_directory`、`xml_directory_recursive`、`xml_dtd`、`xml_fields_absolute`、`xml_fields_relative`、`xml_nested_records`、`xml_record_path`、`xml_repeated_elements`、`xml_text_whitespace`、`xml_xsd`；`pmml_import/` 下 2 个：`pmml_cart_regression`、`pmml_quest_classification` | 已修复，修复后全部 match。根因见“平台修复 3” |
| 预期的缺失 | 1 | `sources/excel_named_range` | oracle 没有打进数据包，Mac 上为 absent |
| 浮点数差异 | 0 | — | 见下节 |

涉及的算法：XML 源节点（`xmlimport` 扩展节点），以及从 PMML 导入的 CART 回归和 QUEST 分类模型的打分。这两个 PMML 流失败在上游读取 CSV 的源节点，和模型本身无关。

## 浮点数差异的证据

没有观察到浮点数差异：

- 修复后，两边都有的 1596 个流，分类全部相同；`detail` 列（如 `1/3 terminal inputs match Modeler (kohonen is held to statistical parity)`）也逐行相同，包括 91 个 statistical 流。
- 第一轮的 13 个差异全部是“找不到数据文件”，没有一个是数值不符。

这个结论有一个局限：`xtask compare` 判定数字相同的标准是“落在 oracle 打印小数位的舍入范围内，或相对误差 1e-9 以内”。比这更小的差异（例如 `exp`、`ln`、`powf` 最后一位不同，没有被迭代算法放大到超过容差）在回归里看不到。要确认 Mac 和 Windows 是否逐位一致，需要直接比较两边 `dms-cli run` 的原始输出（见“建议的下一步”）。

一个旁证：排查 time_series 单元测试时，`x = exp(z)` 再取 `ln(x)`，z 最大约 129，40 个值与 z 的最大误差为 0（macOS libm）。

## 平台修复

### 1. 环境：macOS 需要 unixODBC（文档）

`dms-io` 依赖 `odbc-api = 29.2.0`，它在非 Windows 平台链接 `libodbc`。新的 Mac 上没有这个库，`cargo xtask` 一开始就链接失败：`ld: library 'odbc' not found`。`brew install unixodbc` 后解决（链接命令里已有 `-L /opt/homebrew/lib`，不需要额外设置）。代码不用改，只在 `MAC_VALIDATION.md` 第 0 步补了一行 `brew install unixodbc`。

### 2. `crates/exec/tests/nodes_set_globals.rs`：内联模块里的 `#[path]`

根因：`#[cfg(feature = "corpus")] mod corpus { … #[path = "../support/mod.rs"] mod support; }`。内联模块里的 `#[path]` 以 `tests/corpus/` 为基准目录，于是得到 `tests/corpus/../support/mod.rs`。Windows 按字面消掉 `..`，所以能找到文件。POSIX 要求 `tests/corpus/` 真实存在才能经过 `..`，而这个目录并不存在。结果是 `cargo fmt --all --check` 失败（不论 feature 开不开），`corpus` feature 下这个测试也编译失败。

改动：把 `#[cfg(feature = "corpus")] mod support;` 挪到 crate 根，按常规解析到 `tests/support/mod.rs`，内联模块里改为 `use super::support;`。不再需要 `#[path]`，两个平台的行为相同。

### 3. `xtask/src/compare.rs`：跨标签的数据引用

根因：有些语料流用录制机器上的绝对路径引用另一个标签目录里的数据，例如 `xml_scan` 的流引用 `D:/data-mining-software/corpus/sources/source_records.xml`，`pmml_import` 的流引用 `corpus/cart_ofs2/ids200.csv` 和 `corpus/models_next/class_grid.csv`。录制机器上这个绝对路径存在，`dms-cli` 直接打开。在其他检出位置（macOS，或仓库不在 `D:\data-mining-software` 的 Windows 机器）只能按文件名回退，而 `compare` 只搜索本标签、`ibm-demo-data` 和 `$DMS_MODELER_HOME/Demos`。所以这些流是可移植性问题，并非只在 Mac 上出现。

改动：`compare_stream` 解码流文件，扫描文档 XML 以及 XmlText/Json 附件（扩展节点如 `xml_import` 的设置存在附件里，不在主文档中），找出 `…/corpus/<label>/…` 形式的引用（`/` 和 `\` 都识别）。对本地存在的 `corpus/<label>/` 目录，把它追加到 `--data-dir` 列表末尾。几点说明：

- 只是追加到列表末尾：本标签目录仍然最先搜索；绝对路径存在时 `dms-cli` 直接使用它，不会走到回退。所以 Windows 上的结果不变。
- 解码失败时什么也不加，由 `dms-cli run` 照常报告错误。
- 新增单元测试 `corpus_labels_in_saved_paths`，覆盖两种分隔符、去重，以及 `mycorpus`、`corpus/..`、非 ASCII 等不应匹配的情况。
- 效果：13 个流全部变回 match，全量 regress 为 0 regressions。

没有改动 oracle、`known-gaps.tsv` 或比较容差。

## 其他（不属于平台问题）

### time_series 单元测试：自 `d2a93c5` 起测试期望与代码不同步

`dms-exec` 的 `nodes::time_series::tests::transfer_seasonal_factors_transform_difference_and_delay` 失败：

```
logarithmic: Forecast { point: 2.496180285714286, … }, 0.43296428571428597
```

证据：

- 差值从第一个打分点 t=10 起就是 0.0632，之后按分母系数（普通项 0.2、季节项 0.3）几何衰减：t=20 为 1.4e-3，t=39 为 1.3e-6。这是递推初值不同的特征，不是舍入误差被放大。
- `ln(exp(z))` 在 Mac 上与 z 逐位相等，其余计算只有加减乘除，IEEE 754 保证跨平台一致。
- 在同一台 Mac 上用临时 worktree 分别测试：`d2a93c5^`（979f397）**通过**，`d2a93c5` **失败**。`d2a93c5`（"pre-sample numerator lags"）把分子滞后的预样本值改为第一个值，响应改为从 delay 处开始，但这个测试的独立递推仍是旧规则。

因此这个失败在 Windows 的 d1d2836 上应该同样存在，与平台无关。测试期望该怎么改（跟随新规则重写独立递推）需要负责人决定，本次没有改动。

### test-corpus 的失败（`--no-fail-fast` 补跑：675 通过，4 失败）

| 目标 | 测试 | 原因 | 类别 |
|---|---|---|---|
| `dms-exec --lib` | 上面的 time_series 单元测试 | 测试与代码不同步 | 其他 |
| `dms-clem --test oracle` | `clem_matches_modeler_on_probe_expressions` | 读 `oracle/probes/clem_semantics/manifest.json`，数据包不含 `oracle/probes/` | 数据包缺失 |
| `dms-io --test statistics_corpus` | `corpus_sav_files_match_modeler` | 读 `oracle/probes/sav_to_csv/*.csv`，同上，所以 `only 0 files checked` | 数据包缺失 |
| `dms-exec --test database` | `live::database_oracles` | 需要 Windows 的 Access ODBC DSN `dms_access`（SQLSTATE IM002） | Windows 专用环境 |

`mac-steps.sh test-corpus` 本身就会运行这些测试。注意 `cargo test` 默认在第一个失败的目标处停止，所以单看 `test-corpus.log` 会漏掉后面的失败，上表来自 `--no-fail-fast` 补跑。

## 建议的下一步

1. **暂时不需要跨平台一致的数学函数实现。** 全量回归在现有容差下与 Windows 完全一致，没有任何流因数值改变分类或匹配数。如果想排除容差以下的差异，最便宜的办法是在两台机器上对所有语料流跑 `dms-cli run`，逐字节比较输出的 CSV，只有发现分叉时再考虑 `libm` 之类的纯 Rust 实现。
2. 修 time_series 单元测试的期望值，让 `cargo xtask ci` 在两个平台都变绿（需要负责人确认新规则下的独立推导）。
3. 让 `test-corpus` 在 Mac 上可判定：要么把 `oracle/probes/clem_semantics` 和 `oracle/probes/sav_to_csv` 加进数据包，要么让这两个测试和 `database` 的 live 测试在数据或 DSN 缺失时明确跳过（目前一个 panic，一个断言 `count >= 30`）。
4. 合并本分支后，在 Windows 上重跑一次 `cargo xtask regress`，确认 compare 的改动对 Windows 结果零影响（按设计应为零）。

## 提交内容

- `crates/exec/tests/nodes_set_globals.rs`：平台修复 2
- `xtask/src/compare.rs`：平台修复 3 及单元测试
- `docs/mac/MAC_VALIDATION.md`：第 0 步补 `brew install unixodbc`
- `docs/mac/mac-regress.tsv`：修复后的回归结果，前 4 列（`cut -f1-4`）
- `docs/mac/MAC_REPORT.md`：本报告

未提交：`corpus/`、`oracle/`、`~/modeler-home/`、`target/mac/` 和 `target/mac-run1/`（第一轮的原始日志）。
