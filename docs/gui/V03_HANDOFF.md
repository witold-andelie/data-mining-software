# v0.3 交接：能设置（给 Codex gpt-6.1-sol 和 opencode）

> 协调者：Mac 上的 Claude Code。分支 `mac-app`，起点为 v0.2 之后的提交（`8e4a0a8` 及其后的交接提交）。
> 依据：`docs/gui/MAC_APP_PLAN.md`（v0.3 = 改节点参数；英文、中文翻译表）、`docs/gui/DESIGN.md` §5.3、`AGENTS.md`、`docs/gui/V02_HANDOFF.md` §2（共同规则，本文照样适用）。

| WP | 执行者 | 内容 | 层 | 依赖 |
|---|---|---|---|---|
| WP4 | Codex | 设置模式（15 个节点）、设置读写、可逆的 `SetSettings` 编辑、引擎命令 `node.set_param`、上游字段列表 | L1、L3、L4 | — |
| WP5 | Codex | 按设置模式生成属性表单 | L6、app | WP4 |
| WP6 | Codex | 翻译框架、设置窗口（语言） | L6、app | WP5 |
| WP7 | opencode | 中文翻译表的条目 | 数据文件 | WP6 |

每个 WP 一次运行；协调者审查、跑关口、整合并提交后再开始下一个。

---

## 1. 共同规则

`docs/gui/V02_HANDOFF.md` §2 的八条全部适用（只改列出的文件、不提交不联网、不新增依赖、Never crash、不碰 corpus/oracle/known-gaps/容差、不改截图基准、净室、汇报格式）。补充：

- 本版不新增任何外部依赖。
- 节点名称（`catalog.rs` 的标签）不翻译，保持英文，便于对照文档。
- 参数的**标签和说明是我们自己写的英文**，不照抄 IBM 的界面文案。参数的稳定键（`key`）用文档里的脚本属性名（公开的 API 名称）。

## 2. 已定的设计决定（不要改）

- 设置模式放在 **L3 `dms-exec`**，紧挨读取这些设置的节点代码，避免两处定义漂移。它只描述**执行器实际读取的参数**：执行器不读的设置项不进表单（改了也没用）。
- 设置编辑整体替换一个节点的设置元素：L1 新增 `Edit::SetSettings`，逆操作恢复原来的元素。一次参数修改 = 一个撤销步骤。
- 写入时只改动该参数对应的元素，其余元素（包括我们不认识的）原样保留，位置不变；元素不存在时追加在 `properties` 末尾。
- 写入的文本编码与 Modeler 保存的一致（见 `docs/formats/nodes/*.md` 的 encoding 列）：布尔 `true`/`false`，整数十进制，实数写最短往返文本（Rust 的 `{}` 格式即可；整数值的实数写成 `2.0`），必须能被 `settings::real` 读回同值，选择项写文件值（如 `include`）。
- **参数有两种来源**（`ParamSource`）：`Properties`（`path` 是 `properties` 下的元素路径）和 `Attachment`（`path` 是节点 `properties_attachment` 所指 XML 附件里 `<Parameter name=…>` 的名字；文档里写作 `attachment:<名字>`）。CHAID、C&R Tree、Linear 等扩展节点的算法参数、目标和输入都在附件里，执行器（`crates/exec/src/nodes/build.rs` 的 `Parameters`）从附件读取。**执行器代码不改**：表单改附件，跟 Modeler 保存的位置一致。
- 写附件参数只改那一个 `Parameter` 的 `value` 属性（列表参数改其 `Value` 子元素），附件其余字节原样不变；该 `Parameter` 不存在时，在已有 `Parameter` 的父元素末尾追加一个。节点没有附件、附件不是合法 XML、或附件被别的节点共用时，写入返回错误（表单把这些参数显示为只读并给出原因），不自动新建附件。
- 一次参数修改可能同时产生 `SetSettings` 和 `PutAttachment`；两者放在同一个撤销步骤里。
- 字段列表来自**上一次运行**的结果：节点的输入表的字段。没有运行过时，表单退化为自由文本并提示先运行。v0.3 不做“不运行就推断字段”。

## 3. 覆盖的 15 个节点

