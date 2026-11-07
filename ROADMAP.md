# Roadmap

> 阶段：**P1、P2 完成；P3 除 Python 桥外完成；P0 只差 Mac 上的 CI；P4 放到最后** · 更新：2026-11-06

## 核心数字

| 数字 | 值 | 类型 |
|---|---|---|
| MVP 节点覆盖 | 85 / 85（其中 10 个按统计等价验收） | 实测（`cargo xtask parity`，2026-10-20） |
| 以后再做 / 许可证受阻 / 不做 | 已做 5 / 24；0 / 6；0 / 34 | 实测（`cargo xtask parity`） |
| 回归（全部带标准答案的流） | 1518 个流：1384 完全一致、86 统计等价、48 已知 GAP、0 回归 | 实测（`cargo xtask regress`，2026-11-07） |
| 语料 | `xtask/corpus.tsv` 登记 1638 个文件，80 个 oracle 标签 | 实测 |

## 阶段

| 阶段 | 内容 | 完成标准 |
|---|---|---|
| P0 | 仓库骨架、分层、xtask、节点目录、语料、oracle 流程打通 | Windows 和 Mac 上 `cargo xtask ci` 通过；oracle 对 1 个示例流产出标准答案 |
| P1 | `.str` 格式规格文档 + `strfmt` 解码器 + 覆盖率报告 | 语料中所有 `.str` 解码无错误 |
| P2 | 类型系统、CLEM、数据节点、两套执行器、命令行跑流 | 选定流端到端结果与标准答案一致 |
| P3 | 建模（原生算法 + Python 桥）+ PMML | 模型指标和打分与标准答案一致 |
| P4 | egui 界面，macOS `.app`/`.dmg` | 在 Mac 上完成完整建模流程 |
| P5 | 打磨：导入率、性能预算、记分卡 | 达到约定目标 |

先做哪些节点按团队语料的使用频率排序；MVP 范围是单机版 Modeler 的标准面板（含 Python 面板）。

## 进度日志

