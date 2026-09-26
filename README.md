# 群聊省流 / chat-tldr

> Turn unread QQ group chats into a personal, traceable action inbox.
> 把未读群聊变成可追溯的个人行动收件箱。

**状态 / Status**：共享基础已实现：Rust workspace、协议类型、合成样例、配置检查、QCE JSON 导入和会话/消息查询。模型分析、收件箱操作、GUI 与指标评估仍待实现。分工见 [TEAM_ASSIGNMENTS](docs/TEAM_ASSIGNMENTS.md)。
*Shared foundations and offline import/query commands are implemented. Model analysis, inbox actions, GUI and evaluation metrics are still pending.*

---

## 中文

### 这是什么

几十分钟到几天没看 QQ 群，积压了几百到几千条消息。chat-tldr 帮你快速知道：**我漏掉了什么、我要做什么、什么时候做、证据在哪**。

它和“把聊天记录直接丢给 LLM 总结”的区别：

1. **持续维护状态**：增量处理，只分析上次之后的新消息，话题状态一直保留。
2. **面向个人**：优先展示 @我、分配给我的任务、截止日期、重要通知，而不是流水账。
3. **可追溯**：每条结论都附带原始消息证据，由程序逐字校验证据真实存在，而不是相信 LLM。
4. **个性化**：根据你的反馈调整排序，但“必须处理”的事项永远不会被降级。

### 目标功能（除导入与查询外仍在开发）

