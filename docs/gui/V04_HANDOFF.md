# v0.4 交接：打磨（给 Codex gpt-6.1-sol）

> 协调者：Mac 上的 Claude Code。分支 `mac-app`，起点为 v0.3 之后的交接提交。
> 依据：`docs/gui/MAC_APP_PLAN.md`（v0.4 = 执行器取消、图表、画布的两个已知问题、性能预算）、`docs/gui/DESIGN.md` §1 已知问题表、§2 状态栏、§6 取消、§7 性能、`AGENTS.md`、`docs/gui/V02_HANDOFF.md` §2（共同规则八条，本文照样适用）。

| WP | 内容 | 层 | 依赖 | 可并行 |
|---|---|---|---|---|
| WP8 | 执行器取消：`CancelToken`、节点之间和耗时算法里检查、引擎的“正在停止”状态和部分结果 | L0、L2、L3、L4 | — | 与 WP9 并行 |
| WP9 | 图表：图表节点的结果用 `egui_plot` 画出来，数据表作为第二个视图 | L6 | — | 与 WP8 并行 |
| WP10 | 画布和收尾：连线出入方向、标签背景、模型块注明来源、停止后的界面、只供程序调用的命令不进菜单、`xtask perf` | L4 命令表、L6、xtask | WP8、WP9 | — |

WP8 和 WP9 的文件不重叠，同时在两个副本里运行。两者都整合后再开始 WP10。

---

## 1. 共同规则（补充 V02_HANDOFF.md §2）

- 只能用已登记的依赖。`egui_plot` 0.38.0（MIT OR Apache-2.0）已由协调者加进工作区和 `crates/ui/Cargo.toml`，并登记在 DESIGN.md §9.1。不新增其他依赖。
- 界面文字一律经过 `dms_ui::i18n::tr`/`tr_fmt`（v0.3）。新增的键要加进 `crates/ui/i18n/zh-Hans.toml`（按字母序，值填好简体中文，术语见 `docs/gui/V03_HANDOFF.md` §7.2）；`crates/ui/tests/i18n.rs` 会检查键集合。
- 不改截图基准；沙箱里没有 GPU，截图测试报 `No adapter found` 是预期的。截图由协调者生成和审查。
- 改到 `crates/exec`、`crates/algo` 时，协调者会对提交跑全量回归（不含 database 标签），必须与 `docs/mac/mac-regress.tsv` 逐行相同：**取消检查不能改变任何计算结果**。

---

## WP8：执行器取消（Codex）

### 8.1 现状

`run.cancel` 现在只把引擎状态改回空闲、丢弃这次运行的结果；工作线程不知道被取消，会把整个流跑完（`crates/engine/src/run.rs` 只在终端节点批次之间检查 `JobContext::is_cancelled`）。长时间的训练（神经网络、Time Series 专家建模、C5.0 提升、随机树、TwoStep、K-Means、Kohonen、Apriori、CHAID/C&R 树、逻辑回归和 GLM 的迭代等）停不下来。

### 8.2 文件

只能改：`crates/types/src/`（新增 `cancel.rs` 和 `lib.rs` 里的导出）；`crates/exec/src/{context.rs,error.rs,plan.rs,lib.rs}`；`crates/exec/src/nodes/*.rs` 里**调用耗时算法的地方**（把取消标记传下去）；`crates/algo/src/**/*.rs` 里**耗时的循环**（加检查和参数）；`crates/engine/src/{run.rs,job.rs,session.rs}`；对应的测试文件；`apps/dms-cli/src/main.rs` 和 `xtask/src/` 只在 `ExecContext` 的构造需要补字段时改（用 `Default` 一类的写法，行为不变）。不改 `crates/ui`、`apps/dms`，**例外**：`crates/ui/src/results.rs` 的 `error_text` 只加一个分支 `ErrorKind::Cancelled => tr("Stopped before this node ran.").to_string()`，`crates/ui/i18n/zh-Hans.toml` 只按字母序加这一个键（`"Stopped before this node ran." = "在运行到此节点前已停止。"`）；其余界面处理归 WP10。WP9 同时在改 `results.rs`，协调者整合时合并。

### 8.3 设计

```rust
// crates/types/src/cancel.rs  (L0, so dms-algo can use it)
#[derive(Clone, Default, Debug)]
pub struct CancelToken(Arc<AtomicBool>);
impl CancelToken {
    pub fn new() -> Self;
    pub fn cancel(&self);
    pub fn is_cancelled(&self) -> bool;            // Relaxed load, cheap enough for inner loops
    pub fn check(&self) -> Result<(), Cancelled>;  // Err when cancelled
}
pub struct Cancelled;  // Display: "cancelled"
```

