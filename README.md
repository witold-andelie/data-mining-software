# data-mining-software

用 Rust 重写的轻量版数据挖掘工具，对标 IBM SPSS Modeler 19 的单机功能：可视化建流、数据准备、建模、评估、导出，能导入现有的 `.str` 流文件。目标平台是 Windows 和 Apple 芯片 macOS。

团队内部项目，暂不对外发布。

## 状态

阶段 P0（骨架）。节点覆盖率见 [`docs/parity-checklist.md`](docs/parity-checklist.md)，路线见 [`ROADMAP.md`](ROADMAP.md)。

## 开发

需要 Rust 1.95（`rust-toolchain.toml` 会自动选择）。

```sh
cargo xtask ci        # 提交前的全部检查
cargo xtask parity    # 重新生成节点对标清单
cargo xtask corpus verify
```

规则见 [`AGENTS.md`](AGENTS.md)，架构见 [`docs/architecture.md`](docs/architecture.md)。
