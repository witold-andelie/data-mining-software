# `.str` 流文件格式

> 更新：2026-10-15 · 依据：ITU-T X.891（05/2005）公开规范，以及差分探针 `cargo xtask probe str_basics`（Modeler 19.0.0.0.26 保存的 24 个文件）和 72 个语料流的观察 · 实现：`crates/strfmt`（容器 + Fast Infoset）、`crates/stream`（流模型）

本文只记录可观察的格式事实，并区分**已验证**（规范或差分实验直接证明）与**推断**（从语料归纳、尚未用探针单独验证）。

## 1. 容器（已验证）

`.str` 是标准 ZIP（deflate 压缩）：

| 成员 | 内容 |
|---|---|
| `<名称>.xml`（`data/` 之外唯一的 `.xml`） | 流文档，**Fast Infoset** 编码的 XML（见第 2 节）。新建流里叫 `clementineStream.xml`，旧文件用流名 |
| `data/NNNN.dat` | 附件，由流文档中的 `container@containerData` 或 `node@properties` 引用 |

附件类型（按首字节分类，`dms_strfmt::container::classify`）：纯文本 XML、Fast Infoset、ZIP、gzip、JSON、空文件、“4 字节大端长度 + XML/ZIP/JSON/HTML”的长度前缀格式，以及未识别的二进制（如以 `SCF\x01` 开头）。

Time Series 模型块的 PMML 容器（`ts_arima.str` `data/0003.dat`）是长度前缀 ZIP：local extra 不是 ZIP extra 记录（Python `zipfile` 报 `Corrupt extra field`），flag bit 3 使用 data descriptor。`dms_strfmt::read_length_prefixed_zip` 只走 local header 并解 raw deflate。条目：`0.xml`/`1.xml`（每个目标一份 PMML `TimeSeriesModel`）、`datamodel.xml`、`TSDPOutput.json`、`.meta`。

## 2. 流文档的编码：Fast Infoset（已验证）

文件头 `E0 00 00 01 00` 是 X.891 的标识（16 位）、版本 1（16 位）、1 位填充和 7 位“可选组件”标志（全 0）。之后完全按 X.891 附录 C 编码：

- 元素和属性名使用字面量限定名（无前缀、无命名空间），名称进入名称表，再次出现时用索引；
- 文本和属性值多数设置了 add-to-table，重复的值（`true`、`false`、换行）用索引引用；
- 字符串全部是 UTF-8，未见受限字母表或编码算法（解码器仍按规范支持）；
- 长字符串（7 万字符）用 32 位长度；中文按 UTF-8 存储。

实现完全依据规范，不依赖任何 Modeler 代码。语料中 72 个流全部解码成功，严格检查填充位和文档结束位置。

## 3. XML 结构（已验证）

```text
clementineStream[@version @date]          保存该文件的 Modeler 版本、保存时间
  stream[@saveVersion]
    properties/*                          流属性：简单子元素，例如 dateBaseline=1900、
                                          date2DigitBaseline=1930、scriptLanguage=python
    diagram
      nodes/node[@id @uid @class @saveVersion]
        properties/*                      节点设置（Modeler 内部名称，见第 5 节）
          name, label, autoLabel          名称；setLabel 写 label 并把 autoLabel 置 false
          position/x, position/y          画布坐标
        container[@containerData]/containerName   附件引用及其类型（PMML、StatXML、PSMData）
        contentContainers/containers/container[@name]/content[@content]
                                                Analytic Server / Time Series（`spss.ts`）附件
        modelBuildInfo[@modelBuilderId]   模型块：建模节点的类型标识
        diagram                           超节点的内部图（结构同上，可嵌套）
      links/link[@srcId @dstId]
      comments
      terminalNodeOrder/node[@id]         终端节点的执行顺序
```

## 4. 节点身份与类型标识

- **`@uid` 是稳定身份**（已验证）。**`@id` 只是一次保存内的序号**：增加节点或连线后会重新编号（探针 vf_16、vf_17）。连线和 `terminalNodeOrder` 引用 `@id`，必须在同一个 diagram 内解析为 uid。
- **类型标识**（已验证于语料）：
  - 普通节点：`@class` 的类名短名，如 `com.spss.clementine.component.node.SelectNode` → `SelectNode`；
  - 扩展框架节点：`@extensionID` + `@extensionObjectID`，标识写作 `对象ID!扩展ID`，如 `LinearRegression!spss.alm-extension`；此时 `@class` 是通用外壳（`CFBuildModelNode` 等），节点设置在 `@properties` 指向的附件里；
  - 旧式扩展节点：`@extensionID` + `@extensionElementID`，标识写作 `扩展ID!元素ID`（顺序相反），如 `spss.bayesian_id!buildBN`；`@class` 是 `ExtensionBuildModelNode` / `ExtensionApplyModelNode`；
  - 模型块的类型由 `modelBuildInfo@modelBuilderId` 给出，覆盖率统计时计入对应的建模节点。
