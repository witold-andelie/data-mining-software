# v0.2 交接：能编辑（给 Codex gpt-6.1-sol）

> 协调者：Mac 上的 Claude Code。执行者：Codex（`gpt-6.1-sol`）。分支 `mac-app`，起点为提交 `30d2444` 之后的交接提交。
> 依据：`docs/gui/MAC_APP_PLAN.md`（v0.2 = 修改流并保存）、`docs/gui/DESIGN.md` §5–§6、`AGENTS.md`。本文与 DESIGN.md 不一致处，以本文为准（§3 列出了改动）。

v0.2 拆成三个**按顺序**执行的工作包（WP）。每个 WP 由 Codex 在自己的仓库副本里单独完成一次运行。协调者审查、跑验证关口、整合并提交后，才开始下一个 WP。

| WP | 内容 | 层 | 依赖 |
|---|---|---|---|
| WP1 | 可编辑文档 `StreamDoc`、可逆编辑 `Edit`、原生格式 `.dms`（TOML）、原子保存、`.str` 导入往返、`dms-cli convert`、`regress --via-dms` | L1、CLI、xtask | — |
| WP2 | Session：编辑命令、选中、撤销/重做、复制粘贴文本、保存/另存/新建/关闭确认、自动保存和恢复 | L4 | WP1 |
| WP3 | 界面：画布编辑（移动、连线、删除、框选、从节点面板添加、重命名）、保存对话框、未保存确认、退出拦截、恢复提示、剪贴板、`.dms` 文档类型 | L6、app、bundle | WP2 |

---

## 1. 背景（每个 WP 都先读）

`data-mining-software` 是用 Rust 重写的 IBM SPSS Modeler 19（单机功能）。执行器 `dms-exec` 已经能运行导入的 `.str` 流，全量回归与 Modeler 一致。桌面应用 v0.1 已经能打开 `.str`、在后台运行、显示结果、打包成 `.app`。v0.2 让用户能**修改流并保存**。

必读：`AGENTS.md`（全部规则有效，尤其 Never crash 和净室）、`docs/gui/MAC_APP_PLAN.md`、`docs/gui/DESIGN.md` §5–§6、本文。代码入口：`crates/stream/src/lib.rs`（`Stream`、`Diagram`、`StreamNode`、`Link`、`NodeRef`）、`crates/strfmt/src/fastinfoset/{mod.rs,xml.rs}`（`Element`、`write_element`）、`crates/engine/src/{session.rs,command.rs,run.rs,job.rs}`、`crates/ui/src/{workbench.rs,canvas.rs,graph.rs,menu.rs,results.rs}`、`apps/dms/src/{main.rs,macos_menu.rs}`、`apps/dms-cli/src/main.rs`、`xtask/src/{compare.rs,regress.rs,bundle.rs}`。

## 2. 所有 WP 共同的规则

1. **只改本 WP“文件”一节列出的文件**。需要改别的文件时停下，在汇报里说明原因，不要自己改。
2. 不提交、不推送、不切换分支、不联网、不安装软件。环境变量 `CARGO_NET_OFFLINE=true`；本文用到的依赖都已在本地缓存里。**不新增任何外部依赖**，只能用：`toml`（=1.2.1，已登记）、`roxmltree`（0.20，已在 `dms-stream` 依赖里）、`serde`、`serde_json`、`arboard`（3.7.0，仅 WP3 的 `apps/dms`）。base64 自己实现（几十行，带测试）。
3. 非测试代码禁止 `unwrap`/`expect`/`panic!`/`unreachable!`/`todo!`/`unimplemented!`/`unsafe`，以及对外部数据用 `[]` 下标；工作区 lints 已强制。来自文件和剪贴板的数据都视为不可信：长度、数量、嵌套深度都设上限（见各 WP）；递归要有深度上限，或改用显式栈。
4. 不改 `corpus/`、`oracle/`（只读符号链接）、`xtask/known-gaps.tsv`、任何比较容差、`docs/mac/`。不删除或放宽现有测试。如果现有测试的期望值因为本 WP 的**设计变更**必须改，在汇报里逐条说明。
5. 截图基准图（`crates/ui/tests/snapshots/*.png`）一律不改、不新增。Codex 的沙箱里没有 GPU，渲染截图的测试会报 `No adapter found`，这是预期的；在汇报里注明即可。截图由协调者生成和审查。
6. 界面文字用英文源字符串（v0.3 再加翻译表）。不使用 IBM 的图标、文案、配色（净室）。
7. 每个 WP 结束前都必须跑通该 WP 的验收命令，并把**原样输出的最后几行和退出码**放进汇报。
8. 汇报（最后一条消息）格式：① 改了哪些文件；② 验收命令的原样输出和退出码；③ 本文没有规定、由你自己决定的事；④ 没做完的部分。