- `ExecContext` 新增 `pub cancel: CancelToken`（默认永不取消）。`OperatorError` 新增 `Cancelled`（不算根因：它不是节点出错）。
- `Execution::evaluate_observed` 在**每个节点开始前**检查；已取消时，未开始的节点结果都是 `Cancelled`，已完成的节点结果保留。
- 耗时算法在迭代循环里检查：每轮迭代、每棵树、每个提升轮次、每个候选模型，或者每处理约 1 万条记录检查一次（取最自然的位置）。检查只读一个原子变量，**不改变任何计算顺序和数值**。算法函数的签名可以加 `cancel: &CancelToken` 参数；被 Auto 节点、集成节点间接调用的地方一并传下去。
- 引擎：`JobContext` 的取消和 `ExecContext::cancel` 是同一个标记（取消作业时同时取消执行器）。`run.cancel` 后：
  - `RunState` 新增 `Stopping { job }`：界面显示“正在停止”，**不再接受新的运行**；工作线程在下一次检查时返回。
  - 线程返回后，`RunResults` 保留已完成节点的结果，`RunReport` 新增 `cancelled: bool`；未运行的目标节点结果是 `Cancelled`，不进 `root_causes`。状态回到 `Idle`。
  - 如果 2 秒内线程还没有返回（某个算法没有检查点），退回现在的做法：放弃这次运行的结果、状态回到 `Idle`、线程在后台跑完后丢弃结果；`RunReport` 不出现。用 `session.poll_jobs()` 推进，不阻塞界面线程。
- `Outcome`、命令 id、快捷键不变（⌘. 已绑定 `run.cancel`）。

### 8.4 验收

```sh
export CARGO_BUILD_JOBS=3 CARGO_NET_OFFLINE=true DMS_MODELER_HOME=$HOME/modeler-home
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p dms-types -p dms-algo -p dms-exec -p dms-engine
cargo test -p dms-exec --features corpus --test cancel_corpus -- --nocapture
cargo xtask regress --label team --label models --label models_ml --label ts_more --label c50_boost --out target/cancel-subset.tsv
```

新测试 `crates/exec/tests/cancel_corpus.rs`（`corpus` 特性）：先用 `dms-cli` 或计时找出 corpus 里单次运行最慢的 10 个流（排除 database 标签；打印各自耗时）；对每一个，在运行开始后 50 ms、200 ms、以及耗时一半时取消，测量从取消到 `evaluate` 返回的时间，**全部不超过 250 ms**（release 下；debug 下允许 1 s，测试按构建模式选上限），并打印最大值。另有单元测试：取消前已完成的节点结果保留；`Cancelled` 不进根因；从未取消的运行结果与之前完全相同（同一个流跑两次，一次带从不触发的标记，一次不带）。引擎测试：`Stopping` 状态下 `run.stream` 不可用；线程不返回时 2 秒后退回空闲。

子集回归必须与 `docs/mac/mac-regress.tsv` 中这些标签的行逐行相同。协调者另跑全量回归。

---

## WP9：图表（Codex）

### 9.1 现状

图表节点（Histogram、Distribution、Plot、Multiplot、Time Plot、Collection、Evaluation、Web、Graphboard、E-Plot）的结果是一个 `Document`，里面一张表，是 Modeler 的 VIZML 导出里图背后的数据（`crates/exec/src/nodes/graph_data.rs`；各节点文件开头的文档写了表头和行的含义）。界面现在把这张表当普通文档表格显示。

### 9.2 文件

只能改：新文件 `crates/ui/src/chart.rs`（可拆成 `crates/ui/src/chart/` 目录）；`crates/ui/src/results.rs`（图表节点的结果标签页先显示图，旁边有 “Chart / Data” 切换，Data 是现在的表格）；`crates/ui/src/lib.rs`（加 `pub mod`）；`crates/ui/i18n/zh-Hans.toml`（新键）；新测试 `crates/ui/tests/chart.rs`（不截图）；`crates/ui/src/workbench.rs` **只改调用 `results::show_results` 的地方**，把结果所属节点的设置（`StreamNode::settings`，从 `session.document()` 的流里按 uid 找）传进去，`results::show_results` 的签名可以相应加参数。不改 `crates/exec`、`crates/engine`；接口不够时停下说明。

### 9.3 设计

- 纯函数部分（可测）：`pub fn chart_for(node_type: &str, settings: Option<&Element>, document: &Document) -> Option<Chart>`，从节点类型（目录 id，如 `histogram`）、节点设置（读法用 `dms_exec::settings` 的 `text`/`flag`/`children` 等，键名见 `docs/formats/nodes/` 和各图表节点源文件开头的说明）和结果表构造与绘图库无关的 `Chart { kind, title, x_label, y_label, series: Vec<Series> }`。读不懂的表返回 `None`，界面退回只显示表格（不报错、不崩溃）。数字用 `str::parse::<f64>`，`$null$`、空单元格、`NaN` 跳过。
- 覆盖：Histogram（柱状，叠加字段时按值分组堆叠或并列）、Distribution（按类别的柱状，有叠加时分组）、Plot（散点；`plot` 设置为线时画线）、Multiplot 和 Time Plot（多条线）、Evaluation（增益/提升/响应等曲线，每个模型一条，含基线）、Collection（按类别的柱状）。Web、Graphboard、E-Plot 本版只显示表格。设置不足以确定画法时（例如 Evaluation 的曲线类型、Multiplot 的面板），按执行器读取的同一设置判断；仍不能确定的情况返回 `None` 退回表格，并在汇报里列出。
- 绘制用 `egui_plot`：坐标轴标签、图例（多于一个序列时）、悬停显示数值；颜色用我们主题里的一组序列色（在 `crates/ui/src/theme.rs` 现有颜色里选，或在 `chart.rs` 里定义一组新的；不用 IBM 的配色），浅色和深色主题都要清楚。
- 上限：每个序列最多画 100 000 个点，超过时均匀抽样并在图下注明“显示 N / M 个点”；类别最多 500 个，超过的合并为 “Other”。
- 图表视图记住每个标签页选的是 Chart 还是 Data（不持久化）。

