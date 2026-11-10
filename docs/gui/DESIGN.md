# P4 界面设计（阶段 A）

> 状态：阶段 A（设计 + 技术验证）完成，等待负责人审查 · 分支 `gui-design` · 2026-11-09
> 依据：`docs/gui/GUI_BRIEF.md`（硬约束）、`AGENTS.md`、`docs/architecture.md`

本文回答三个问题：界面长什么样、怎么用（§2–§3）；界面之下的 L1/L4 要补什么、接口是什么（§4–§6）；怎么保证质量，风险和待决定的事（§7–§11）。§1 是技术验证的结果，后面的设计都以它为依据。

## 目录

1. [技术验证（spike）的结果](#1-技术验证spike的结果)
2. [主窗口布局](#2-主窗口布局)
3. [主要交互流程](#3-主要交互流程)
4. [命令清单](#4-命令清单)
5. [L1：编辑模型和原生保存格式](#5-l1编辑模型和原生保存格式)
6. [L4：命令注册表、Session、撤销、长任务](#6-l4命令注册表session撤销长任务)
7. [界面状态的序列化和截图测试](#7-界面状态的序列化和截图测试)
8. [Mac 体验的落实方式](#8-mac-体验的落实方式)
9. [依赖清单和许可证](#9-依赖清单和许可证)
10. [风险](#10-风险)
11. [待负责人决定的问题和建议](#11-待负责人决定的问题和建议)
12. [阶段 B 起的实施顺序](#12-阶段-b-起的实施顺序)

## 1. 技术验证（spike）的结果

目的：验证 eframe + wgpu/Metal、节点画布、原生菜单、离屏截图测试这一套组合在 Apple M1 上可行。代码在本分支，阶段 B 可以重写。

| 项 | 结果 |
|---|---|
| 新 crate | `dms-ui`（L6，`crates/ui`）、`dms`（apps，`apps/dms`）；`dms-engine` 新增 `command`、`session` 两个模块（L4，无界面依赖）。都已在 `xtask/src/layers.rs` 登记，`cargo xtask layers` 通过（15 packages ok） |
| 打开 `.str` | `dms corpus/team/model_comparison.str`、菜单“Open…”（原生文件对话框）、或把文件拖进窗口，都会走 `file.open` 命令。节点按 `.str` 里保存的坐标摆放；目录里没有的节点和未实现的节点保留并画成虚线框，状态栏统计“N not supported yet” |
| 画布 | 自己在 egui painter 上写（§1.1）。触控板双指平移、捏合缩放（以指针为中心）、鼠标拖背景平移、⌘ + 滚轮缩放、⌘= / ⌘- / ⌘0 缩放到合适；单击选中节点，属性面板显示类型和状态 |
| 原生菜单 | macOS 用 `muda` 生成屏幕顶部的菜单栏，菜单项和快捷键都来自 `dms_engine::command::COMMANDS`；每帧按 `Session::is_enabled` 同步可用状态。应用菜单含“服务、隐藏、隐藏其他、全部显示”等系统项，Window/Help 菜单交给系统管理。Windows 上 `muda` 的初始化函数是 `unsafe fn`，工作区禁止 `unsafe`，所以由同一张命令表生成窗口内菜单（`dms_ui::menu::menu_bar`） |
| 真实流 | `corpus/team/model_comparison.str`（24 个节点、23 条连线）和 `cluster_analysis.str`（24 个节点、24 条连线）：全部节点都已识别，没有悬空连线，布局与保存的坐标一致。离屏渲染工具 `DMS_RENDER_STREAM=<file> cargo test -p dms-ui --test render_file -- --ignored` 只写到 `target/render/`，团队数据和它的图片都不提交 |
| 截图测试 | `crates/ui/tests/snapshots.rs`（egui_kittest + wgpu，离屏）：浅色主窗口（窗口内菜单，即 Windows 布局）、深色主题下放大并选中节点的画布，以及一个空窗口交互测试。只用合成流（`dms_ui::synthetic`，标签含捷克语变音字符）。基准图在 `crates/ui/tests/snapshots/` |
| 帧率（M1，release，1280×800 窗口，Retina） | 200 个节点同时在屏幕上、标签全部可见、持续平移：开启垂直同步时稳定 **60 fps**（显示器刷新率）；关闭时 **176 fps**；每帧 CPU 平均 1.86 ms，p95 2.70 ms。1000 个节点：177 fps。命令：`dms --bench 200 [--vsync]` |
| CPU 单帧开销（离屏，不含 GPU） | 200 个节点、标签可见：平均 0.71 ms，p95 0.97 ms（`cargo test --release -p dms-ui --test perf -- --ignored --nocapture`） |
| 启动 | 从进程启动到第一帧 150–170 ms（冷启动一次为 412 ms），目标是不超过 2 s |
| 中文、捷克语 | 应用开启 eframe 的系统字体回退（默认），缺字时按需加载系统字体；截图测试里的捷克语（ž、ř、ě、≥）显示正确 |
| 浅色/深色 | 跟随系统（egui 默认 `ThemePreference::System`），两套节点配色都已定义 |
| 界面状态 | `ShellState`（面板开合、平移、缩放、选中项）用 serde 存进 eframe 的存储（RON），读回后先 `sanitize()` 再用；窗口大小和位置由 eframe 保存 |
| Windows 编译 | 在 Mac 上用 `cargo check --target x86_64-pc-windows-msvc -p dms -p dms-ui -p dms-engine --all-targets` 交叉检查：通过，0 个警告。这只证明能编译；在 Windows 上运行、截图测试和窗口内菜单的实际操作要等 Windows 端审查 |

### 1.1 为什么画布自己写

- `egui-snarl` 0.12（最新）依赖 egui 0.36，和 egui 0.37 不兼容，会让工作区里同时存在两套 egui。
- 画布要的东西不多（平移、缩放、节点、连线、选中、拖动、框选、连线编辑），自己写可以完全控制：视图状态是三个可序列化的数（`CanvasView`）；按可见区域裁剪，标签在缩得很小时不画；节点形状和配色是我们自己的。
- egui 0.37 自带 `Scene` 容器也能平移缩放，但它对每个子控件做变换，200 个节点时开销更大，也不方便按缩放级别省略标签。

### 1.2 技术验证中遇到的问题

| 问题 | 处理 |
|---|---|
| `egui-snarl` 版本落后 | 自己写画布（上节） |
| egui 本身不开 `default_fonts` 时，测试里没有任何字体，渲染第一个字就失败 | 测试通过 dev-dependency 打开 `egui/default_fonts`，截图不依赖本机字体；字体的许可证问题见 §11 第 4 条 |
| `muda` 在 Windows 上的 `init_for_hwnd` 是 `unsafe fn` | Windows 用窗口内菜单（任务书允许）；macOS 的 `init_for_nsapp` 是安全函数 |
| egui 的快捷键匹配对 Shift 是“子集”语义，⌘⇧S 也会命中 ⌘S | 修饰键多的快捷键先匹配（`dms_ui::menu::take_shortcuts`） |
| macOS 上原生菜单的快捷键和 egui 的快捷键可能重复触发 | 有原生菜单时 egui 不再处理命令快捷键，只由菜单触发 |
| 无法从本会话自动点击原生菜单（读取其他进程的菜单栏需要给终端开辅助功能权限） | 菜单安装无错误（安装失败时会打印并退回窗口内菜单）；点击验证请负责人手动做，步骤见提交时的汇报 |
| `muda` 的 `Modifiers::SUPER` 已弃用 | 改用 `META` |
| 真实流里，竖直方向的连线会穿过节点下方的标签 | 阶段 B：连线从图块边缘的四个方向中选最近的一个出入，标签加背景色 |
| 建模节点和它生成的模型块用同一个图标，画布上分不出来 | 阶段 B：模型块用单独的样式（例如菱形外框），并在属性面板注明“由哪个建模节点生成” |

## 2. 主窗口布局

macOS（原生菜单在屏幕顶部，窗口内没有菜单栏）：

```text
 Data Mining  File  Edit  View  Run  Window  Help                        ← 系统菜单栏
┌──────────────────────────────────────────────────────────────────────────────┐
│ ● ● ●   model_comparison.dms — Data Mining                                   │ ← 标题栏（文档名，已修改时有圆点）
├──────────────────────────────────────────────────────────────────────────────┤
│ [Open] [Save] │ [▶ Run] [■ Stop] │ [−] [Fit] [+] │ [Nodes] [Properties] [Results] │ ← 工具栏
├───────────────┬──────────────────────────────────────────────┬───────────────┤
│ Nodes  [🔍  ] │                                              │ Properties    │
│ ▾ Sources 9/22│     ┌──┐      ┌──┐      ┌──┐      ┌──┐       │ Select        │
│   Var. File   │     │▤ │────▶│≡ │────▶│▥ │────▶│⋀ │       │ ───────────── │
│   Excel       │     └──┘      └──┘      └──┘      └──┘       │ Mode  (•)Incl │
│   …           │   data.csv   Select    Type     C5.0         │ Condition     │
│ ▸ Record Ops  │                 │                            │ [Age >= 18  ] │
│ ▸ Field Ops   │                 ▼                            │               │
│ ▸ Graphs      │               ┌──┐                           │ ⚠ 2 settings  │
│ ▸ Modeling    │               │⊞ │                           │   kept as     │
│ ▸ Output      │               └──┘                           │   imported    │
│ ▸ Export      │               Table                          │               │
│ (灰色项悬停   │                                              │               │
│  显示原因)    │                            画布：可平移、缩放 │               │
├───────────────┴──────────────────────────────────────────────┴───────────────┤
│ Results: [Table ▾ data.csv→Table] [Preview] [Model: C5.0] [Chart]   ✕        │ ← 结果区（可收起）
│ ┌───────┬────────┬────────┬─────┐                                            │
│ │ Age   │ Sex    │ Income │ … │  1 000 000 rows · virtual scroll           │
├──────────────────────────────────────────────────────────────────────────────┤
│ model_comparison.dms · 24 nodes · 23 links · 1 not supported · 74% · Running C5.0 ▰▰▰▱ 62% [Cancel] │
└──────────────────────────────────────────────────────────────────────────────┘
```

Windows：同样的布局，菜单栏在窗口内工具栏上方（技术验证的截图 `workbench_light_in_window_menu.png` 就是这个样子）。

区域说明：

| 区域 | 内容 | 实现要点 |
|---|---|---|
| 节点面板（左） | 按类别列出 `dms_catalog::NODES`；已实现的可用，未实现的灰显，悬停说明原因（以后再做、许可证受阻、不做）；顶部搜索框；拖到画布或双击添加 | 纯数据驱动，类别和状态来自节点目录和 `dms_engine::nodes` |
| 画布（中） | 节点、连线、超节点（v1 只显示，双击进入只读视图）；框选、拖动、连线、删除、复制粘贴 | §1.1；编辑都走命令 |
| 属性面板（右） | 选中节点的设置表单，按节点的设置模式（schema）生成；导入时保留但不认识的参数单独列出（只读，“按导入保留”） | §5.3 |
| 结果区（下） | 每个终端节点一个结果标签页：表格、数据预览、统计输出、模型块（树、系数、规则）、图表（`egui_plot`） | 大表格虚拟滚动（`egui_extras` 的表格），按需从结果的列式数据取行 |
| 状态栏 | 文件名、节点/连线数、不支持的节点数、缩放比例、正在运行的任务、进度和取消 | 错误时显示出错的节点名和原因，点一下定位到该节点 |

## 3. 主要交互流程

1. **打开或导入。** ⌘O 或拖入文件。`.dms` 是原生格式，直接打开；`.str` 导入成一个未保存的新文档（标题带“已导入”）。导入时识别不了的节点和参数原样保留：画布上虚线框，属性面板列出“按导入保留”的参数，状态栏有计数。第一次保存时询问保存为 `.dms` 的位置，不覆盖 `.str`。
2. **新建流、接数据源。** ⌘N 新建。从节点面板把 Var. File 拖到画布（命令 `node.add`），在属性面板选文件（文件选择是表单控件，结果作为参数传给 `node.set_param`）。
3. **字段处理。** 拖入 Type、Select、Derive 等节点，从上游节点的输出端口拖到下游节点（`node.connect`）。连线规则（端口数、不能成环）在 L1 校验，不合法时连线变红并提示原因，不会生成无效的流。
4. **训练模型。** 拖入建模节点（如 C5.0），连到 Type 后面，选中它按 ⌘R（`run.stream`）或在节点上右键“运行到此”（`run.node`）。运行在后台进行，状态栏显示进度，可取消（`run.cancel`，⌘.）。训练完成后生成模型块，作为节点出现在画布上（和 Modeler 的概念一致：建模节点产出模型块）。
5. **看结果和打分。** 终端节点（Table、Analysis、图表）的结果出现在结果区。模型块的查看器显示树、系数或规则。把模型块接到新数据后面再运行，就是打分。
6. **出错。** 某个节点失败时，运行停在该节点：节点标红，状态栏显示“节点名：原因”，点击定位。其他分支的结果照常显示（执行器已经按节点报告失败，见 `dms-cli run` 的 `root_causes`）。
7. **撤销和保存。** 每个编辑命令都能撤销（⌘Z / ⇧⌘Z）。⌘S 原子保存（§5.4）；未保存时标题栏有圆点，关闭或退出前询问。崩溃后重启时提示从自动保存恢复。

## 4. 命令清单

规则：id 形如 `<area>.<verb>`，一旦发布不改含义。标签是英文源字符串，以后作为翻译键。“⌘” 在 Windows 上是 Ctrl。“阶段”列：A 表示技术验证里已有，B/C/D 表示计划在哪个阶段做（§12）。

| id | 标签 | macOS | Windows | 菜单 | 参数 | 阶段 |
|---|---|---|---|---|---|---|
| `app.about` | About Data Mining | — | — | 应用（Win：Help） | — | A |
| `app.settings` | Settings… | ⌘, | — | 应用（Win：File） | — | B |
| `app.quit` | Quit Data Mining | ⌘Q | （Alt+F4，系统） | 应用（Win：File） | — | A |
| `file.new` | New Stream | ⌘N | Ctrl+N | File | — | B |
| `file.open` | Open… | ⌘O | Ctrl+O | File | `path` | A |
| `file.open_recent` | Open Recent | — | — | File | `path` | B |
| `file.close` | Close | ⌘W | Ctrl+W | File | — | A |
| `file.save` | Save | ⌘S | Ctrl+S | File | （新文档：`path`） | B |
| `file.save_as` | Save As… | ⇧⌘S | Ctrl+Shift+S | File | `path` | B |
| `file.revert` | Revert to Saved | — | — | File | — | B |
| `edit.undo` | Undo | ⌘Z | Ctrl+Z | Edit | — | B |
| `edit.redo` | Redo | ⇧⌘Z | Ctrl+Shift+Z（及 Ctrl+Y） | Edit | — | B |
| `edit.cut` / `edit.copy` / `edit.paste` | Cut / Copy / Paste | ⌘X / ⌘C / ⌘V | Ctrl+X / C / V | Edit | 选中的节点 | B |
| `edit.duplicate` | Duplicate | ⌘D | Ctrl+D | Edit | 选中的节点 | B |
| `edit.delete` | Delete | ⌫ | Delete | Edit | 选中的节点、连线 | B |
| `edit.select_all` | Select All | ⌘A | Ctrl+A | Edit | — | B |
| `node.add` | Add Node | （拖放、双击） | 同左 | 画布右键 | `kind`、`position` | B |
| `node.connect` / `node.disconnect` | Connect / Disconnect | （拖连线） | 同左 | 画布右键 | `from`、`to` | B |
| `node.move` | Move | （拖动） | 同左 | — | `nodes`、`delta` | B |
| `node.rename` | Rename… | ↩︎ | F2 | 画布右键 | `node`、`label` | B |
| `node.set_param` | （属性面板） | — | — | — | `node`、`param`、`value` | B |
| `run.stream` | Run Stream | ⌘R | Ctrl+R | Run | — | C |
| `run.node` | Run to Here | ⇧⌘R | Ctrl+Shift+R | Run、画布右键 | `node` | C |
| `run.cancel` | Stop | ⌘. | Esc | Run | — | C |
| `view.zoom_in` / `view.zoom_out` | Zoom In / Out | ⌘= / ⌘- | Ctrl+= / Ctrl+- | View | — | A |
| `view.zoom_fit` | Zoom to Fit | ⌘0 | Ctrl+0 | View | — | A |
| `view.toggle_palette` / `_properties` / `_results` | Show/Hide … | ⌥⌘1 / 2 / 3 | Ctrl+Alt+1 / 2 / 3 | View | — | B |
| `view.fullscreen` | Enter Full Screen | ⌃⌘F | F11 | View | — | A |
| `help.docs` | Data Mining Help | ⌘? | F1 | Help | — | D |

快捷键遵循 Apple HIG（⌘N、⌘O、⌘S、⇧⌘S、⌘Z、⇧⌘Z、⌘W、⌘Q、⌘,、⌃⌘F）。⌘R 运行是常见做法（Xcode）。单元测试 `command::tests` 检查：id 唯一且格式正确、两个平台各自没有快捷键冲突、macOS 的标准快捷键。

## 5. L1：编辑模型和原生保存格式

### 5.1 现状和原则

`dms-stream` 现在是 `.str` 的只读解析结果：节点设置保存为原始 XML 元素（`settings: Option<Element>`），各节点实现自己读。执行器（`dms-exec::Plan`）以 `dms_stream::Stream` 为输入。

原则：**不重写执行器的设置读取。** 编辑模型以“原始设置树 + 有类型的访问层”为中心：已知参数通过节点的设置模式读写设置树中对应的元素；不认识的部分原样保留。这样导入的流不丢信息，执行器照旧读设置树，一次只需给一个节点补上设置模式。

### 5.2 数据结构（`dms-stream`，L1）

```rust
pub struct StreamDoc {
    pub format: FormatHeader,           // "dms-stream", version
    pub meta: StreamMeta,               // 名称、作者、创建/修改时间、导入来源（.str 文件名和 saveVersion）
    pub properties: Settings,           // 流属性（日期格式、小数点等；现为 stream/properties）
    pub nodes: Vec<Node>,               // 保持插入顺序，按 id 索引另建 HashMap
    pub links: Vec<Link>,
    pub terminal_order: Vec<NodeId>,
    pub attachments: Vec<Attachment>,   // 模型 PMML 等，按 id 引用
    pub extra: BTreeMap<String, Value>, // 新版本写入、本版本不认识的字段，原样写回
}

pub struct Node {
    pub id: NodeId,                     // 导入时用 .str 的 uid，新建时生成
    pub kind: NodeKind,                 // Catalog("select") | Unknown { type_identifier }
    pub label: Option<String>,
    pub position: Pos,                  // 画布坐标（文档数据，不是视图状态）
    pub settings: Settings,             // 设置树（见下）
    pub attachments: Vec<AttachmentRef>,
    pub subdiagram: Option<Diagram>,    // 超节点（v1 只读）
    pub extra: BTreeMap<String, Value>,
}

/// 设置树：与 .str 的设置元素一一对应，可以无损往返。
pub struct Settings { pub root: Element /* 现有的 fastinfoset::Element */ }
```

编辑是一组可逆操作，所有修改都经过它们（L1 只提供操作，L4 的命令负责组合和撤销）：

```rust
pub enum Edit {
    AddNode { node: Node, index: usize },
    RemoveNode { id: NodeId },                       // 连带的连线由命令先单独删除
    Connect { link: Link, index: usize },
    Disconnect { link: Link },
    Move { ids: Vec<NodeId>, delta: Vec2 },
    Rename { id: NodeId, label: Option<String> },
    SetSetting { id: NodeId, path: SettingPath, value: Option<SettingValue> },
}

impl StreamDoc {
    /// 校验后应用，返回逆操作；失败时文档不变。
    pub fn apply(&mut self, edit: Edit) -> Result<Edit, EditError>;
}
```

校验在 `apply` 里做：id 唯一；连线两端存在、不成环（超节点内外分别检查）；端口数符合节点注册表（L3 已知每种节点的输入数，L1 通过参数接收规则，不依赖上层）；坐标、字符串长度、设置树深度都有上限。

### 5.3 设置模式（schema）和属性表单

每种已实现节点在 `dms-exec` 的注册表旁登记设置模式（L3，紧挨读取设置的代码，避免两处漂移）：

```rust
pub struct ParamSpec {
    pub key: &'static str,          // 设置树中的路径，如 "mode"、"condition"
    pub label: &'static str,        // 英文源字符串（翻译键）
    pub kind: ParamKind,            // Bool, Int{min,max}, Real{..}, Text, Expr(CLEM), Choice(&[..]),
                                    // Field{measure/role 过滤}, FieldList, FilePath{filters}, Table(列定义)
    pub default: DefaultValue,
    pub visible_if: Option<Condition>, // 依赖其他参数（如 mode = custom 时才显示）
}
```

属性面板只做一件事：按 `ParamSpec` 列表生成控件，用户改值时发出 `node.set_param` 命令。字段列表、测量级别、角色这些选项来自上游节点运行或推断出的数据模型（`dms_types::DataModel`），界面不写死。没有设置模式的节点显示只读的设置树，以及“按导入保留”的提示。

### 5.4 原生保存格式

推荐 **TOML**，扩展名 `.dms`，UTF-8、LF（理由和替代方案见 §11 第 7 条）：

```toml
format = "dms-stream"
version = 1

[meta]
name = "model_comparison"
imported_from = { file = "model_comparison.str", save_version = "19.0" }

[[node]]
id = "id228WNW9FCUR"
kind = "statistics_file"
label = "customers"
position = [270, 427]
[node.settings]                       # 有模式的参数：有类型的键值，可 diff
full_filename = "data/customers.sav"
read_names_and_labels = true

[[node]]
id = "id95XMSGEDSH"
kind = "unknown"
type_identifier = "SomeFutureNode"
position = [400, 427]
raw_settings = '''
<properties>
  <property name="something">…</property>
</properties>
'''                                   # 不认识的设置：原样保留的 XML，多行字符串，diff 友好

[[link]]
from = "id228WNW9FCUR"
to = "id95XMSGEDSH"

[[attachment]]
id = "data/0002.dat"
media_type = "application/pmml+xml"
text = '''<?xml version="1.0"?> <PMML …> … </PMML>'''   # PMML 是文本，直接存
# 二进制附件：encoding = "base64"，单个上限 64 MiB
```

- **兼容**：所有结构体 `#[serde(default)]`；不认识的字段进 `extra` 并原样写回；`version` 只在不兼容时升级，并附迁移函数和测试。
- **确定性**：字段顺序固定，同一文档每次保存逐字节相同（测试保证），便于 git diff 和代码评审。
- **原子保存**：先写同目录下的临时文件 `.<name>.dms.tmp-<pid>`，`sync_all`，再 `rename` 覆盖目标（Windows 上 `std::fs::rename` 替换已有文件），最后同步目录。任何一步失败都不碰原文件。
- **自动保存和恢复**：文档有修改时每 30 秒写一份到应用数据目录（`~/Library/Application Support/dms/autosave/`，Windows 为 `%APPDATA%\dms\autosave\`），正常保存或关闭后删除；启动时如果自动保存比文件新，提示恢复。
- **`.str` 只导入，不写出。**

### 5.5 从 `.str` 导入，以及执行器的输入

导入：`dms_stream::Stream`（现有解析器）→ `StreamDoc`。节点设置树直接搬过去，附件按 ZIP 成员名搬进 `attachments`。执行时用 `StreamDoc::to_stream()` 生成执行器现在要的 `dms_stream::Stream`，以及 `ExecContext.attachments`。转换是纯函数，有往返测试：对语料里的每个 `.str`，导入 → 保存 → 读回 → 转换，与直接解析的结果比较，节点、连线、设置树、附件逐一相同（语料测试，放在 `corpus` feature 后面，数据不提交）。

## 6. L4：命令注册表、Session、撤销、长任务

### 6.1 命令注册表（`dms-engine::command`）

技术验证里已有的部分：`CommandSpec { id, label, menu, group, shortcuts, requires }`，`COMMANDS` 静态表，`by_id`；`Requires::{Nothing, Document, Planned(reason)}`；分平台的快捷键。阶段 B 补上：

```rust
pub struct CommandSpec {
    // …现有字段…
    pub params: &'static [CommandParam],          // 名称、类型、是否必填、说明（CLI 和 MCP 据此生成帮助）
    pub handler: fn(&mut Session, &Args) -> Result<Outcome, CommandError>,
    pub enabled: fn(&Session) -> Availability,    // 取代 Requires，可以看选中项等状态
    pub undoable: bool,
}
```

- **调用约定（已实现）**：`Session::execute(id, args) -> Result<Outcome, CommandError>`。缺少需要用户选择的参数时返回 `Outcome::NeedsArgument(name)`，由界面弹对话框再调一次；程序调用永远不会弹对话框。视图类动作（缩放、全屏）返回 `Outcome::View`，由界面执行，因为引擎不持有视图状态。
- **同一套命令的四个入口**：macOS 原生菜单、窗口内菜单和快捷键、`dms-cli cmd <id> key=value …`（阶段 B）、MCP（以后）。都按 id 调用。
- **测试**：每个命令至少一个单元测试（技术验证里有“所有登记的命令要么能执行，要么说明不可用的原因”的总测试）；编辑命令另测撤销后文档与之前逐字节相同。

### 6.2 Session

```rust
pub struct Session {
    documents: Vec<DocumentState>,   // v1 一个窗口一个文档；数据结构支持多个
    active: Option<DocId>,
    jobs: JobManager,
    settings: AppSettings,           // 语言、最近文件、自动保存间隔等，存在应用数据目录
}

pub struct DocumentState {
    pub id: DocId,
    pub path: Option<PathBuf>,       // None：新建或导入后未保存
    pub doc: StreamDoc,
    pub undo: UndoStack,
    pub revision: u64,               // 每次修改加一
    pub saved_revision: Option<u64>, // 与 revision 不同即“已修改”
    pub selection: Vec<NodeId>,      // 选中项属于文档会话（命令要用），但不写进文件
    pub results: ResultStore,        // 按终端节点存最近一次运行结果，附运行时的 revision
}
```

### 6.3 撤销/重做

- 一个命令 = 一个事务：命令内部可以应用多个 `Edit`（删除节点 = 先断开连线再删节点），撤销栈记录整组逆操作，撤销时逆序应用。
- 合并：拖动节点的整个过程算一步；在属性面板连续输入同一参数，1 秒内合并为一步。
- 上限：500 步，或逆操作总大小 64 MiB，超出时丢弃最早的。
- “已修改”状态由 `revision` 和 `saved_revision` 比较得出，撤销回保存时的状态后标题栏的圆点消失。

### 6.4 长任务（运行流、训练、读大文件）

```rust
pub struct JobManager { /* worker 线程、任务表 */ }

impl JobManager {
    pub fn spawn(&mut self, kind: JobKind, work: impl FnOnce(&JobContext) -> JobResult + Send + 'static) -> JobId;
    pub fn cancel(&self, id: JobId);
    pub fn poll(&mut self) -> Vec<JobEvent>;   // 界面每帧调用
}

pub struct JobContext { pub cancel: CancelToken, pub progress: ProgressSink }
pub enum JobEvent { Started(JobId), Progress { id, fraction: Option<f32>, message }, NodeFinished { id, node, ok },
                    Finished { id, result: Result<JobOutput, JobError> } }
```

- 工作在独立线程里进行，整段用 `std::panic::catch_unwind` 包住（release 配置已是 `panic = "unwind"`），panic 转成 `JobError::Internal`，界面显示错误，不会卡在“运行中”，也不会闪退。
- 运行用的是文档在某个 revision 的快照（`StreamDoc` 克隆后转换），运行期间可以继续编辑；结果带上 revision，文档已经改过时，结果区标明“结果来自较早的版本”。
- 引擎不依赖 egui。界面在创建 Session 时传入一个唤醒回调（`Arc<dyn Fn() + Send + Sync>`，内部调用 `ctx.request_repaint()`），任务有事件时调用它。
- **取消需要执行器配合**：`dms-exec` 现在没有取消机制。阶段 C 在 `ExecContext` 里加 `CancelToken`：节点之间检查一次，耗时的算法（神经网络、Time Series 专家、C5.0 提升等）在迭代循环里检查。取消后返回 `OperatorError::Cancelled`，已完成的节点结果保留。

## 7. 界面状态的序列化和截图测试

### 7.1 界面状态

| 状态 | 存在哪里 | 格式 |
|---|---|---|
| 窗口大小、位置、全屏 | eframe（`persist_window`） | eframe 存储（RON） |
| 面板开合和宽度、画布平移/缩放/选中（`ShellState`） | eframe 存储，键 `shell` | serde，`#[serde(default)]`，读回后 `sanitize()`（限制缩放和平移范围、清除 NaN） |
| 每个文档的视图（平移、缩放） | 阶段 B：`ShellState` 里按文档路径的散列存最近 20 个 | 同上 |
| 应用设置（语言、最近文件、自动保存） | 应用数据目录 `settings.toml` | 有版本号 |
| 节点位置 | 文档（`.dms`），因为它属于流本身 | §5.4 |

所有界面状态都是普通数据结构，测试可以构造任意状态再渲染（技术验证的截图测试就是这样做的）。

### 7.2 截图测试方案

- **工具**：`egui_kittest` 0.37 + wgpu 离屏渲染（Mac 上是 Metal，Windows 上是 DX12/Vulkan）。测试位于 `crates/ui/tests/`，在 `cargo xtask ci` 的 `cargo test --workspace` 中运行。
- **数据**：只用 `dms_ui::synthetic` 生成的流和我们自己构造的结果表；不含 IBM 或团队数据，也不含 Modeler 截图。
- **确定性**：固定窗口尺寸、`pixels_per_point`（1.0 和 2.0 各测关键画面）、主题（浅、深各一）；测试使用 egui 自带字体（只在 dev-dependency 里打开），不依赖本机字体；不显示时间、帧率这类会变的内容。
- **容差**：kittest 默认的单像素阈值 0.6（YIQ 距离），再允许 64 个像素超阈值（不同 GPU 的抗锯齿差异）。如果 Windows 上差异更大，在仓库根目录的 `kittest.toml` 里按操作系统设置，而不是放宽所有平台；实在不行，再为 Windows 单独保存基准图。
- **更新基准**：`UPDATE_SNAPSHOTS=1 cargo test -p dms-ui --test snapshots`，提交前人工检查 `.png` 的改动；`*.new.png`、`*.diff.png` 已被 `.gitignore` 忽略。
- **交互测试**：kittest 通过 AccessKit 树按标签查找控件（例如 `get_by_label("No stream open")`），模拟点击和按键，再检查 `Workbench` 的状态。这同时保证无障碍信息完整。
- **性能**：`crates/ui/tests/perf.rs`（`#[ignore]`，release 模式手动运行）测 CPU 单帧开销；`dms --bench N` 测真实窗口的帧率。阶段 D 以后由 `cargo xtask perf` 统一运行并记录。

## 8. Mac 体验的落实方式

| 要求 | 做法 | 技术验证 |
|---|---|---|
| 原生菜单栏、⌘ 快捷键 | `muda`，由命令表生成；可用状态每帧同步 | 已做（菜单点击请负责人手动确认） |
| Windows 窗口内菜单、Ctrl 快捷键 | 同一命令表生成 egui 菜单；快捷键由 `take_shortcuts` 处理 | 已做，截图测试覆盖 |
| 触控板平移、捏合缩放、惯性 | egui 的 `smooth_scroll_delta`（含系统惯性滚动）和 `zoom_delta`（捏合） | 已做 |
| Retina | 节点图标全是矢量图形，文字由 egui 按像素比栅格化 | 已做（截图测试在 1.0 像素比；阶段 B 增加 2.0） |
| 浅色、深色 | 跟随系统；节点配色两套 | 已做 |
| 系统字体、中文、捷克语 | eframe 系统字体回退（缺字时加载系统字体）；是否改用系统字体为主字体见 §11 第 4 条 | 捷克语已验证；中文在应用里依赖系统字体回退，阶段 B 加中文截图测试（需要先定字体） |
| 记住窗口、全屏、拖入文件 | eframe 持久化；`view.fullscreen`；拖入 `.str`/`.dms` 时打开 | 已做 |
| 200 节点 60 fps | 可见区域裁剪；缩得很小时不画标签 | 已做，60 fps（开垂直同步），176 fps（关） |
| 百万行预览 | `egui_extras` 表格的虚拟滚动，只取可见行；结果按列存（`dms_types::Table`） | 阶段 C |
| 启动 ≤ 2 秒 | 启动时不做任何重活，节点目录是静态表 | 首帧 150–170 ms |
| AccessKit | eframe 默认开启 | 开启（kittest 交互测试依赖它） |
| `.app` / `.dmg` | 阶段 D：`cargo xtask bundle-mac`（Info.plist、图标、文档类型 `.dms`/`.str`），签名和公证见 §11 第 9 条 | — |

## 9. 依赖清单和许可证

版本固定在根 `Cargo.toml` 的 `[workspace.dependencies]`（`=` 精确版本）；所列 crate 声明的 `rust-version` 都不高于仓库的 1.95.0。

### 9.1 直接依赖（本分支新增）

| 名字 | 版本 | 许可证 | 用途 | 使用者 |
|---|---|---|---|---|
| `egui` | 0.37.0 | MIT OR Apache-2.0 | 即时模式界面库 | `dms-ui` |
| `eframe` | 0.37.0（默认特性 + `persistence`） | MIT OR Apache-2.0 | 窗口、事件循环、wgpu 渲染（Mac 上 Metal）、状态持久化、系统字体回退 | `dms` |
| `egui_kittest` | 0.37.0（`wgpu`、`snapshot`） | MIT OR Apache-2.0 | 离屏渲染、截图测试、交互测试（只在 dev-dependencies） | `dms-ui` 测试 |
| `muda` | 0.21.2（无默认特性） | Apache-2.0 OR MIT | macOS 原生菜单栏（只在 `cfg(target_os = "macos")`） | `dms` |
| `rfd` | 0.18.0 | MIT | 原生文件对话框 | `dms` |
| `serde` | 1（`derive`） | MIT OR Apache-2.0 | 界面状态序列化（仓库里已间接使用） | `dms-ui` |
| `serde_json` | 1 | MIT OR Apache-2.0 | 测试里的状态往返（仓库里已在用） | `dms-ui` 测试 |
| `egui_extras` | 0.37.0（无默认特性） | MIT OR Apache-2.0 | 大表格虚拟滚动（v0.1） | `dms-ui` |
| `toml` | 1.2.1（`unbounded`） | MIT OR Apache-2.0 | 原生格式 `.dms`（v0.2） | `dms-stream` |
| `arboard` | 3.7.0 | MIT OR Apache-2.0 | 系统剪贴板（v0.2） | `dms` |
| `egui_plot` | 0.38.0（无默认特性；对应 egui 0.37） | MIT OR Apache-2.0 | 图表节点结果的图（v0.4）；只新增这一个包 | `dms-ui` |

`egui_extras`、`toml`、`arboard`、`egui_plot` 已按计划引入并登记在上表。停靠面板（`egui_tiles` 或 `egui_dock`）v1 不引入，固定面板已能满足第一版。

评估后不采用：`egui-snarl` 0.12（依赖 egui 0.36，见 §1.1）。

### 9.2 传递依赖

本分支给 `Cargo.lock` 新增 417 个包（wgpu、naga、winit、objc2、windows-* 等，含我们自己的 `dms`、`dms-ui`）。按 `cargo metadata` 的许可证字段统计：

| 许可证 | 包数 |
|---|---|
| MIT 和/或 Apache-2.0（含 “Apache-2.0 WITH LLVM-exception” 选项） | 352 |
| Zlib，或含 Zlib 的多选一 | 22 |
| BSD-2/3-Clause，或含 BSD 的多选一（`zerocopy`、`tiny-skia`、`num_enum` 等） | 9 |
| ISC、Unlicense OR MIT（`libloading`、`walkdir`、`termcolor` 等） | 6 |
| Unicode-3.0（ICU4X 系列：`icu_*`、`zerovec`、`yoke`、`tinystr` 等） | 19 |
| 可选 MIT/Apache 的多选一（`self_cell`：Apache-2.0 OR GPL-2.0-only；`r-efi` 两个版本：MIT OR Apache-2.0 OR LGPL-2.1-or-later） | 3 |
| BSL-1.0（`clipboard-win`、`error-code`：Windows 剪贴板） | 2 |
| MPL-2.0（`colored`，经 `egui_kittest` → `dify`，只在测试里） | 1 |
| `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`（`epaint_default_fonts`，egui 自带字体） | 1 |
| 我们自己的 crate（`LicenseRef-Proprietary`） | 2 |

需要负责人确认的是最后四类，见 §11 第 4–6 条。可以选择宽松许可的双许可包，我们一律按 MIT 或 Apache-2.0 使用。

## 10. 风险

| 风险 | 影响 | 应对 |
|---|---|---|
| egui 不是原生控件，外观与 AppKit 有差距 | Mac 用户觉得“不够原生” | 菜单栏、文件对话框、窗口、全屏、深色模式都用系统的；控件按 macOS 调整间距、圆角和字体；见 §11 第 2 条 |
| 执行器没有取消机制 | 长时间训练无法停止 | 阶段 C 在 `ExecContext` 加 `CancelToken`，算法循环里检查；在此之前“停止”只能放弃结果、让线程在后台跑完 |
| 设置模式要逐个节点补（85 个已实现节点） | 属性面板的覆盖面增长慢 | 先做完整建模流程用到的约 15 个节点（Var. File、Statistics File、Excel、Type、Filter、Derive、Select、Partition、C5.0、CHAID、Regression、Logistic、Table、Analysis、Histogram）；其余先显示只读设置树 |
| 截图在不同 GPU 上有像素差 | Windows 上测试不稳定 | 按操作系统设阈值（`kittest.toml`），必要时为 Windows 单独保存基准图；无 GPU 的 Windows 虚拟机可以用 DX12 WARP 软件渲染 |
| wgpu 在旧显卡或远程桌面上不可用 | Windows 上无法启动 | eframe 支持 glow（OpenGL）后端，作为备用特性；启动失败时给出明确的错误信息 |
| 原生菜单和 egui 同时处理快捷键 | 一次按键触发两次 | 有原生菜单时 egui 不处理命令快捷键（已实现）；阶段 B 加交互测试 |
| 字体：中文依赖系统字体回退；egui 自带字体的许可证 | 显示和许可证 | §11 第 4 条 |
| 大文档的撤销内存 | 内存增长 | 逆操作只存差异；上限 500 步或 64 MiB |
| `.dmg` 需要 Apple 开发者账号签名和公证 | 未签名的应用会被 Gatekeeper 拦截 | §11 第 9 条 |
| 依赖升级（egui 大约每季度一个大版本，API 常有变化） | 升级成本 | 版本精确固定；界面代码集中在 `dms-ui`；每次升级单独提交，并跑截图测试 |

## 11. 待负责人决定的问题和建议

| # | 问题 | 选项 | 建议 |
|---|---|---|---|
| 1 | 界面语言 | 只做英文；英文 + 中文；再加捷克语 | **英文 + 中文**。第一版起所有界面文字经过翻译表（每种语言一个 TOML 文件，编译进程序，缺词时回退到英文），语言在设置里切换，默认跟随系统。捷克语等团队需要时再加，只是多一个文件。节点名称保持 `catalog.rs` 的英文名，不翻译，方便对照文档 |
| 2 | 视觉风格 | 接近 macOS 原生；egui 默认样式 + 自己的主题 | **egui + 我们自己的“类 macOS”主题**：系统菜单、对话框、窗口外框都用原生的；控件不去模仿 AppKit 的每个细节，只统一间距、圆角、字号和强调色，浅色和深色各一套。完全模仿原生控件成本高，而且在 Windows 上反而别扭 |
| 3 | 节点图标 | 自己画；宽松许可的图标集 | **自己画矢量图标**（技术验证里每个类别一个形状和颜色，色盲用户也能区分），阶段 B 为最常用的约 30 个节点各画一个专属图标。工具栏的通用图标可以采用 Lucide（ISC）或 Phosphor（MIT），登记来源和许可证 |
| 4 | 字体许可证 | egui 自带字体（Ubuntu Font Licence、SIL OFL）随应用分发；或只用系统字体 | **应用只用系统字体，不分发 egui 的字体**：macOS 用 SF Pro 和 PingFang，Windows 用 Segoe UI 和 Microsoft YaHei，运行时从系统加载，不打包。egui 自带字体只留在测试里（dev-dependency），保证截图在每台机器上相同。需要你确认，并允许我在阶段 B 改掉 eframe 的默认特性（目前技术验证的应用仍打包了这些字体） |
| 5 | 测试依赖中的 MPL-2.0（`colored`，经 `egui_kittest` 的截图比对） | 接受（只在测试中使用，不随应用分发）；或换截图比对实现 | **接受**，在依赖清单里注明“仅测试” |
| 6 | 宽松但不在任务书清单里的许可证：BSL-1.0（Windows 剪贴板）、Unicode-3.0（ICU 数据） | 接受；或寻找替代 | **接受**，把这两个加进允许清单。两者都是 OSI 认可的宽松许可，替代它们要放弃剪贴板或 Unicode 处理 |
| 7 | 原生格式用 TOML 还是 JSON | TOML；JSON | **TOML**：不认识的设置要以 XML 原样保存，PMML 也是文本，TOML 的多行字符串能让它们按行 diff，JSON 只能转义成一行。代价是 TOML 不适合很深的嵌套，而我们的嵌套在设置树里，以字符串或简单表格表示，问题不大 |
| 8 | 原生文件扩展名、双击打开 | `.dms`；其他 | **`.dms`**，在 `.app` 的 Info.plist 里登记 `.dms` 为本应用文档、`.str` 为可导入文档 |
| 9 | `.dmg` 的签名和公证 | 用学校或团队的 Apple 开发者账号；暂不签名（用户右键打开） | 阶段 D 前决定即可；团队内部使用可以先不签名，附说明 |
| 10 | Windows 的菜单 | 窗口内 egui 菜单；为 `muda` 开一个只含 `unsafe` 调用的小 crate | **窗口内菜单**，保持 `unsafe_code = "forbid"` 全局有效 |
| 11 | 超节点 | v1 只显示（任务书）；v1 就支持编辑 | **v1 只显示**，双击进入只读视图 |

## 12. 阶段 B 起的实施顺序

每一步都能编译、有测试、`cargo xtask ci` 在 Mac 上通过，并保证能在 Windows 上编译。

1. **B1 L1 编辑模型**：`StreamDoc`、`Edit`/`apply`、校验；`.str` → `StreamDoc` 导入和 `to_stream()`；TOML 读写、原子保存；语料往返测试。
2. **B2 L4**：命令注册表加上处理函数和参数说明；`Session` 管理文档、撤销/重做、选中项；`dms-cli cmd`；自动保存和恢复。
3. **B3 界面编辑**：画布上添加、移动、连线、删除、复制粘贴、框选；节点面板拖放和搜索；属性面板按设置模式生成（先做 15 个节点）；翻译表（英文、中文）。
4. **C 运行和结果**：`JobManager`；`ExecContext` 的取消；运行整个流或单个终端节点；进度和取消；出错时定位节点；结果区（表格、虚拟滚动预览、统计输出、模型块查看器、`egui_plot` 图表）。
5. **D 打包和打磨**：`.app`/`.dmg`、文档类型、图标；性能预算（`xtask perf`）；Windows 端完整走一遍建模流程。