## 3. 已定的设计决定（不要改）

相对 DESIGN.md §5–§6 的调整：

- **可编辑文档以现有的 `dms_stream::Stream` 为核心**，不另造一套节点结构。`StreamDoc` 只是在 `Stream` 外面包上附件和元数据。执行器继续直接读 `&Stream`，所以运行路径一行不改。
- 节点设置继续是原始设置树（`Option<Element>`）。v0.2 不做按设置模式生成的表单（那是 v0.3），只做结构编辑：增删节点、连线、移动、重命名、复制粘贴。新添加的节点没有设置，运行时会报设置错误，这在 v0.2 是预期行为。
- 只编辑顶层画布。超节点内部只读：编辑操作遇到超节点内部的 uid 时返回错误。
- 原生格式是 TOML，扩展名 `.dms`。设置树以 XML 文本保存在 TOML 的多行字面字符串里；遇到 XML 无法精确表示的情况，改存结构化形式（见 WP1 §4.3）。
- `.str` 只导入、从不写出。导入的文档在首次保存时必须另存为 `.dms`。
- 节点 uid：导入的保留原样；新建的用 `"id"` 加 11 位大写字母和数字，在文档内保证唯一。生成器可注入，测试里结果确定。

---

## WP1：L1 可编辑文档和原生格式

### 4.1 目标

1. `dms-stream` 新增 `StreamDoc`、`Edit` 及可逆的 `apply`；设置树和 XML 文本之间无损互转；`.dms` 的读写和原子保存；`.str` → `StreamDoc` 导入。
2. `dms-cli convert <in.str> <out.dms>`；`dms-cli run` 同时接受 `.dms`。
3. `cargo xtask regress --via-dms`：每个流先转换成 `.dms` 再读回运行，结果必须与直接运行 `.str` 逐流相同。这是 WP1 的核心验收。

### 4.2 文件

只能改：`crates/stream/src/lib.rs`（只加 `pub mod` 和 `pub use` 行）；新文件 `crates/stream/src/doc.rs`、`crates/stream/src/edit.rs`、`crates/stream/src/native.rs`、`crates/stream/src/settings_xml.rs`、`crates/stream/src/base64.rs`；`crates/stream/Cargo.toml`（只能调整已登记依赖的特性）；新文件 `crates/stream/tests/native_roundtrip.rs`（放在 `corpus` 特性后面）；`apps/dms-cli/src/main.rs`（新增 `convert`，`run` 支持 `.dms`）；`xtask/src/compare.rs`、`xtask/src/regress.rs`、`xtask/src/main.rs`（只改用法说明文字）（新增 `--via-dms`）。

### 4.3 接口（照此实现，可以加私有辅助函数）

