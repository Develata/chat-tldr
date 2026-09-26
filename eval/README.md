# 评估工具与数据

本目录同时容纳 `chat-tldr-eval` crate 和评估资料，以子目录隔开：

- `src/`、`tests/`：Rust 源码与测试。
- `synthetic/<dataset-id>/`：可提交的合成聊天、gold 和来源说明。
- `private/data/`、`private/gold/`、`private/consent/`：真实导出、真实标注与同意记录，全部忽略。
- `private/runs/<experiment-id>/<system>/profile/`：每个实验、每个系统独立的 CLI 数据目录。
- `private/runs/<experiment-id>/<system>/*.jsonl`：供评分工具读取的事件流。
- `private/results/<experiment-id>/`：指标和图表，审核后才把聚合材料复制到仓库 `reports/`。

`--data-dir` 指向 `profile/`；评分工具的 `--run` 指向它的上一层，即包含 `analyze.jsonl`、`inbox.jsonl`、`messages.jsonl` 和 `jev.jsonl` 的目录。

算法/标注规范见 [EVALUATION.md](../docs/EVALUATION.md)，完整目录树见 [FILE_LAYOUT.md](../docs/FILE_LAYOUT.md)。

## 已实现：事件流检查

```powershell
cargo run -p chat-tldr-eval -- check-stream fixtures/jsonl/version.jsonl
cargo run -p chat-tldr-eval -- check-stream fixtures/jsonl/analyze-partial.jsonl --exit-code 6
```

`check-stream <PATH> [--exit-code N]` 离线逐行读取 UTF-8 JSONL，复用 `core::EventStreamValidator` 检查协议版本、事件结构、连续序号、同一 run_id、唯一末尾 `done` 和退出码。未知事件类型按核心协议兼容规则接受；已知事件的错误结构、空行、截断或 `done` 之后的事件均失败。

不传 `--exit-code` 时，以文件里 `done.exit_code` 自检，只能证明文件内部一致，**不能证明真实子进程退出码**。传入该参数后才检查它与 `done.exit_code` 是否一致；调用方应保存实际退出码。合法的失败/部分完成运行也可以通过检查，因为这里验证协议完整性，不判断分析是否成功。

stdout 恰好输出一条 JSON 诊断摘要（`valid`、成功接受的 `events` 数量、`run_id`、`done_exit_code`、`process_exit_code`、`exit_code_source`、`error`），**这不是主 CLI 的 CliEvent 协议**。错误详情同时写 stderr。检查通过退出 0，文件/协议错误退出 1，命令用法错误退出 2；`--help` 和 `--version` 输出文本。

`score`、评估指标计算和实验执行尚未实现，未知子命令会报错。