- 导入 [QQChatExporter](https://github.com/shuakami/qq-chat-exporter) 导出的 JSON，幂等去重
- 混合式话题拆分：回复、@、时间间隔等规则 + [Jev](https://docs.typesafe.ai) 结构化决策模型
- 结构化抽取：@我、待办、截止日期、通知、决策、话题摘要
- 证据校验：LLM 提出，Rust 验证
- P0–P3 分层收件箱 + 有界的反馈校准
- 有界智能体控制器，每一步决策都有日志
- Rust CLI（全部逻辑）+ egui 桌面界面

### 现在就能运行（无需密钥）

在仓库根目录的 PowerShell 中运行：

```powershell
cargo run -p chat-tldr -- version
cargo run -p chat-tldr -- --data-dir ./private/demo config init
cargo run -p chat-tldr -- --data-dir ./private/demo import ./fixtures/qce/synthetic-group.json
cargo run -p chat-tldr -- --data-dir ./private/demo chats
cargo run -p chat-tldr -- --data-dir ./private/demo messages --chat qq:group:synthetic-study
cargo run -p chat-tldr-eval -- check-stream ./fixtures/jsonl/inbox.jsonl
```

主 CLI 的 stdout 为 JSONL，`version` 的 `capabilities` 只列出已实现命令。`config init` 不覆盖已有配置；`doctor` 只做离线检查，不联系云服务，缺 LLM key 时返回 4，但仍说明导入/查询是否可用。这些命令的数据库和配置都留在被 Git 忽略的 `private/demo/`。

`fixtures/` 全为人工合成数据；适配器已按上游字段与合成样例测试，真实 QCE 导出仍需实测。GUI 当前明确提示尚未实现并退出，不显示成功假象。

### 后续完整使用流程（尚未实现）

```bash
# 1. 配置密钥（只放在环境变量中，不写进任何文件）
#    PowerShell 写法：$env:TYPESAFE_API_KEY = "..."
export TYPESAFE_API_KEY=...          # Jev
export CHAT_TLDR_LLM_API_KEY=...     # DeepSeek（也可换成任意 OpenAI / Anthropic 兼容的 LLM）

# 2. 导入 QCE 导出的群聊 JSON（可重复导入，结果不变）
chat-tldr import ./exports/my-group.json

# 3. 分析自上次以来的新消息
chat-tldr chats
chat-tldr analyze --chat qq:group:<id>

# 4. 查看收件箱（JSON Lines；或生成 HTML）
chat-tldr inbox --chat qq:group:<id> --html inbox.html

# 或者直接打开图形界面
chat-tldr-gui
```

LLM 的 `base_url`、模型名和接口格式（`openai` / `anthropic`）在 `config.toml` 中配置，见 [docs/CLI_PROTOCOL.md](docs/CLI_PROTOCOL.md) §7。

### 隐私

程序和数据库都在本机，但聊天文本会发送给 Jev（TypeSafe）和你配置的 LLM（默认 DeepSeek）两个云服务，不做脱敏；图片不上传。仓库中不包含任何真实聊天记录或密钥。

### 文档

| 文档 | 内容 |
|---|---|
| [TEAM_ASSIGNMENTS](docs/TEAM_ASSIGNMENTS.md) | 当前分工、目录范围、接入接口与交付验收 |
| [FILE_LAYOUT](docs/FILE_LAYOUT.md) | 源码、运行数据、QCE 管理、评估材料的存放与读写边界 |
| [ARCHITECTURE](docs/ARCHITECTURE.md) | 架构、模块边界、设计不变量 |
| [DATA_MODEL](docs/DATA_MODEL.md) | 冻结的共享类型与数据库表结构 |
| [CLI_PROTOCOL](docs/CLI_PROTOCOL.md) | 命令、JSONL 事件、错误码 |
| [CLI_V1_REVIEW](docs/CLI_V1_REVIEW.md) | 已采纳的 CLI v1 调整记录 |
| [PIPELINE](docs/PIPELINE.md) | 切分、抽取、校验、排序、智能体控制器 |
| [EVALUATION](docs/EVALUATION.md) | 基线、指标、标注规范 |
| [ROADMAP](docs/ROADMAP.md) | 3 天计划与降级预案 |
| [decisions/](docs/decisions/) | 架构决策记录 |
| [OPEN_QUESTIONS](docs/OPEN_QUESTIONS.md) | 待定问题 |
| [CONTRIBUTING](CONTRIBUTING.md) | 协作流程（写给第一次用 Git 的同学） |

### 团队分工

| 成员 | 负责 |
|---|---|
| @Develata | 参与设计、审核架构与代码 |
| Codex | 主线编码，包含 QCE JSON 导入、core、engine、CLI、GUI 接入与评估工具 |
| 同学协作 | A：QCE 管理；B：GUI 设计；C：合成场景与验收材料。详见 TEAM_ASSIGNMENTS，席位尚未绑定真实账号 |

---

## English

### What it is

You've been away from a QQ group for an hour or a few days, and hundreds to thousands of messages have piled up. chat-tldr tells you **what you missed, what you need to do, by when, and where the evidence is**.

How it differs from pasting the chat into a general-purpose LLM:

1. **Stateful**: incremental processing. Only messages since the last run are analyzed, and topic state persists across runs.
2. **Personal**: @-mentions of you, tasks assigned to you, deadlines and announcements come first, not a play-by-play recap.
3. **Traceable**: every conclusion cites the original messages, and the program checks that each quote really exists, word for word, instead of trusting the LLM.
4. **Adaptive**: your feedback reorders items, but must-handle items are never demoted.

### Planned features (offline import and queries are implemented)

- Imports JSON exported by [QQChatExporter](https://github.com/shuakami/qq-chat-exporter); idempotent deduplication
- Hybrid topic disentanglement: reply, mention and time-gap rules plus the [Jev](https://docs.typesafe.ai) structured-decision model
- Structured extraction: mentions of you, to-dos, deadlines, announcements, decisions, topic summaries
- Evidence verification: the LLM proposes, Rust verifies
- P0–P3 tiered inbox with bounded feedback calibration
- A bounded agent controller that logs every decision
- A Rust CLI that owns all the logic, plus an egui desktop GUI

### Usage (draft)

See the Chinese section above. The commands are identical.

### Privacy

The program and its database run locally, but chat text is sent unredacted to two cloud services: Jev (TypeSafe) and the LLM you configure (DeepSeek by default). Images are never uploaded. The repository contains no real chat logs and no keys.

### License

MIT. chat-tldr only reads files exported by QQChatExporter (GPL-3.0). It does not link or include any of QQChatExporter's code.