```rust
// crates/stream/src/doc.rs
pub struct StreamDoc {
    pub stream: Stream,
    /// ZIP members of an imported `.str` (model PMML ...), by member name.
    pub attachments: BTreeMap<String, Arc<[u8]>>,
    pub meta: DocMeta,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocMeta {
    /// File name of the `.str` this document was imported from, if any.
    pub imported_from: Option<String>,
    /// Keys of a newer format version that this version does not know, written back unchanged.
    pub extra: BTreeMap<String, toml::Value>,
}
impl StreamDoc {
    pub fn new() -> Self;                                  // empty stream, no attachments
    pub fn import_str(bytes: &[u8], file_name: &str) -> Result<StreamDoc, DocError>;
    pub fn node(&self, uid: &str) -> Option<&StreamNode>;  // top level only
}
pub enum DocError { Read(String), Format(String), TooLarge(String), Version(u32) }  // + Display

// crates/stream/src/edit.rs
pub enum Edit {
    AddNode { node: StreamNode, index: usize },             // index into diagram.nodes
    RemoveNode { uid: String },                             // fails while links touch it
    Connect { link: Link, index: usize },
    Disconnect { link: Link },
    Move { uids: Vec<String>, dx: i64, dy: i64 },
    Rename { uid: String, label: Option<String> },
    SetTerminalOrder { order: Vec<NodeRef> },
    PutAttachment { name: String, bytes: Option<Arc<[u8]>> }, // None removes
}
pub enum EditError { UnknownNode(String), DuplicateUid(String), StillLinked(String),
                     NotTopLevel(String), WouldCycle { from: String, to: String },
                     SelfLink(String), Duplicate, OutOfRange(String), Limit(&'static str) }
impl StreamDoc {
    /// Validates, applies, and returns the edit that undoes it. On error the document is unchanged.
    pub fn apply(&mut self, edit: Edit) -> Result<Edit, EditError>;
    /// Applies all edits or none; returns the inverses in the order to apply them for undo.
    pub fn apply_all(&mut self, edits: Vec<Edit>) -> Result<Vec<Edit>, EditError>;
}
pub fn new_uid(existing: &dyn Fn(&str) -> bool, rng: &mut dyn FnMut() -> u64) -> String;
```

规则：
- 连线两端都必须是顶层节点；不能自连；不能重复；连线不能成环（只看顶层有向图）。
- `Move` 之后坐标限制在 ±1 000 000 以内，超出返回 `OutOfRange`。
- 上限：节点数 100 000、连线数 400 000、标签长度 1 000 个字符、附件单个 256 MiB、附件总计 1 GiB。
- `apply` 的逆操作再 `apply` 一次，文档必须恢复成**完全相同**的值，包括节点在列表里的位置。用属性测试来验证：在合成流上对随机编辑序列做“做→撤”往返。随机数用确定的种子，不新增依赖。

```rust
// crates/stream/src/settings_xml.rs
/// XML text of an element (re-export of strfmt's writer, plus parsing back).
pub fn element_to_xml(element: &Element) -> String;
pub fn element_from_xml(text: &str) -> Result<Element, String>;   // roxmltree, depth limit 512
/// Whether the element survives element_to_xml → element_from_xml unchanged, after merging
/// adjacent text chunks (the only normalization allowed).
pub fn xml_roundtrips(element: &Element) -> bool;
pub fn normalize(element: &Element) -> Element;                    // merges adjacent Text nodes
```

### 4.4 `.dms` 格式（照此实现）

```toml
format = "dms-stream"
version = 1
imported_from = "model_comparison.str"        # optional
file_version = "…"                            # Stream.file_version, optional
save_version = "…"                            # optional
stream_properties = '''<properties>…</properties>'''   # optional; XML text
terminal_order = ["id…", "id…"]               # NodeRef::Uid; unresolved ones as "?7"

[[node]]
uid = "id228WNW9FCUR"
class = "com.spss.clementine.nodes.…"         # Stream's class string, unchanged
extension = { id = "spss.xml_data", object = "xml_import" }   # or { id, element }; optional
name = "…"                                    # optional
label = "…"                                   # optional
position = [270, 427]                         # optional
model_builder = "…"                           # optional
properties_attachment = "data/0001.dat"       # optional
containers = [{ name = "…", member = "data/0002.dat" }]       # optional
settings = '''<properties>…</properties>'''   # optional; XML text when xml_roundtrips
settings_tree = { … }                         # instead of `settings` when it does not: the
                                              # element as nested tables (name, attributes, children)
binning_output = '''…'''                      # optional, same rule
result_items = ['''…''']                      # optional, same rule
[node.subdiagram]                             # supernodes: nodes, links, terminal_order, recursive
…

[[link]]
from = "id…"                                  # "?7" for NodeRef::Unresolved("7")
to = "id…"

[[attachment]]
name = "data/0002.dat"
text = '''…'''                                # when valid UTF-8 without NUL; else:
base64 = "…"
```

