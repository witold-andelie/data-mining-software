# data-mining-software

用 Rust 重写的轻量版数据挖掘工具，对标 IBM SPSS Modeler 19 的单机功能：可视化建流、数据准备、建模、评估、导出，能导入现有的 `.str` 流文件。目标平台是 Windows 和 Apple 芯片 macOS。

团队内部项目，暂不对外发布。

## 自研的建模底座：自己训练，不是逆向

本项目的建模算法全部由我们自己实现、自己训练。它们**不是**对 IBM SPSS Modeler 原有模块的逆向工程，也没有复用 Modeler 的任何代码：

- **不碰 Modeler 内部**：从不反编译、反汇编或打开 Modeler 的 jar、DLL、exe。C5.0 是 RuleQuest 的专有算法，我们也没有阅读或参考过 C5.0/See5 的源代码。
- **只依据公开资料实现**：IBM 公开发布的 *Algorithms Guide*（算法公式）、用户手册、*Python Scripting and Automation Guide*（节点属性），以及 PMML 标准和公开文献。每个算法模块的文档注释都写明了所依据的章节和页码。
- **Modeler 只当黑盒对照**：我们构造输入数据，让 Modeler 在批处理模式下训练并打分，再把它的输出当作标准答案（oracle），和我们的输出逐格比对。规则未写进文档的地方，也只靠这种"改一个输入、看输出怎么变"的探针实验确定，不看程序内部。

训练出的模型分两档对标（详见 [`docs/statistical-parity.md`](docs/statistical-parity.md)）：

| 档次 | 含义 | 模型 |
|---|---|---|
| 精确 | 训练出的模型和打分结果与 Modeler 逐位一致 | C5.0、CHAID、C&R Tree 与 QUEST（不设过拟合防止集）、Tree-AS、Linear、Linear-AS、Regression、Logistic、Decision List、K-Means、TwoStep、Apriori、CARMA、PCA/Factor、Anomaly、Feature Selection、RBF 神经网络 |
| 统计 | Modeler 用了不公开的随机数发生器或优化器停止规则，无法逐位复现；我们实现文档里的算法，用自己的可设种子发生器或收敛的优化器训练，并检验 Modeler 的结果落在我们多次运行的统计范围内 | MLP 神经网络、Kohonen、LSVM、Random Trees、Time Series、Sequence、Auto Cluster，以及带过拟合防止集的 C&R Tree / QUEST |

Auto Classifier 和 Auto Numeric 的训练正在实现中。

## 状态

节点覆盖率见 [`docs/parity-checklist.md`](docs/parity-checklist.md)，路线见 [`ROADMAP.md`](ROADMAP.md)。

## 开发

需要 Rust 1.95（`rust-toolchain.toml` 会自动选择）。

```sh
cargo xtask ci        # 提交前的全部检查
cargo xtask parity    # 重新生成节点对标清单
cargo xtask corpus verify
```

规则见 [`AGENTS.md`](AGENTS.md)，架构见 [`docs/architecture.md`](docs/architecture.md)。
