# 群聊省流 / chat-tldr

> Turn unread QQ group chats into a personal, traceable action inbox.
> 把未读群聊变成可追溯的个人行动收件箱。

**状态 / Status**：CLI 基础闭环、HTML 导出及历史统计/日志查询已实现；eval 已有标注表格导出、导入和协议校验。原生 GUI 已接入 CLI，Windows 合成数据联调、浅色/深色/窄窗口截图及交互测试通过，见 [GUI 验证记录](docs/GUI_VERIFICATION.md)。当前 `ours` 仍是基础策略，指标评分和真实模型质量验收尚未完成。分工见 [TEAM_ASSIGNMENTS](docs/TEAM_ASSIGNMENTS.md)。
*The basic CLI workflow, HTML export, history queries and native GUI are implemented. Windows synthetic-data smoke checks and GUI interaction tests pass. Evaluation supports annotation-sheet conversion and stream validation; scoring and real-model acceptance remain pending.*

---

## 中文

### 这是什么

几十分钟到几天没看 QQ 群，积压了几百到几千条消息。chat-tldr 帮你快速知道：**我漏掉了什么、我要做什么、什么时候做、证据在哪**。

它和“把聊天记录直接丢给 LLM 总结”的区别：

1. **持续维护状态**：增量处理尚未完成的消息，包括后续导入的旧消息；话题状态和已提交的检查点保存在本地。
2. **面向个人**：优先展示 @我、分配给我的任务、截止日期、重要通知，而不是流水账。
3. **可追溯**：每条结论都附带原始消息证据，由程序逐字校验证据真实存在，而不是相信 LLM。
4. **个性化**：根据你的反馈调整排序，但“必须处理”的事项永远不会被降级。

### 当前已实现