| 目录 id | 文档 | 至少要覆盖的参数（以执行器实际读取为准，可多不可少） |
|---|---|---|
| `var_file` | `variablefile.md` | 文件路径、是否读字段名、分隔符、字段名所在行 / 跳过行、引号处理 |
| `statistics_file` | `statisticsimport.md` | 文件路径、读取值标签/变量标签的方式 |
| `type` | `type.md` | 表格：每个字段的测量级别、角色（direction）；缺失值处理（如执行器读取） |
| `filter` | `filter.md` | 表格：每个字段 包含/排除、新名字；默认模式 |
| `derive` | `derive.md` | 新字段名、派生方式（公式/标志/计数等，执行器支持的）、公式（CLEM）、对应方式的参数 |
| `select` | `select.md` | 模式（包含/丢弃）、条件（CLEM） |
| `partition` | `partition.md` | 字段名、分区数（2/3）、各分区比例、随机种子、是否可重复 |
| `chaid` | `chaid.md` | 目标、输入（如未用 Type 角色）、最大深度、显著性水平、最小记录数 |
| `cart` | `cart.md` | 目标、输入、最大深度、剪枝、最小记录数 |
| `logistic` | `logreg.md` | 目标、输入、方法（Enter/逐步等，执行器支持的）、是否含常数项 |
| `linear` | `linear.md` | 目标、输入、是否含截距、方法 |
| `table` | `table.md` | 执行器读取的输出选项（若没有，模式为空也可） |
| `analysis` | `analysis.md` | 执行器读取的选项（重合矩阵、性能评估、置信度等） |
| `histogram` | `histogram.md` | 字段、叠加字段、箱数或箱宽 |
| `data_audit` | `dataaudit.md` | 字段（全部/指定）、是否显示统计和图表等执行器读取的选项 |

---

## WP4：设置模式和设置编辑（Codex）

### 4.1 文件

只能改：`crates/stream/src/edit.rs`（新增 `Edit::SetSettings` 及其校验、逆操作、测试）；`crates/exec/src/lib.rs`（只加 `pub mod schema;` 和 `pub use` 行）；新文件 `crates/exec/src/schema.rs`（可拆成 `crates/exec/src/schema/` 目录下多个文件）；新文件 `crates/exec/tests/schema_corpus.rs`（`corpus` 特性）；`crates/engine/src/session.rs`、`crates/engine/src/command.rs`、`crates/engine/src/edit_session.rs`（新增命令和查询）；`crates/ui/src/workbench.rs` 只在编译需要时为新的 `Arg`/`Outcome` 加最小分支（应当不需要）。

### 4.2 接口

```rust
// crates/stream/src/edit.rs
Edit::SetSettings { uid: String, settings: Option<Element> }   // Element = dms_strfmt::fastinfoset::Element; top level only; inverse restores

// crates/exec/src/schema.rs
pub struct ParamSpec {
    pub key: &'static str,                // scripting property name from the docs, stable
    pub source: ParamSource,              // Properties or Attachment (see §2)
    pub path: &'static str,               // Properties: element path under `properties`, '/'-separated;
                                          // Attachment: the `Parameter` name
    pub label: &'static str,              // our English label
    pub help: &'static str,               // one sentence, ours
    pub group: &'static str,              // section heading in the form ("Fields", "Options", ...)
    pub kind: ParamKind,
    pub default: Option<&'static str>,    // file text when the element is absent
    /// Shown only when another parameter has one of these file values.
    pub visible_if: Option<(&'static str, &'static [&'static str])>,
}
pub enum ParamKind {
    Bool,
    Int { min: i64, max: i64 },
    Real { min: f64, max: f64 },
    Text,
    Expr,                                              // CLEM
    Choice(&'static [(&'static str, &'static str)]),   // (file value, our label)
    Field { measures: &'static [&'static str] },       // empty = any
    Fields { measures: &'static [&'static str] },
    FilePath { extensions: &'static [&'static str] },
    Table { row_path: &'static str, key_child: &'static str, columns: &'static [ColumnSpec] },
}
pub struct ColumnSpec { pub child: &'static str, pub label: &'static str, pub kind: ParamKind }
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue { Unset, Bool(bool), Int(i64), Real(f64), Text(String), Fields(Vec<String>),
                      Rows(Vec<(String, Vec<Option<String>>)>) }   // (key, column file values)

pub enum ParamSource { Properties, Attachment }

pub fn schema(catalog_id: &str) -> Option<&'static [ParamSpec]>;
// Attachment parameters: read from / rewrite the attachment's bytes (only that Parameter changes).
pub fn read_attachment(attachment: Option<&[u8]>, spec: &ParamSpec) -> Result<ParamValue, String>;
pub fn write_attachment(attachment: &[u8], spec: &ParamSpec, value: &ParamValue) -> Result<Vec<u8>, String>;
pub fn read(settings: Option<&Element>, spec: &ParamSpec) -> Result<ParamValue, String>;
/// The settings element after setting `spec` to `value` (a new `properties` element when
/// `settings` is None). Only that parameter's elements change.
pub fn write(settings: Option<&Element>, spec: &ParamSpec, value: &ParamValue) -> Result<Element, String>;
pub fn validate(spec: &ParamSpec, value: &ParamValue) -> Result<(), String>;

// crates/engine
Arg::Param(String /* key */, ParamValue)
"node.set_param"  // args: Node(uid), Param(key, value); one undo step; label "Change <Param label>"
pub struct FieldInfo { pub name: String, pub storage: String, pub measure: String }
impl Session {
    pub fn node_schema(&self, uid: &str) -> Option<(&'static str /*catalog id*/, &'static [ParamSpec])>;
    pub fn node_settings(&self, uid: &str) -> Option<&Element>;
    /// Fields of the data reaching `uid` in the latest run (its inputs' outputs), if any.
    pub fn input_fields(&self, uid: &str) -> Option<Vec<FieldInfo>>;
    /// The parameter's current value from either source (settings or the node's attachment).
    pub fn param_value(&self, uid: &str, key: &str) -> Result<ParamValue, String>;
    /// Why the parameter cannot be changed (e.g. the node has no attachment), if it cannot.
    pub fn param_read_only(&self, uid: &str, key: &str) -> Option<String>;
}
```

