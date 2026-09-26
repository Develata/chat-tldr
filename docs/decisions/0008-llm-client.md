# ADR-0008 自写 LLM 客户端，支持 OpenAI / Anthropic 两种格式，同步 IO

**背景**：最初建议使用 `async-openai` + `schemars`。之后确定：不预设 LLM provider，由用户配置 `base_url`、`api_key` 以及接口格式，格式可以是 OpenAI 兼容或 Anthropic 兼容。并非所有兼容接口都支持严格的 `json_schema` 输出。

**决策**（已接受，见 OPEN_QUESTIONS Q-DEC-1；客户端仍按主线计划实现）
- 不使用 `async-openai`。用 `reqwest` 自写两个最小客户端：`OpenAiCompatClient`（`POST {base_url}/chat/completions`）和 `AnthropicCompatClient`（`POST {base_url}/v1/messages`），都实现 `LlmClient` trait。Jev 与 embedding 也用同一个 `reqwest` 客户端。
- 输出约束统一处理：用 `schemars` 生成 schema 写进提示词，用 `serde` 校验，失败重试一次。
- 使用**同步 IO**（`reqwest::blocking`），不引入 async/tokio。

**理由**
- 一个 trait、两个各约 100 行的实现，比引入一个只覆盖 OpenAI 格式的大依赖、再为 Anthropic 另写一套更简单一致。
- 请求只用到 messages、temperature、max_tokens 和 usage 几个字段，自写的线类型很小，而且完全可控。
- CLI 是批处理工具，瓶颈在远程服务的延迟，而不是本地并发。同步代码对团队更友好：没有 `Send` 约束、没有 async trait、没有运行时配置。以后如果需要并行执行 AnalyzeTopic，可以用 `std::thread::scope` 实现有界并发。

**代价**
- 流式输出、工具调用等高级特性需要自己实现（本项目用不到）。
- 以后如果要大规模并发，可能需要迁移到 async。