- **兼容**：所有字段 `#[serde(default)]`；不认识的顶层键和节点键放进 `extra`，保存时原样写回。`version` 大于 1 时返回 `DocError::Version`。
- **确定性**：同一个 `StreamDoc` 每次写出的字节完全相同（有测试）。
- **上限**：读入前先查文件大小，上限 1 GiB；超节点嵌套最多 64 层（与 `Stream` 一致）。
- **原子保存**：`pub fn save_atomic(doc: &StreamDoc, path: &Path) -> Result<(), DocError>`。在同一目录写 `.<name>.dms.tmp-<pid>-<n>`，`sync_all`，再 `rename` 覆盖目标；任何一步失败都删掉临时文件，原文件不受影响。读取用 `pub fn load(path: &Path) -> Result<StreamDoc, DocError>`，以及 `to_toml(&StreamDoc) -> Result<String, DocError>` 和 `from_toml(&str) -> Result<StreamDoc, DocError>`。

### 4.5 CLI 和 xtask

- `dms-cli convert <in.str> <out.dms>`：导入后原子保存。`dms-cli run <file>`：按扩展名 `.dms` 或 `.str` 选择读取方式，其余行为不变。
- `cargo xtask regress … --via-dms`：对每个流，先用 `dms-cli convert` 写到 `<target>/via-dms/<label>/<stem>.dms`，再对这个 `.dms` 运行 `compare`。其余逻辑和输出格式不变。不带 `--via-dms` 时行为完全不变。

### 4.6 验收

```sh
export CARGO_BUILD_JOBS=3 CARGO_NET_OFFLINE=true DMS_MODELER_HOME=$HOME/modeler-home
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p dms-stream -p dms-cli -p xtask
cargo test -p dms-stream --features corpus --test native_roundtrip -- --nocapture
cargo xtask regress --label team --label misc --label sources --label xml_scan --label pmml_import --via-dms --out target/via-dms-subset.tsv
```

`native_roundtrip`（corpus 特性）：对 `xtask/corpus.tsv` 登记的**每个** `.str`，执行导入 → `to_toml` → `from_toml`，要求得到的 `Stream` 与直接解析的结果在 `normalize` 意义下相等（节点、连线、设置树、附件逐一相同），并且第二次 `to_toml` 与第一次逐字节相同。打印三个数：文件总数、走 XML 文本的设置数、走 `settings_tree` 的设置数。不打印任何文件内容。

协调者另外会跑全量 `regress --via-dms`，要求 `cut -f1-4` 的结果与 `docs/mac/mac-regress.tsv` 逐行相同。只要有任何一个流不同，WP1 就不能通过。

---

## WP2：L4 编辑会话

### 5.1 目标

Session 能新建、打开（`.str` 和 `.dms`）、编辑、撤销/重做、保存/另存、在关闭前确认，以及自动保存和恢复。所有操作都是带 id 的命令，程序调用从不弹对话框。

### 5.2 文件

只能改：`crates/engine/src/session.rs`、`crates/engine/src/command.rs`、`crates/engine/src/run.rs`（只为改从 `StreamDoc` 取流）、`crates/engine/src/lib.rs`；新文件 `crates/engine/src/edit_session.rs`、`crates/engine/src/undo.rs`、`crates/engine/src/clipboard.rs`、`crates/engine/src/autosave.rs`；`crates/ui/src/menu.rs`（只改 `egui_key` 里新 `Key` 的分支）、`apps/dms/src/macos_menu.rs`（只改 `accelerator` 里新 `Key` 的分支）；`crates/ui/src/workbench.rs` 里 `finish` 函数对 `Outcome` 的 `match`：只为新增的 `Outcome` 变体各加一个最小分支——`ConfirmDiscard` 在状态栏显示错误消息“There are unsaved changes.”，`Clipboard(text)` 在有 `ctx` 时调用 `ctx.copy_text(text)`（真正的确认对话框和系统剪贴板由 WP3 做），该文件的其他部分一律不动；以及为了让编译通过而必须做的、只涉及调用签名的一行改动（在汇报里逐一列出）。