上限：一个设置元素最多 100 000 个子元素；表格最多 10 000 行；文本参数最多 1 MiB。

### 4.3 验收

```sh
export CARGO_BUILD_JOBS=3 CARGO_NET_OFFLINE=true DMS_MODELER_HOME=$HOME/modeler-home
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p dms-stream -p dms-exec -p dms-engine
cargo test -p dms-exec --features corpus --test schema_corpus -- --nocapture
cargo xtask regress --label team --label misc --label sources --out target/schema-subset.tsv
```

`schema_corpus`：对 `corpus/` 里这 15 种节点的**每个实例**：用 `read` / `read_attachment` 读出全部参数（不得报错，读不懂的值算失败并列出）；再把读出的值用 `write` / `write_attachment` 写回，设置元素在 `dms_stream::settings_xml::normalize` 意义下与原来完全相同，附件逐字节相同。打印每种节点的实例数和参数数。另外每种节点至少一个单元测试：改一个参数后 `read` 读回新值，且执行器读取该设置的函数得到新值（例如 Select 改条件后，过滤结果随之改变；CHAID 改最大深度后建出的树随之改变）。引擎测试：附件参数的一次修改是一个撤销步骤，撤销后附件逐字节恢复。

协调者另跑全量 `regress`，必须与 `docs/mac/mac-regress.tsv` 逐行相同。

---

## WP5：属性表单（Codex）

### 5.1 文件

只能改：新文件 `crates/ui/src/properties.rs`；`crates/ui/src/workbench.rs`（属性面板改为调用 `properties.rs`，以及为文件选择新增 `Pick::DataFile`）；`crates/ui/src/lib.rs`（加 `pub mod`）；`apps/dms/src/main.rs`（`Pick::DataFile` 的原生对话框）；新测试 `crates/ui/tests/properties.rs`（不截图）。不改引擎；接口不够时停下说明。

### 5.2 表单规则

- 按 `group` 分节，节标题加粗；`visible_if` 不满足的参数隐藏；每个控件旁有 `help` 的悬停说明。
- `Bool` → 勾选框；`Choice` → 下拉框（显示我们的标签）；`Int`/`Real` → 数字输入框，超出范围或无法解析时控件变红、给出原因、不提交；`Text` → 单行文本框；`Expr` → 等宽多行文本框，失去焦点或 ⌘↩ 提交，提交前用 `dms_clem` 解析，语法错误在框下显示错误位置和原因、不提交；`Field`/`Fields` → 有上游字段时用下拉/多选列表（按 `measures` 过滤），没有时用文本框加提示“Run the stream to list the fields reaching this node.”；`FilePath` → 文本框加“Choose…”按钮（`Pick::DataFile`，按扩展名过滤，macOS 不过滤，见 `288e211`）；`Table` → 每行一个键（字段名），各列是下拉或文本，用表格显示，可滚动。
- 提交时机：勾选、下拉、表格单元格改变时立即提交；文本和数字在回车或失去焦点时提交；每次提交发一条 `node.set_param`（一个撤销步骤）。撤销后表单显示撤销后的值。
- 不在模式里的节点：显示只读的设置 XML（可折叠，等宽字体，最多显示 2 000 行），并说明“Settings for this node type cannot be edited yet.”。
- 导入后被保留、但我们不认识的元素：在表单底部列出名字（只读），说明会原样保存。

