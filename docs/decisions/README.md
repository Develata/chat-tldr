# 架构决策记录（ADR）

> 用途：用简短的记录说明关键决策“为什么这样定”，避免反复争论。读者：全体成员，写报告的人。
> 格式：背景 → 决策 → 理由 → 代价。推翻一条 ADR 需要新写一条，并在旧的一条里标注“已被 ADR-XXXX 取代”。

| 编号 | 标题 | 状态 |
|---|---|---|
| [0001](0001-export-only-qce.md) | 只读取 QCE 导出文件，不内嵌 QCE | 已接受 |
| [0002](0002-rust-full-stack.md) | 全栈 Rust | 已接受 |
| [0003](0003-cli-gui-subprocess.md) | CLI 拥有全部逻辑，GUI 通过子进程调用 | 已接受 |
| [0004](0004-hybrid-topic-segmentation.md) | 混合式话题切分，embedding 可选 | 已接受 |
| [0005](0005-model-roles.md) | Jev + LLM + embedding 分工，统一 Decider 抽象 | 已接受 |
| [0006](0006-evidence-verification.md) | 证据校验不变量：LLM 提出，Rust 验证 | 已接受 |
| [0007](0007-cursors-and-lifecycle.md) | 三游标与结论生命周期 | 已接受 |
| [0008](0008-llm-client.md) | 自写 LLM 客户端，支持 OpenAI / Anthropic 两种格式，同步 IO | 已接受 |
| [0009](0009-bounded-agent-controller.md) | 规则限定 + Jev 选择的有界控制器 | 已接受 |
| [0010](0010-semantic-relations.md) | 独立语义关系表与只读查询 | 已接受 |
| [0011](0011-provider-settings.md) | CLI/GUI 模型设置与本地明文凭据 | 已接受 |