### 5.3 命令（加入或改为可用；id 不可改）

| id | 标签 | macOS | 其他 | 参数 | 说明 |
|---|---|---|---|---|---|
| `file.new` | New Stream | ⌘N | Ctrl+N | — | 有未保存修改时返回 `ConfirmDiscard`（见下） |
| `file.open` | Open… | ⌘O | Ctrl+O | `Path` | `.str` 导入（未保存、无 native 路径），`.dms` 打开 |
| `file.save` | Save | ⌘S | Ctrl+S | （无 native 路径时 `NeedsArgument("save_path")`） | 原子保存；清除已修改标记；删除自动保存 |
| `file.save_as` | Save As… | ⇧⌘S | Ctrl+Shift+S | `Path` | 路径不以 `.dms` 结尾时自动加上 |
| `file.close` | Close | ⌘W | Ctrl+W | `Force`（可选） | 已修改且无 `Force` 时返回 `ConfirmDiscard` |
| `app.quit` | Quit | ⌘Q | — | `Force`（可选） | 同上，然后 `Outcome::Quit` |
| `edit.undo` / `edit.redo` | Undo / Redo | ⌘Z / ⇧⌘Z | Ctrl+Z / Ctrl+Shift+Z | — | 标签带上操作名：“Undo Move” |
| `edit.cut` / `edit.copy` / `edit.paste` | Cut / Copy / Paste | ⌘X / ⌘C / ⌘V | Ctrl+X/C/V | 粘贴需要 `Text`，可选 `Position` | 复制返回 `Outcome::Clipboard(String)` |
| `edit.duplicate` | Duplicate | ⌘D | Ctrl+D | — | 原地复制并偏移 (+30, +30) |
| `edit.delete` | Delete | ⌫ | Delete | — | 删除选中的节点及其连线（一个事务） |
| `edit.select_all` | Select All | ⌘A | Ctrl+A | — | 顶层全部节点 |
| `edit.select` | （界面用） | — | — | `Nodes(Vec<String>)` | 设置选中项，不进撤销栈 |
| `node.add` | Add Node | — | — | `Kind(String)`，`Position(i64, i64)` | `Kind` 是目录 id；已实现的节点才能添加；新节点选中 |
| `node.move` | Move | — | — | `Nodes`、`Delta(i64, i64)` | 一次拖动一个事务 |
| `node.connect` / `node.disconnect` | Connect / Disconnect | — | — | `Link(String, String)` | |
| `node.rename` | Rename… | ↩︎ | F2 | `Node`、`Text` | 空字符串表示清除标签 |
| `file.recover` | （界面用） | — | — | `Path` | 打开一个自动保存，作为未保存文档 |

- 新增 `Arg` 变体：`Force`、`Text(String)`、`Kind(String)`、`Position(i64, i64)`、`Delta(i64, i64)`、`Nodes(Vec<String>)`、`Link(String, String)`；新增 `Outcome` 变体：`ConfirmDiscard`、`Clipboard(String)`。新增 `Key`：`X`、`C`、`V`、`D`、`A`、`Backspace`、`Delete`、`Enter`、`F2`，并在 `menu.rs` 的 `egui_key` 和 `macos_menu.rs` 的 `accelerator` 里补上对应分支。
- 添加节点：`class` 用目录里该节点的第一个 Modeler 标识。普通类名直接用；`a!b` 形式的扩展标识拆成 `Extension::Object { object_id: a, extension_id: b }`，`class` 留空。
- 命令表测试必须继续通过：两个平台各自没有快捷键冲突；macOS 标准快捷键测试按上表补充。

### 5.4 文档、撤销、已修改