### 5.3 验收

```sh
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test -p dms-ui --lib --test properties --test editing --test connect_gestures
```

`properties.rs` 测试（合成流，用真实指针和键盘事件）：Select 的模式下拉改为“Discard”后设置元素改变且可撤销；条件里输入非法 CLEM 时显示错误且设置不变；Partition 的整数超出范围时不提交；Type 表格改一个字段的测量级别；没有模式的节点显示只读 XML。

---

## WP6：翻译框架和设置窗口（Codex）

### 6.1 文件

只能改：新文件 `crates/ui/src/i18n.rs`、`crates/ui/i18n/zh-Hans.toml`（只建文件和全部键，值留空字符串）；`crates/ui/src/*.rs` 里的界面文字（改为经过 `tr`）；新文件 `crates/ui/src/settings_window.rs`；`apps/dms/src/main.rs`、`apps/dms/src/macos_menu.rs`（菜单标签经过 `tr`，语言改变后重建菜单）；`crates/macos-open/src/lib.rs` 只能加一个**不含 unsafe** 的函数 `preferred_language() -> Option<String>`（用 objc2-foundation 的 `NSLocale::preferredLanguages`，如需特性只改该 crate 的 `Cargo.toml` 特性列表）；`crates/engine/src/command.rs` 让 `app.settings` 可用（`Requires::Nothing`）。

### 6.2 规则

- `pub fn tr(source: &'static str) -> &'static str`：英文源文本即键；当前语言为中文且表里有非空译文时返回译文，否则返回原文。带参数的文字用 `tr_fmt("{count} nodes failed", &[("count", ...)])` 一类的简单占位替换，不引入依赖。
- 语言：`System`（默认）、`English`、`简体中文`。存进 `ShellState`（`#[serde(default)]`）。System 时用 `preferred_language()`，以 `zh` 开头即中文，否则英文；Windows 暂时总是英文。
- 设置窗口（⌘, / 应用菜单“Settings…”）：语言下拉框；数据文件夹列表（显示、删除单个、“Add…”）。改语言立即生效。
- 命令标签、菜单名、按钮、提示、对话框、状态栏消息都要经过 `tr`。节点名称、字段名、用户数据、引擎和执行器返回的错误原文不翻译。
- 测试：扫描 `crates/ui/src` 和 `apps/dms/src` 的源代码，找出所有 `tr("…")` / `tr_fmt("…"` 的字面量，`zh-Hans.toml` 必须恰好包含这些键（不多不少）；`tr` 在未翻译时返回原文；切换语言后菜单和工具栏文字改变（kittest）。

---

## WP7：中文翻译（opencode）

### 7.1 文件

只能改 `crates/ui/i18n/zh-Hans.toml` 的**值**（键一个不改、不增不删）。

### 7.2 术语（必须一致）

Stream 流；Node 节点；Run 运行；Stop 停止；Results 结果；Properties 属性；Settings 设置；Field 字段；Target 目标；Input 输入；Partition 分区；Model / nugget 模型 / 模型块；Expression 表达式；Data Folder 数据文件夹；Undo 撤销；Redo 重做；Save 保存；Save As 另存为；Open 打开；Close 关闭；Connect 连接；Select 选中（菜单动作）；Measurement 测量级别；Role 角色；Storage 存储类型；Missing values 缺失值。占位符 `{…}` 原样保留。节点名称不翻译。

### 7.3 验收

```sh
cargo test -p dms-ui --lib i18n
```

---

## 3.4 交接指令（协调者执行）

与 `V02_HANDOFF.md` §7 相同：每个 WP 一个 `git clone --local` 副本、`corpus`/`oracle` 符号链接、任务说明 = 本文 §1–§3 + 对应 WP 一节，Codex 用 `codex exec -m gpt-6.1-sol -c model_reasoning_effort="xhigh" -s workspace-write …`，opencode 用 `opencode run --standalone --auto -m 'opencode/glm-5.3-flash-free' …` 并在副本根目录放只允许 `cargo test -p dms-ui*` 和 `cargo fmt*` 的权限配置。WP4 之后在独立副本里对提交跑全量 `regress`；WP5 之后协调者生成并审查截图、打包试用。
