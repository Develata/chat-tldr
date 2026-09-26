# 群聊省流 / chat-tldr

> Turn unread QQ group chats into a personal, traceable action inbox.
> 把未读群聊变成可追溯的个人行动收件箱。

**状态 / Status**：设计阶段，还没有可运行的代码。文档即开发依据，见 [docs/](docs/)。
*Design stage — no runnable code yet. The docs under [docs/](docs/) are the build spec.*

---

## 中文

### 这是什么

几十分钟到几天没看 QQ 群，积压了几百到几千条消息。chat-tldr 帮你快速知道：**我漏掉了什么、我要做什么、什么时候做、证据在哪**。

它和“把聊天记录直接丢给 LLM 总结”的区别：

1. **持续维护状态**：增量处理，只分析上次之后的新消息，话题状态一直保留。
2. **面向个人**：优先展示 @我、分配给我的任务、截止日期、重要通知，而不是流水账。
3. **可追溯**：每条结论都附带原始消息证据，由程序逐字校验证据真实存在，而不是相信 LLM。
4. **个性化**：根据你的反馈调整排序，但“必须处理”的事项永远不会被降级。

### 核心特性

- 导入 [QQChatExporter](https://github.com/shuakami/qq-chat-exporter) 导出的 JSON，幂等去重
- 混合式话题拆分：回复、@、时间间隔等规则 + [Jev](https://docs.typesafe.ai) 结构化决策模型
- 结构化抽取：@我、待办、截止日期、通知、决策、话题摘要
- 证据校验：LLM 提出，Rust 验证
- P0–P3 分层收件箱 + 有界的反馈校准
- 有界智能体控制器，每一步决策都有日志
- Rust CLI（全部逻辑）+ egui 桌面界面

### 使用方式（草案）

```bash
# 1. 配置密钥（只放在环境变量中，不写进任何文件）
#    PowerShell 写法：$env:TYPESAFE_API_KEY = "..."
export TYPESAFE_API_KEY=...          # Jev
export CHAT_TLDR_LLM_API_KEY=...     # 任意 OpenAI 兼容或 Anthropic 兼容的 LLM

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

聊天内容会发送给 Jev 和你配置的 LLM 两个云服务。默认开启脱敏：昵称、群号替换为代号，图片不上传。仓库中不包含任何真实聊天记录或密钥。

### 文档

| 文档 | 内容 |
|---|---|
| [ARCHITECTURE](docs/ARCHITECTURE.md) | 架构、模块边界、设计不变量 |
| [DATA_MODEL](docs/DATA_MODEL.md) | 冻结的共享类型与数据库表结构 |
| [CLI_PROTOCOL](docs/CLI_PROTOCOL.md) | 命令、JSONL 事件、错误码 |
| [PIPELINE](docs/PIPELINE.md) | 切分、抽取、校验、排序、智能体控制器 |
| [EVALUATION](docs/EVALUATION.md) | 基线、指标、标注规范 |
| [ROADMAP](docs/ROADMAP.md) | 3 天计划与降级预案 |
| [decisions/](docs/decisions/) | 架构决策记录 |
| [OPEN_QUESTIONS](docs/OPEN_QUESTIONS.md) | 待定问题 |
| [CONTRIBUTING](CONTRIBUTING.md) | 协作流程（写给第一次用 Git 的同学） |

### 团队分工

| 成员 | 负责 |
|---|---|
| @Develata | 核心设计与实现（core、engine、cli）、CI、审核所有 PR |
| 同学 A | QCE 适配器（`crates/qce`）、截止日期规范化、证据校验 |
| 同学 B | 桌面界面（`apps/gui`） |
| 同学 C | 评估（`eval/`）、演示数据、README 与报告、演示视频 |

---

## English

### What it is

You've been away from a QQ group for an hour or a few days, and hundreds to thousands of messages have piled up. chat-tldr tells you **what you missed, what you need to do, by when, and where the evidence is**.

How it differs from pasting the chat into a general-purpose LLM:

1. **Stateful**: incremental processing. Only messages since the last run are analyzed, and topic state persists across runs.
2. **Personal**: @-mentions of you, tasks assigned to you, deadlines and announcements come first, not a play-by-play recap.
3. **Traceable**: every conclusion cites the original messages, and the program checks that each quote really exists, word for word, instead of trusting the LLM.
4. **Adaptive**: your feedback reorders items, but must-handle items are never demoted.

### Features

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

Chat content is sent to two cloud services: Jev and the LLM you configure. Redaction is on by default: nicknames and group numbers are replaced with codes, and images are never uploaded. The repository contains no real chat logs and no keys.

### License

MIT. chat-tldr only reads files exported by QQChatExporter (GPL-3.0). It does not link or include any of QQChatExporter's code.