```rust
pub struct DocumentState {
    pub doc: StreamDoc,
    pub native_path: Option<PathBuf>,      // `.dms` once saved or opened
    pub imported_from: Option<PathBuf>,    // `.str` it came from
    pub selection: Vec<String>,            // uids, top level
    revision: u64, saved_revision: Option<u64>,
    undo: UndoStack,
}
impl Session {
    pub fn document_state(&self) -> Option<&DocumentState>;
    pub fn is_dirty(&self) -> bool;
    pub fn revision(&self) -> u64;
    pub fn undo_label(&self) -> Option<&str>;  // "Move", "Delete", …
    pub fn redo_label(&self) -> Option<&str>;
}
```

- 现有的 `Document { path, stream, attachments }` 和 `Session::document()` 保留，供 v0.1 的界面和运行使用（可以改成由 `DocumentState` 派生，路径取 `native_path` 或 `imported_from`）。运行取 `doc.stream` 和 `doc.attachments`。
- 撤销栈：每个命令一个事务（`Vec<Edit>` 的逆操作）；上限 500 个事务；新的编辑清空重做栈；撤销到保存时的 revision 时，已修改标记消失。
- 运行结果记录它对应的 revision；`Session::results_stale() -> bool` 在文档已改动时返回 true（界面会提示）。

### 5.5 剪贴板文本

`clipboard.rs`：复制得到 `format = "dms-clipboard"`、`version = 1` 的 TOML 文本，节点和连线的写法与 `.dms` 相同，只包含选中的节点、它们之间的连线，以及它们引用的附件。粘贴时：解析（输入上限 16 MiB，节点 10 000 个）、重新分配 uid、按映射改写连线；有 `Position` 时把这组节点的左上角放到该位置，否则整体偏移 (+30, +30)；作为一个事务应用，粘贴出的节点成为选中项。文本不是 `dms-clipboard` 格式时返回错误，不改动文档。

### 5.6 自动保存和恢复

```rust
impl Session {
    pub fn set_autosave_dir(&mut self, dir: Option<PathBuf>);
    /// Writes the document there when it is dirty and `now - last_autosave >= 30 s`. Call every
    /// frame; cheap when nothing is due. Errors are returned for the status line, never panics.
    pub fn autosave_if_due(&mut self, now: std::time::Instant) -> Result<bool, String>;
    /// Autosaves left by an earlier session that crashed or was killed, newest first.
    pub fn recoverable(&self) -> Vec<Recovery>;
}
pub struct Recovery { pub path: PathBuf, pub original: Option<PathBuf>, pub saved_at: std::time::SystemTime }
```

自动保存文件名为 `<文档 id>.dms`，文档 id 在打开或新建时生成；元数据里记录原始路径。保存或正常关闭时删除对应的自动保存。目录里最多保留 20 个文件，最旧的先删。

### 5.7 验收

```sh
export CARGO_BUILD_JOBS=3 CARGO_NET_OFFLINE=true
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p dms-engine -p dms-stream
cargo test -p dms-ui --lib                      # 不渲染的测试
```

必须新增的测试：表中每个命令至少一个；做/撤/重做后文档与原值逐字节相同（比较 `to_toml`）；`ConfirmDiscard` 与 `Force` 的流程；复制→粘贴的 uid 重映射和连线改写；恶意剪贴板文本（超大、深嵌套、非 UTF-8 替换字符、格式错误）返回错误且文档不变；自动保存的到期判断、恢复列表和清理；导入 `.str` 后 `file.save` 返回 `NeedsArgument("save_path")`。

---

## WP3：界面编辑

### 6.1 目标

在画布上编辑流，保存成 `.dms`，并在关闭和退出前确认；崩溃后能恢复。

### 6.2 文件

只能改：`crates/ui/src/{workbench.rs,canvas.rs,graph.rs,menu.rs,results.rs}`；新文件 `crates/ui/src/{edit_canvas.rs,dialogs.rs}`；新测试 `crates/ui/tests/editing.rs`（不做截图）；`apps/dms/src/main.rs`、`apps/dms/Cargo.toml`（只加 `arboard.workspace = true`，并在根 `Cargo.toml` 的 `[workspace.dependencies]` 加 `arboard = "=3.7.0"` 这一行）；`xtask/src/bundle.rs`（`.dms` 文档类型）。**不要改** `crates/ui/src/theme.rs`、`crates/ui/src/icon.rs`、`crates/ui/src/node_icons.rs` 和 `crates/ui/tests/node_icons.rs`：节点图标由协调者负责。`canvas.rs` 里画节点图块和模型块样式的部分（`draw_node`）保持现有外观，只能加交互状态（拖动预览、连线端口）。

