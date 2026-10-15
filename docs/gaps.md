# 待办（按优先级排序）

> 更新：2026-10-15

1. **`.str` 格式规格**：用差分观察法写 `docs/formats/str.md`（容器、二进制 XML 的标记和长度编码、`data/*.dat`）。
2. **`dms-strfmt` 解码器 + `dms-cli inspect`**：解码全部语料，输出每个流的节点和参数，算出覆盖率。
3. **节点目录的未定映射**：
   - `random_trees`、`genlin`、`svm`、`knn`、`bayes_net`、`discriminant` 等节点在 `.str` 里的标识还没确认。
   - 团队流里出现的 `BuildPredictiveModelNode`、`BuildTreeNode`、`BuildModelNode`、`CFBuildModelNode`、`CFApplyModelNode` 可能是基类或通用包装，解码后再确定归属。
4. **在 Mac 上跑通 `cargo xtask ci`**（P0 完成标准，排在本机工作之后）。
5. **Python 桥设计**：Python 环境检测、Arrow 交换格式、模型持久化、错误和取消。
6. **CI**：私有仓库用 GitHub Actions 要消耗分钟数（macOS 机器按 10 倍计）。目前做法是本地 Windows 和 Mac 各跑 `cargo xtask ci`；需要时再加 Linux/Windows CI。
7. **oracle 覆盖面**：超节点内部的节点；分析、数据审核、统计等输出节点本身的结果（目前只导出它们的输入数据）。