- 导入 [QQChatExporter](https://github.com/shuakami/qq-chat-exporter) 导出的 JSON，幂等去重
- 基础话题处理：小批量直接抽取，或按时间、发送者和回复规则形成消息组，再由 [Jev](https://docs.typesafe.ai) 或 LLM 判断归属
- 结构化抽取：@我、待办、截止日期、通知、决策、话题摘要
- 证据校验：LLM 提出，Rust 验证
- P0–P3 分层收件箱 + 有界的反馈校准
- 基于规则选择动作的控制器，具有步数上限、费用预估、取消检查和决策日志
- 本地 SQLite 检查点、模型响应缓存与用量记录；部分失败后可继续处理未完成消息
- 同步 Jev 和 DeepSeek 客户端、OpenAI / Anthropic 兼容接口与 Mock；模型协议和流程用合成数据、Mock 及本地假服务器验证
- Rust CLI 的 `analyze/inbox/feedback/resolve/mark-read`，以及 `analyze --html` / `inbox --html` 导出
- 只读 `stats`、`decisions --run`、`jev-log --run`：查询累计用量或历史运行，重放决策与带归属的模型回答
- eval 的 `check-stream`、`export-sheet`、`import-sheet`：协议检查与人工标注 CSV 往返，不计算模型效果分数

尚未实现 `MergeTopics`、控制器用 Jev 选择下一步动作、embedding 候选筛选，以及 eval 的 `score/calibrate/summarize` 和基线比较。当前仅支持 `--strategy ours`，不代表 [PIPELINE](docs/PIPELINE.md) 的全部策略已经完成；真实 QCE 导出与真实云模型效果仍待验收。

### 现在就能运行（无需密钥）

在仓库根目录的 PowerShell 中运行：

```powershell
cargo run -p chat-tldr -- version
cargo run -p chat-tldr -- --data-dir ./private/demo config init
cargo run -p chat-tldr -- --data-dir ./private/demo import ./fixtures/qce/synthetic-group.json
cargo run -p chat-tldr -- --data-dir ./private/demo chats
cargo run -p chat-tldr -- --data-dir ./private/demo messages --chat qq:group:synthetic-study
cargo run -p chat-tldr -- --data-dir ./private/demo analyze --chat qq:group:synthetic-study --dry-run
cargo run -p chat-tldr -- --data-dir ./private/demo inbox --chat qq:group:synthetic-study --html ./private/demo/inbox.html
cargo run -p chat-tldr -- --data-dir ./private/demo stats
cargo run -p chat-tldr-eval -- check-stream ./fixtures/jsonl/inbox.jsonl
```

主 CLI 的 stdout 为 JSONL，`version` 的 `capabilities` 只列出已实现命令。`--dry-run` 只报告计划与密钥就绪情况，不联网、不写数据库或缓存；上述流程尚未执行模型分析，收件箱为空是正常结果。`config init` 不覆盖已有配置；`doctor` 只做离线检查，不联系云服务，缺 LLM key 时返回 4，但仍说明导入/查询是否可用。示例中的数据库、配置和 HTML 都留在被 Git 忽略的 `private/demo/`。

`fixtures/` 全为人工合成数据；适配器已按上游字段与合成样例测试，真实 QCE 导出仍需实测。CSV 需独立人工填写，再用 `import-sheet <CSV> --out <新目录>` 生成 gold；列格式、UTF-8 与不覆盖规则见 [eval 使用说明](eval/README.md)。这一步不生成评分。

### 启动原生 GUI

在仓库根目录编译；Windows 调试程序位于 `target/debug/`：

```powershell
cargo build --workspace
.\target\debug\chat-tldr-gui.exe --demo

# 正常模式：与 CLI 共用一个数据目录；GUI 默认寻找同目录的 chat-tldr.exe
.\target\debug\chat-tldr-gui.exe --cli .\target\debug\chat-tldr.exe --data-dir .\private\my-chat
# 可额外传 --config C:\path\config.toml；不传则使用 <data-dir>/config.toml
```

`--demo` 仅展示合成内容，不启动 CLI、不写 GUI 偏好。正常模式通过 CLI 子进程执行导入、分析、查询和状态操作；只有完整且已展示的收件箱才能标为已读。详细使用边界见 [GUI 使用说明](apps/gui/README.md)。

使用自己的聊天前，先在 CLI 初始化配置并写入自己的 QQ 身份；`--self-uin` 是你本人的 QQ 号，`--self-uid` 是可选的 QQNT UID：

```powershell
.\target\debug\chat-tldr.exe --data-dir .\private\my-chat config init
.\target\debug\chat-tldr.exe --data-dir .\private\my-chat import "C:\path\group.json" --self-uin "<你的QQ号>"
```

GUI 的导入只选择已完成的导出文件；GUI 不填写密钥或 QQ 身份。密钥按下节设置在启动 GUI 的进程环境中；首次云端分析需确认聊天原文会发送给配置的服务。仅打开 GUI、导入和查询不调用模型。`doctor` 检查本地配置和环境变量是否就绪，不验证网络连通性。

### 使用真实模型

下面的 `analyze` 会把聊天原文发送到配置的云服务。默认使用 Jev 做分类与话题归属、DeepSeek 做抽取；LLM 密钥必需，Jev 密钥缺失或服务不可用时会警告并改用 LLM。也可显式传入 `--decider llm`。密钥只从环境变量读取。

```powershell
$env:TYPESAFE_API_KEY = "<你的 Jev 密钥>"
$env:CHAT_TLDR_LLM_API_KEY = "<你的 DeepSeek 密钥>"

# 沿用上面的合成示例；实际导出也可用 import 导入，再从 chats 取得会话 ID
$chatId = "qq:group:synthetic-study"
cargo run -p chat-tldr -- --data-dir ./private/demo analyze --chat $chatId --max-steps 64 --budget-usd 0.50
cargo run -p chat-tldr -- --data-dir ./private/demo inbox --chat $chatId --html ./private/demo/inbox.html

# 将占位值换成 inbox 返回的 insight.id 和非 null 的 view_cursor
cargo run -p chat-tldr -- --data-dir ./private/demo feedback "<INSIGHT_ID>" --useful
cargo run -p chat-tldr -- --data-dir ./private/demo resolve "<INSIGHT_ID>" --done
cargo run -p chat-tldr -- --data-dir ./private/demo mark-read --chat $chatId --up-to "<VIEW_CURSOR>"

# RUN_ID 来自那次 analyze 的事件信封；这里的新命令有各自的 run_id
cargo run -p chat-tldr -- --data-dir ./private/demo stats --run "<RUN_ID>"
cargo run -p chat-tldr -- --data-dir ./private/demo decisions --run "<RUN_ID>"
cargo run -p chat-tldr -- --data-dir ./private/demo jev-log --run "<RUN_ID>"
```

反馈只调整同一优先级内的排序，处理状态与已读游标分别维护；查看或分析不会自动标为已读。HTML 可直接用浏览器打开，不需要 GUI。预算按配置单价保守估算，不是服务端账单的硬上限。

历史查询无需密钥。`stats --chat <ID>` 是该会话的累计统计，`--run` 是已保存分析运行的统计。`calls` 统计逻辑模型调用，包含失败调用但不展开内部 HTTP 重试；token/费用仅记录服务响应已报告的用量，缓存命中不重复计费，不能当作账单对账。旧记录缺少精确统计或 subject 时会告警；`unknown/unavailable` 的回答不能用于校准对齐。空工作分析和 dry-run 不写运行历史。

LLM 的 `base_url`、模型名和接口格式（`openai` / `anthropic`）在 `config.toml` 中配置；默认模板见 [config.example.toml](config.example.toml)，完整参数见 [CLI_PROTOCOL](docs/CLI_PROTOCOL.md) §7。

### 从 QCE Docker 导出

输入是 QCE 生成的完整单文件 JSON。使用 QCE 界面选定群和时间范围，格式选 JSON，关闭“流式导出”，等任务完成后下载到本机，再把本机文件路径交给 `import`。分块 JSONL 和 NapCat 原始消息不是当前输入格式。

导出步骤及已核对的上游字段见 [QCE_DOCKER_EXPORT](docs/QCE_DOCKER_EXPORT.md)。可先用 [fixtures/qce/template-docker-export.json](fixtures/qce/template-docker-export.json) 联调；它是按上游源码手写的 7 条合成消息，不能作为真实容器导出成功的证明。真实导出只保存在 `private/` 或数据目录，勿提交到仓库。

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
| [ANALYSIS_EXECUTION](docs/ANALYSIS_EXECUTION.md) | 分析执行、检查点、恢复与预算边界 |
| [QCE_DOCKER_EXPORT](docs/QCE_DOCKER_EXPORT.md) | 单文件 JSON 导出步骤与合成模板依据 |
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

1. **Stateful**: incrementally processes unfinished messages, including older messages imported later. Topic state and committed checkpoints persist locally.
2. **Personal**: @-mentions of you, tasks assigned to you, deadlines and announcements come first, not a play-by-play recap.
3. **Traceable**: every conclusion cites the original messages, and the program checks that each quote really exists, word for word, instead of trusting the LLM.
4. **Adaptive**: your feedback reorders items, but must-handle items are never demoted.

### Current scope

- Imports JSON exported by [QQChatExporter](https://github.com/shuakami/qq-chat-exporter); idempotent deduplication
- Basic topic assignment using time, sender and reply rules plus the [Jev](https://docs.typesafe.ai) structured-decision model or LLM fallback
- Structured extraction: mentions of you, to-dos, deadlines, announcements, decisions, topic summaries
- Evidence verification: the LLM proposes, Rust verifies
- P0–P3 tiered inbox with bounded feedback calibration
- A bounded controller with rule-based action selection, SQLite checkpoints, response caching and usage records
- Synchronous Jev / DeepSeek clients, OpenAI / Anthropic-compatible interfaces, and Mock-based tests
- CLI analysis, inbox, feedback, lifecycle and mark-read commands, plus standalone HTML export
- Read-only run statistics, decision replay and model-answer history; annotation CSV export/import and protocol validation

`ours` currently provides this basic workflow. Topic merging, Jev-based controller action selection, embedding candidates and evaluation scoring remain pending. The native GUI has passed Windows synthetic-data smoke checks, screenshot review and egui pointer interaction tests. Real export and cloud-model acceptance remains outstanding.

### Usage

See the PowerShell examples above. Import, queries and `analyze --dry-run` need no keys; dry-run does not contact services or write analysis state. Real analysis requires `CHAT_TLDR_LLM_API_KEY`; `TYPESAFE_API_KEY` enables Jev, otherwise decisions fall back to the LLM. Real analysis sends unredacted chat text to the configured services. Synthetic Docker-format input is at `fixtures/qce/template-docker-export.json`; export instructions are in [QCE_DOCKER_EXPORT](docs/QCE_DOCKER_EXPORT.md).

### Privacy

The program and its database run locally, but chat text is sent unredacted to two cloud services: Jev (TypeSafe) and the LLM you configure (DeepSeek by default). Images are never uploaded. The repository contains no real chat logs and no keys.

### License

MIT. chat-tldr only reads files exported by QQChatExporter (GPL-3.0). It does not link or include any of QQChatExporter's code.