### 6.3 交互（照此实现）

- **选中**：单击节点选中；⇧单击加入或取出选中；在空白处拖动出现矩形框，框住的节点被选中（⇧ 时追加）；单击空白处清空选中。选中通过 `edit.select` 同步到 Session。两指平移、捏合缩放保持不变：拖动背景现在用来框选，平移改为两指滚动或按住空格拖动。
- **移动**：拖动选中的节点。拖动过程中只在界面里预览偏移，松开时发送一次 `node.move`（一个撤销步骤）。
- **连线**：节点图块右边缘中点有一个小圆形端口（悬停时才显示）。从端口拖到另一个节点上松开，发送 `node.connect`；不合法时在指针旁显示原因（成环、自连、重复），不发送。单击连线选中它（高亮），⌫ 或 Delete 发送 `node.disconnect`。
- **删除、复制、粘贴、复制一份、全选**：走 `edit.*` 命令。macOS 上，原生菜单的 ⌘C/⌘V/⌘X/⌘A 在**文本框有焦点时**（`ctx.wants_keyboard_input()`）要变成文本操作：给 egui 注入 `Event::Copy`、`Event::Cut`、`Event::Paste(text)`、全选；其余时候是节点操作。读写系统剪贴板用 `arboard`（只在 `apps/dms` 里），界面库通过一个 `Clipboard` trait 拿到它，测试里用假实现。
- **单键快捷键**：⌫、Delete、↩︎、F2、Esc 这类不带 ⌘/Ctrl 的快捷键，不要注册成 macOS 原生菜单的 accelerator（菜单项只显示按键文字），否则会截走文本框里的退格和回车。改由界面在画布有焦点、且 `ctx.wants_keyboard_input()` 为 false 时处理。`apps/dms/src/macos_menu.rs` 的 `accelerator` 为没有 `command` 修饰的快捷键返回 `None`；`dms_ui::menu::take_shortcuts` 在有原生菜单时仍处理这些单键。
- **撤销菜单的标签**：用 WP2 的 `Session::command_label(id)`（例如“Undo Move”），原生菜单和窗口内菜单都要同步。
- **从节点面板添加**：可以把已实现的节点从面板拖到画布，松开处就是位置；或者双击，放在可见区域中心。发送 `node.add`。未实现的节点不能拖，悬停时说明原因（现有行为）。
- **重命名**：双击节点标签，或选中节点后按 ↩︎ / F2，在原处出现文本框；↩︎ 确认（发送 `node.rename`），Esc 取消。
- **右键菜单**：在节点上：运行到此、重命名、复制、删除；在连线上：删除；在空白处：粘贴、全选。
- **画布图**：`StreamGraph` 只在 `Session::revision()` 变化时重建，不要每帧重建。
- **保存**：`NeedsArgument("save_path")` 时弹出 `rfd` 保存对话框（过滤 `.dms`，默认文件名取 `.str` 的主文件名）。窗口标题为 `<名字> — Data Mining`，已修改时为 `<名字> — Edited — Data Mining`。
- **确认对话框**（`dialogs.rs`，egui 模态窗口）：收到 `ConfirmDiscard` 时显示“Do you want to save the changes you made to <名字>?”和三个按钮 **Save**、**Don't Save**、**Cancel**，按 macOS 惯例，Cancel 响应 Esc，Save 是默认按钮（响应 ↩︎）。Save → 保存，成功后再执行原命令并带上 `Force`；Don't Save → 直接带 `Force` 重新执行原命令。
- **退出拦截**：窗口收到关闭请求（`viewport().close_requested()`）而文档已修改时，发送 `ViewportCommand::CancelClose`，然后走上面的确认流程。⌘Q 走 `app.quit`。
- **自动保存**：应用把自动保存目录设为 `~/Library/Application Support/Data Mining/Autosave`（Windows：`%APPDATA%\Data Mining\Autosave`，用环境变量拼，不新增依赖），每帧调用 `autosave_if_due`。启动时如果 `recoverable()` 不为空，显示一个恢复窗口，列出文件名和时间，提供“Recover”（`file.recover`）和“Discard”（删除该文件）。
- **结果过期**：`results_stale()` 为 true 时，结果区顶部显示“The stream changed since this run.”和 Run 按钮。
- **文档类型**：`bundle.rs` 的 Info.plist 增加 `.dms`：导出 UTI `io.github.witold-andelie.dms.stream`（符合 `public.data`），角色 `Editor`，`LSHandlerRank` 为 `Owner`；原来的 `.str` 条目保持不变。