### 9.4 验收

```sh
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test -p dms-ui --lib --test chart --test results --test i18n
cargo test -p dms-ui --test chart --features corpus -- --nocapture   # 若加了 corpus 特性；没有就跳过
```

`chart.rs` 测试：每种覆盖的图至少一个合成 `Document` 的单元测试（序列数、点数、坐标正确）；畸形表（缺列、全是文字、空表、超大表）返回 `None` 或被截断，不 panic。若 `dms-ui` 没有 `corpus` 特性，可以在测试里按 `DMS_CHART_CORPUS=1` 环境变量（默认跳过）跑 corpus 里所有图表节点：运行流、对每个图表节点的结果调用 `chart_for`，打印每种节点“画得出 / 退回表格”的数量。

---

## WP10：画布和收尾（Codex，WP8、WP9 整合后）

### 10.1 内容

1. **连线出入方向**（DESIGN.md §1 已知问题）：连线从源图块和目标图块边缘的四个方向（右、左、下、上）中，选让两端最近的一对出入；箭头仍指向目标。竖直排列的节点连线从下边出、上边入，不再穿过标签。输出端口圆圈（v0.2 的连线手势）保持在右边不变。
2. **标签背景**：节点标签画一个与画布背景同色、略透明的圆角底，画在连线之上，连线经过时不和文字重叠。选中和运行标记不受影响。
3. **模型块注明来源**：选中模型块时，属性面板显示 “Generated by <建模节点名>”（按 `model_builder` 找到同一流里对应的建模节点；找不到就显示建模节点类型）。点名字选中该建模节点。
4. **停止后的界面**（接 WP8）：`Stopping` 时状态栏显示“正在停止…”，Stop 按钮不可用；停止后状态栏显示 “Stopped after N s”，已完成节点照常有结果标签页和运行标记，未运行的节点没有标记、不算失败。
5. **只供程序调用的命令不进菜单**：`node.set_param`、`node.add`、`node.move`、`node.connect`、`node.disconnect`、`edit.select` 这类需要参数、只由界面手势或程序调用的命令，在 `CommandSpec` 上标记为不进菜单（例如 `Menu` 加一个 `Hidden`，或单独一个字段），原生菜单和窗口内菜单都不显示；命令面板和快捷键不受影响。
6. **`cargo xtask perf`**：运行 `crates/ui/tests/perf.rs`（release）并输出 200/1000 个节点的 CPU 单帧平均和 p95，以及 `dms --bench` 不可用时的说明；与 DESIGN.md §7 的预算比较（200 节点 p95 ≤ 4 ms 视为通过），超出时退出码非 0。不跑真实窗口（沙箱和 CI 都没有显示器）。

### 10.2 文件

只能改：`crates/ui/src/{canvas.rs,graph.rs,edit_canvas.rs,workbench.rs,menu.rs,results.rs,theme.rs}`；`crates/ui/i18n/zh-Hans.toml`；`crates/engine/src/command.rs`（菜单标记）；`apps/dms/src/macos_menu.rs`（跳过隐藏命令）；`xtask/src/main.rs` 和新文件 `xtask/src/perf.rs`；`crates/ui/tests/{editing.rs,connect_gestures.rs,perf.rs}` 和新测试 `crates/ui/tests/canvas_links.rs`（不截图）。

### 10.3 验收

```sh
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test -p dms-ui --lib --test canvas_links --test editing --test connect_gestures --test i18n --test results
cargo test -p dms-engine -p dms -p xtask
cargo xtask perf
cargo check --target x86_64-pc-windows-msvc -p dms -p dms-ui -p dms-engine --all-targets
```

`canvas_links.rs`：竖直排列的两个节点之间的连线端点在下边/上边；水平排列在右/左；连线手势（从端口拖、Option 拖、Connect From Here）仍能连上；隐藏命令不出现在 `menu_items` 里但 `execute` 仍可用。

---

## 交接指令（协调者执行）

与 `V02_HANDOFF.md` §7 相同：每个 WP 一个 `git clone --local` 副本（`~/dev/agents/v04-wpN`），`corpus`/`oracle` 符号链接；任务说明 = 本文 §1 + 对应 WP 一节 + 汇报格式；Codex 用 `codex exec -m gpt-6.1-sol -c model_reasoning_effort="xhigh" -s workspace-write …`。WP8 整合后在独立副本里对提交跑全量回归（直接和 `--via-dms`，不含 database 标签）；WP9、WP10 整合后协调者生成并审查截图（图表浅色和深色、连线方向），打包试用。版本号在 v0.4 结束时升到 0.4.0（协调者做）。