- `SuperNodeInputConnectorNode`、`SuperNodeOutputConnectorNode` 是超节点的结构节点。

## 5. 节点设置的命名与取值（已验证，按节点逐步扩充）

Python 脚本接口的属性名与文件中的元素名不同，映射要逐个节点用探针确定：

| 节点 | 脚本属性 | 文件元素 | 观察到的取值 |
|---|---|---|---|
| Var. File | `full_filename` | `inputFileName` | 原样字符串（含 7 万字符、中文） |
| Var. File | `read_field_names` | `fieldNamesIncluded` | `true` / `false` |
| Var. File | `skip_header` | `skipHeaderCount` | 十进制整数（0、1、300、70000） |
| Sample | `rand_pct` | `sampleRandom` | Java `Double.toString` 形式（`50.0`、`12.5`、`0.1`）；默认 `50.0`；越界值被截到 `0.0` |
| 所有节点 | `setLabel(...)` | `label` + `autoLabel=false` | |
| 所有节点 | `setXYPosition` | `position/x`、`position/y` | 整数 |

## 6. 每次保存都会变的内容

同一个未修改的空流连续保存两次，XML 完全相同（探针 empty_01/02）。随保存变化的是 `clementineStream@date`、各处 `lastSavedDate`（毫秒时间戳）以及 ZIP 成员时间戳；节点 `@id` 随图的变化重新编号。比较两个流时应忽略这些内容。

## 7. 未解决的问题

- 附件格式：PMML（标准）、StatXML、PSMData、`SCF\x01` 二进制；长度前缀 ZIP（Time Series）已按 local header 读取。
- 扩展节点 `@properties` 附件里的设置格式。
- 元素在 `properties` 中的顺序是否稳定（目前读取不依赖顺序）。
- 节点设置的完整映射表：需要按节点逐个探针（P2 的工作）。

## 节点设置映射

各节点的脚本属性到文件元素的映射见 [`nodes/`](nodes/)（每个节点类型一个文件，`common.md` 为所有节点共有的属性），汇总的机器可读表见 [`node-settings.tsv`](node-settings.tsv)。均由差分探针 `cargo xtask probe settings_<type>` 得到。

## Analytic Engine nuggets（`models_as`，2026-10-18）

**已验证的保存文件事实。** 读取既有 `models_as_streams.py` 产出的 16 个流（本次没有运行 probe）：Random Trees、Tree-AS、Linear-AS、LSVM 均为 `ExtensionApplyModelNode`，标识分别为 `spss.randomTrees!applyRandomTrees`、`spss.treeEngine!applyTree`、`spss.linearEngine!applyLinearEngine`、`spss.linearSVM!applyLinearSVM`。PMML 的附件路径在 `contentContainers/containers/container[@name="PMML"]/content@content`；附件为四字节大端长度加 ZIP，成员 `.meta` 和 `PMML.xml`，local extra 元数据沿用已有有界 ZIP 读取器。StatXML、StatJSON、Output 容器不参与评分。

| 脚本属性 | 保存的 properties 子元素 | 本批观察值 |
|---|---|---|
| `calculate_conf`（Random Trees、Tree-AS） | `_calculate_conf` | `true` / `false` |
| `display_rule_id`（Tree-AS） | `_display_rule_id` | `true` |
| `calculate_raw_propensities`（LSVM） | `_calculateRawProp` | `false` |
| `enable_sql_generation` | `_GenerateSQL` | 默认保存为 `false`，显式 native 保存为 `native` |

**已验证的输出差异。** `rt_noconf`、`rt_batch` 的两份 nugget 均保存 `_calculate_conf=false`。自动回填 nugget 有 `refreshLinkState=TurnOnReplaceLink` 和非空 `repalcebuilderNodeId`（拼写保留），oracle 中仍有 `$RC-respond`；独立副本无自动替换标记，也无该字段。

**推断。** oracle 会运行 builder，可能由自动替换将原始 nugget 的置信度字段重置到默认。评分层按上述标记组合兼容该已观察的输出，不重训、不修改流。其他替换顺序和仅加载保存流但不运行 builder 的行为未验证。完整评分规则见 [`../models.md`](../models.md) 第 6 节。