### 6.4 验收

```sh
export CARGO_BUILD_JOBS=3 CARGO_NET_OFFLINE=true
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p dms-ui --lib --test editing --test document_view
cargo test -p dms-engine -p dms-stream -p xtask
cargo check --target x86_64-pc-windows-msvc -p dms -p dms-ui -p dms-engine --all-targets   # 若沙箱里没有该 target，注明即可
```

`crates/ui/tests/editing.rs` 用 `egui_kittest`，只用 `harness.run()` 和查询、点击、拖动，不调用 snapshot。至少覆盖：从面板双击添加节点；拖动移动后一次撤销复原；从端口拖动连线，以及成环时被拒绝并显示原因；框选两个节点后删除，再撤销；复制粘贴（假剪贴板）；重命名（双击、输入、↩︎）；已修改时关闭弹出确认，三个按钮各自的结果；在文本框里按 ⌘C 不触发节点复制。

---

## 7. 交接指令（协调者执行）

每个 WP 一次运行。前一个 WP 整合并提交后，再开下一个。

```sh
# 准备（以 WP1 为例；WP2/WP3 把名字换成 v02-wp2 / v02-wp3）
cd ~/dev/agents && git clone --local ~/dev/data-mining-software v02-wp1
cd v02-wp1 && git switch mac-app
ln -s ~/dev/data-mining-software/corpus corpus && ln -s ~/dev/data-mining-software/oracle oracle
printf 'corpus\noracle\n' >> .git/info/exclude
mkdir -p ~/dev/agents/v02-wp1.out

# 任务说明 = 本文的 §1、§2、§3 和对应 WP 一节（协调者拼成 ~/dev/agents/briefs/v02-wp1.md）
source ~/.cargo/env
export CARGO_BUILD_JOBS=3 CARGO_NET_OFFLINE=true DMS_MODELER_HOME=$HOME/modeler-home
codex exec -m gpt-6.1-sol -c model_reasoning_effort="xhigh" \
  -c 'sandbox_workspace_write.writable_roots=["/Users/mac/.cargo"]' \
  -s workspace-write -C ~/dev/agents/v02-wp1 --json \
  -o ~/dev/agents/v02-wp1.out/last-message.md - \
  < ~/dev/agents/briefs/v02-wp1.md > ~/dev/agents/v02-wp1.out/events.jsonl \
  2> ~/dev/agents/v02-wp1.out/stderr.log
```

协调者在每个 WP 之后要做的事：

1. `git -C ~/dev/agents/v02-wpN status --porcelain`，确认只改了允许的文件，再审查 diff。
2. 在主仓库应用改动，跑 `cargo xtask ci`。
3. WP1：在独立副本里对提交后的状态跑全量 `regress` 和 `regress --via-dms`，两者都必须与 `docs/mac/mac-regress.tsv` 逐行相同。
4. WP3：生成截图基准并人工审查；打包 `.app`，用 `corpus/team/` 的流实际走一遍：打开、改动、保存成 `.dms`、关闭、重新打开、运行。
5. 提交信息注明“implemented by Codex gpt-6.1-sol, reviewed by the coordinator”。

遇到 `usage limit`：停下，告诉负责人，未完成的改动留在副本里，不换模型。