- 2026-11-07：C5.0 筛除的最后一例 `c50_winnow2/w03` 解决：新探针 `c50_trial`（24 例）把每个筛除数据集按确定性奇偶规则切出训练半份，让 Modeler 在其上建普通树（不筛除、不全局剪枝、严重度 0 与 75），24 棵全部与我们一致，且 w03 半份即使严重度 0 也只用 a；据此，有害属性组被删除时，保留的属性改取两棵试验树剪枝后所用属性的并集，筛除录制例子 32/32 一致，关闭 1 个已知 GAP。
- 2026-11-06（续十九）：C5.0 按“只借鉴思路”的两组隔离法：Codex 读 GPL 版 C5.0 2.07 写出不含代码的功能说明，实现方（协调者，未接触源码）据此重写全局剪枝（每减少一片叶子增加的错误数 d/(l−1)、单精度最优分、不相交平局组整体接受否则停止，关闭 `c50_global/wn100_mc3`），并加入生长期折回（子树错误 ≥ 叶子错误的 99.9% 时折回）与按说明实现的属性筛除（类别奇偶对半分、屏蔽属性的评估误差、一次整组复核、删除剪枝前未用属性）。筛除重现 `c50_presets`/`models_ml` 全部 20 棵录制树与 `c50_winnow2` 12 例中的 11 例（`w03` 不同），经用户决定，节点已启用筛除（`w03` 登记为已知 GAP，此前它只是借已保存模型匹配；筛除与规则集或提升组合仍为 GAP）；说明中“剪枝前无信息量检验”一条与 Modeler 不符（去掉我们的检验会损失 25 个流），保留原检验。
- 2026-11-06（续十八）：Apriori 等支持度项目的排序：两个新探针 `apriori_tie`（12 例）、`apriori_tie2`（11 例）改变相等项的个数、名字、列位置与共现块布局，由完整规则顺序穷举出每例可行的排名集合；Modeler 按（支持度，包含该项的频繁项集数）升序作带哨兵的插入排序（先把第一个最小者与首位交换，其余稳定插入），23 种布局与 `tie_equal`、`tie_twonew`、`trueflags_false` 全部一致，关闭 2 个已知 GAP。GLE：按指南字面“H(i) 在 i ≤ K 时用 Fisher”（默认混合法实为两步 Fisher）并去掉三处补丁，结果不变；新探针 `gle5` 显示 G 组每个 Modeler 估计都在我们路径上（混合法停在第一次 Newton 步之后），拒绝下一步的条件尚未确定（`gle5` 未登记）。
- 2026-11-06（续十六）：Time Series 拟合器扩展到 d ≤ 2、稀疏 AR/MA 滞后至 24 阶（Codex gpt-6.1-sol；完整 AR/MA 多项式根检验，新结构用 Durbin-Levinson 计算精确新息），原先超出范围的 8 个 ARIMA 专家选型可以拟合，目标函数不劣于 Modeler 保存参数、预测最大偏差 0.0021 个标准差；去掉保存容器后 85 个已复现的 ARIMA 选型全部经节点输出与 Modeler 结构一致，28 个“全部模型”录制流全部选对。
- 2026-11-06（续十五）：Time Series Expert“全部模型”自动选型：没有保存规格时分别拟合 ARIMA 专家与指数平滑专家，取原始尺度归一化 BIC = ln(SSE/(n−k)) + k ln(n)/n 较小者（`time_series::normalized_bic`，SSE 为原始尺度一步预测误差）；28 个“全部模型”录制流去掉保存容器后 26 个模型族与结构一致、0 个不一致，2 个因 d=2 拟合仍为 GAP。
- 2026-11-06（续十四）：移植 Codex 第 7b 轮的三条 ARIMA 专家规则（EACF 用 Bartlett 标准误；ML 剪枝 t 值取加权残差雅可比的联合协方差；Step 6 诊断用 Step 5 局部拟合的未加权残差），完整规格由 82/93 升到 85/93（`p2_ma2_ar`、`p2_ma2_all`、`p4_ma4_ar`），已吻合的无一退化；构建节点无保存规格时 68 个经节点输出与 Modeler 结构一致。
- 2026-11-06（续十三）：ARIMA 专家的 Step 4 定阶、Step 5 剪枝与 ML 重拟合、Step 6 Ljung-Box 修正移植为 `arima::expert_arima`（`crates/algo/src/expert.rs`，由 opencode hy4 子代理按 Python 参考移植），93 个录制选型中完整规格 82 个吻合（`ts_expert5` 全部、`ts_expert6` 16 例中 13 例）；仅 ARIMA 的 Expert 构建节点在没有保存规格时用它选型并拟合，去掉保存容器后 66 个经节点输出与 Modeler 结构一致，其余为全部模型（8 个，仍为 GAP）与拟合器不支持的结构（8 个：d=2 或滞后超过 4 阶）；未吻合的 11 个见 `docs/gaps.md`。`ts_expert6` 16 例并入全量回归。
- 2026-11-06（续十二）：ARIMA 专家的差分按指南附录 Step 3 实现（`arima::expert_differencing`，93 个记录 (d, D) 全部吻合，关键细节为步骤 (b) 条件最小二乘的样本前值取 0），取代经验季节差分规则；定阶、剪枝与诊断的 Python 参考实现已复现 51/67 个完整规格。
- 2026-11-06（续十一）：在算法指南附录（PDF 第 768–786 页）找到 Expert Modeler 的完整公开算法；ARIMA 专家的变换判定按其 Step 2 实现（`arima::expert_transform`），91 个 ARIMA 记录全部吻合（此前黑箱反推的所有规则都未能解释 off500/scale500、sqrthet、linehet）。
- 2026-11-06（续十）：GLE 二项 identity/log 越界的分界：均值 ≤ 0 时对数似然无定义、不做步长减半而取整步（Modeler 的 `gle3`/`gle4` D 组为非收敛 3 周期、混合法止于第 99 步、纯 Newton 止于第 100 步），均值 ≥ 1 仅排除、仍做步长减半（B 组收敛）；修正混合法下一步 Hessian 的类型判定差一与纯 Newton 首步；新探针 `gle4`（18 例）；三标签 49 流中 34 例精确一致，C 组（两侧越界、混沌路径）与对数链接深度越界仍为已知 GAP。
- 2026-11-06（续九）：GLE 二项 identity/log 链接：拟合均值越出 (0, 1) 的记录在该次迭代的似然、得分与 Hessian 中排除（Modeler 的估计正是剩余记录得分方程的根），脚本 `max_iterations`（存为 `_interEffMaxOuterIters`）不再当作 IRLS 迭代上限；新探针 `gle3`（14 例，起始均值 0–3 个越界）9 例一致，其余 5 例与原两例为 Modeler 在越界区中途停止的路径，仍为已知 GAP。
- 2026-11-06（续八）：ARIMA 专家的常数项已确定：无 AR/MA 项时保留，否则均值的 GLS t 值（ARMA 系数固定）|t| > 1.96 时保留，75 个 ARIMA 记录全部吻合（`arima::expert_keeps_constant`）。
- 2026-11-06（续七）：ARIMA 专家的季节差分 D 已确定：在未做非季节差分的（变换后）序列上一次季节差分使方差下降则 D = 1，16 个季节性 ARIMA 记录全部吻合（`arima::expert_seasonal_difference` 与语料测试）；d、阶数、剪枝、对数判定仍为 GAP。
- 2026-11-06（续六）：Time Series Expert 第五个探针 `ts_expert5`（12 例，卡对数变换门槛）：12 例全部取对数，推翻了“ln y 变异系数门槛 + 算术均值雅可比 Box-Cox 似然比”两段式规则（该规则拟合此前 61 个 ARIMA 记录）；极差-均值斜率同样无法区分，近似同源序列判定相反，指向两种尺度各自定阶后的完整似然比较。模型块打分 12 例全部一致。
- 2026-11-06（续五）：GLE 第一期（normal、binomial、Poisson、gamma，典范连接及 identity/log，主效应）：建模按混合 IRLS，模型块按 Modeler 保存的 PMML 打分；因子空白值按缺失、模型外类别打分为 $null$，计数目标的均值输出实数；`gle` 探针 17 例中 14 例一致，二项 identity/log 链接与带因子的 logit 收敛差异 3 例为已知 GAP，节点保持默认分布（Modeler 实际按逆高斯拟合）时建模为 GAP。
- 2026-11-06（续四）：E-Plot（Beta）节点：设置按 Modeler 保存的元素名读取（`eplot` 探针 16 例），记录选择与 Plot 同形输出；Modeler 批处理能运行该节点但五种格式都无法导出其输出（Unsupported for windowed rendering），故 oracle 只比对到达节点的记录，清单新增状态 `done (output unverified)`。Self-Learning Response Model：本机 Modeler 已装扩展但脚本报 unknown node type（与 Bayes Net、GenLin 相同），改为许可证受阻，未验证的实现不合并。
- 2026-11-06（续三）：Clementine 11 内联 `RuleTreeModel` C&R Tree 模型块映射到 PMML 树打分（`telco_dataaudit` 一致，移出已知 GAP）；流日期格式 `format27`（MON YYYY）的读入、`to_date` 与终端导出（`broadband_create_models` 的 Type 终端一致）。Time Series Expert 第四个探针 `ts_expert4`（24 例，由冻结的成对假设设计）及其 oracle，模型块打分全部一致；已确认 3s 季节门槛、全部模型按原始尺度归一化 BIC 比较、初始 MA 扫描可达稀疏第 5/7 阶，对数变换门槛不具尺度不变性，常数项按拟合决定；ARIMA 专家选型仍为 GAP。
- 2026-11-06（续二）：Time Series Expert 第三个判别探针 `ts_expert3`（30 例：平方根异方差、偏移 200/1000、线性异方差、近单位根 AR、无漂移 IMA、AR 第 2 阶、MA 第 1/3 阶、36/48 个月季节）及其 oracle，30 例模型块打分全部一致；平滑专家 10/10 吻合；ARIMA 专家确认：从不取平方根，取对数不只看最大/最小比，常数始终保留，36 个月即启用季节差分，近单位根序列取 MA(4)，选型规则仍为 GAP。
- 2026-11-06（续）：ARIMA 未来预测按有限历史做高斯条件化（新息递推延伸到预测期），季节平滑模型块支持非零末期相位；`ts_expert`、`ts_expert2` 60 例全部一致。Time Series Expert 限定指数平滑时自动选型：无变换、在简单/Holt/Brown/阻尼及（样本多于一个周期时）季节/Winters 加法/乘法中取归一化 BIC = ln(SSE/(n−k)) + k ln(n)/n 最小，22 个 Modeler 记录全部吻合；限定 ARIMA 与全部模型仍为 GAP。
- 2026-11-06：`cargo xtask regress` 不再把统计等价构建节点所在流里未产出的终端算作统计等价（只有全部终端都产出、仅数值不同才算），由此暴露的 4 个流（Time Series 季节/变换传递函数、Streaming TS、CPLEX、18 点季节平滑相位）登记为已知 GAP。Time Series Expert：两个判别探针 `ts_expert`、`ts_expert2`（各 30 例，平滑/ARIMA/全部模型）及其 oracle；模型块打分支持白噪声 ARIMA（无 `NonseasonalComponent`）与 d=2，两标签 60 例中 56 例一致；自动选择仍为 GAP：平滑专家按归一化 BIC 取最小在 22 个记录中吻合 19 个，ARIMA 专家尚未确定。
- 2026-11-05：输出节点写文件模式（Matrix、Means、Analysis、Statistics、Data Audit 的 .tab/.csv/.txt/.html 与 Modeler 逐字节或 1e-12 一致）、Matrix/Means 缺失值、Evaluation 成本/收益/权重字段，`outputs_more` 27 例中 21 例一致（.cou 与 Data Audit 图片 HTML 保留 GAP）。ADP 数值型名义/flag 输入原样保留（`ADP_basic_demo` 一致）；TS `_calculatePI`/`_isOutputOthers` 不影响打分（`catalog_forecast` 一致）；C5.0 winnowing 以修正后的学习器重做：20 个 oracle 均有精确子集，但最佳规则 15/20 会造成回归，`c50_winnow2` 探针最佳候选 9/12，仍为 GAP。
- 2026-10-29（续二）：记分卡（`docs/scorecard.md`）：1234 个带终端的流中 94.2% 全部跑通、96.5% 与 Modeler 行为一致（含同样报错）；每流中位数 0.010 s，对比 Modeler 批处理 7.2 s。由此补录 11 个 IBM 演示流的 oracle 并修复可变文件源 `multiBlankDelimiters`（无空白分隔符时）与 SAV `F w.0`/`DOLLAR w.0` 整数读入（四舍五入）；新探针 `outputs_more`（27 例，输出节点写文件模式、缺失值矩阵、Evaluation 成本字段）。
- 2026-10-29（续）：Time Series 30 例中 28 例达到统计等价（传递函数分母按指南的稳态初值实现；`ts_tf_static`、`ts_sarlog_noconst` 两例经独立精确似然确认为 Modeler 优化器提前停止）。Apriori 新探针 `apriori_eval`（17 例）：四种评价度量公式、模型块评分设置（检查方式、五种排序准则、`$AC-` 写准则值、不忽略未匹配项时前件须等于购物篮）、相连模型块按默认评分、置信度平局按前件长度与项 id、事务数据忽略字段列表；`apriori_more` 19 例中 17 例、`apriori_eval` 全部一致，剩等支持度项目排序 2 例。
- 2026-10-29：三个新探针：`adp_rest`（24 例）、`graphboard_more`（12 例）、`ts_more`（30 例）。Auto Data Prep 剩余步骤全部实现（字符串序数按名义重编码、几乎恒定筛除、显式字段、频率权重、目标异常值设缺失与组合步骤、含缺失的监督合并/分箱、分类目标与日期特征选择、日期 Min-Max、时长与时间元素并存、构造特征位置），ADP 71 个流全部一致；Modeler 本身拒绝分析权重。Graphboard 直方图分箱改为约 (32n)^(1/3) 箱并在增幅 <10% 时取整跨度、超过 2000 条仍分箱，箱线图四分位改为 R-3，颜色/分面/动画/坐标地图可用，12 例中 11 例一致（2500 条散点为随机抽样）。Time Series 新探针覆盖 Brown、阻尼、季节、Winters、变换、季节 ARIMA、传递函数和 Expert，实现进行中。
- 2026-10-28：C5.0 剪枝：只让正增益测试进入平均增益门槛、提升时只重生长被选中的分支、提升最大的内部子分支、对重生长后的候选继续做子树提升、全局剪枝在误差增量相同时先坍缩叶子多的子树、并先做信息量检验（切分的比特数低于 log2（正增益检验数）即剪掉，由 90 个探针流反推）；`c50_prune` 6 例与 16 个 Auto Classifier C5.0 组件全部逐节点一致，`c50_presets/c50_wn100_expert` 也已一致，平票组被拒后只再试其祖先子树（`wn89`）；剪枝置信度的正态分位数改为在 CF 0.1/0.2/0.4/0.5 表点间线性插值（`c50_conf` 探针 17 例反推），`mc3`/`mc5` 关闭全局剪枝时的局部树随之一致；剩 `c50_global/wn100_mc3`、`wn100_mc5` 两例（全局剪枝阶段）。Decision List 判别实验 21 例中 20 例完整一致。输出节点补上 flag/数值矩阵、成对 t 检验、利润/ROI 图。
- 2026-10-20：**MVP 85/85**。Excel 源节点（`excel_more` 证实空 `<row>` 元素是夹具问题、文本逗号读成句点）、XML 源、导入 PMML（回归与分类）。Auto Data Prep 按指南补齐异常值、min-max、目标 Box-Cox/缺失/异常值/重编码、监督与无监督合并（名义与序数）、监督分箱、特征选择、主成分特征构造、时间字段与日期年/自动单位（`adp_next`/`adp_corr`/`adp_more`/`adp_time` 共 43 例全部一致）。Decision List、Linear-AS、Sequence 用判别实验查到公开资料的尽头，结论记入文档；LSVM 停止点与 Cox（本机未装）确认不可复现。
- 2026-10-19：Time Series、Auto Cluster、Random Trees、Auto Classifier/Numeric；统计等价验收层（`docs/statistical-parity.md`）。
- 2026-10-18：PCA/Factor、Anomaly Detection（训练与打分）、Transform 输出、Apriori（表格与事务数据，规则与 oracle PMML 逐条一致）、Auto Data Prep（`misc_next` 三例）；Kohonen 仅打分。MVP 覆盖 65/85（许可证受阻的 5 个模型不计入）。
- 2026-10-15：**P1 完成**。`.str` 流文档是 Fast Infoset（ITU-T X.891）编码的 XML；按公开规范实现解码器 `dms-strfmt`（不 panic、无递归、迭代式析构），流模型 `dms-stream`（节点、连线、超节点、两种扩展标识），`dms-cli xml/inspect/coverage`；差分探针 `cargo xtask probe`；格式规格 `docs/formats/str.md`。
- 2026-10-15：`cargo xtask oracle` 打通：Modeler 本地批处理（`clemb`）无需 Server 许可、不弹界面；导出 PMML 和终端节点输入 CSV；跨机器数据路径按文件名重定位；druglearn 两次运行结果可复现（仅 PMML 时间戳不同）。
- 2026-10-15：建立仓库骨架、工作区 lints、`xtask`（ci / layers / parity / corpus）、146 个节点的目录（MVP 100 个）、语料清单 49 个文件。
