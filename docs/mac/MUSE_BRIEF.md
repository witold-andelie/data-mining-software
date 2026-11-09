# Muse 子代理任务：在 Mac 上运行验证命令并如实汇报

你在 `data-mining-software` 仓库的根目录。这个仓库是用 Rust 重写的数据挖掘工具。你的工作只有一件：按顺序运行下面五条命令，然后如实汇报结果。

```sh
bash docs/mac/muse-steps.sh ci
bash docs/mac/muse-steps.sh test-corpus
bash docs/mac/muse-steps.sh regress
bash docs/mac/muse-steps.sh compare
bash docs/mac/muse-steps.sh details
```

## 规则

- 只运行这五条命令，一字不改。其他 shell 命令都会被拒绝，也不要尝试。
- 不要修改、创建或删除任何文件。你没有编辑权限，也不需要。
- 某条命令失败（退出码不是 0）时，继续运行下一条。`compare` 在有差异时退出码为 1，这是正常的。
- 每条命令都可能运行很久（`regress` 可能超过一小时）。等它结束，不要中途放弃。
- 不要解释原因，不要下结论，不要说某个失败"可以忽略"或是"上游的怪癖"。分析由协调者负责。

## 汇报格式（你最后一条消息）

对每条命令：

1. 它最后一行 `muse-steps <step>: exit=… elapsed=…`，原样复制。
2. 如果退出码不是 0，从对应的 `target/mac/<step>.log` 里原样复制最先出现的错误（编译错误、失败的测试名和断言信息、`regress` 的汇总行），最多 40 行。不要改写，不要总结。
3. `regress` 的汇总行（以 `xtask regress:` 开头）原样复制。
4. `compare` 输出的前 30 行原样复制（`target/mac/mac-compare.txt`）。

最后列出 `target/mac/` 下你看到的文件名。
