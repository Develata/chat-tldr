# 群聊省流 / chat-tldr

> Turn unread QQ group chats into a personal, traceable action inbox.
> 把未读群聊变成可追溯的个人行动收件箱。

**状态 / Status（2026-09-27）**：文件导入、增量分析、话题合并/自动关闭/历史回填、收件箱及六个总览已实现。CLI 新增更正/取消/冲突关联与待回应查询；eval 支持 Ours/B0、话题切分和概率校准。真实云调用记录见 [ACCEPTANCE](docs/ACCEPTANCE.md)，真实质量评分与 Jev 校准曲线仍为**待标注**。CLI/Windows GUI/Docker 发布见 [RELEASING](docs/RELEASING.md)，分工见 [TEAM_ASSIGNMENTS](docs/TEAM_ASSIGNMENTS.md)。
*The CLI supports import, incremental analysis, topic lifecycle, inbox/overview and evidence-backed changes and question states. Ours/B0 scoring, partition metrics and calibration tools are available. Real-data quality and Jev calibration await independent human labels. Version 0.2.0 adds Windows GUI distribution, local QQ acquisition and editable model settings; tag CI publishes the tested CLI, GUI and Docker artifacts.*

[![CI](https://github.com/Develata/chat-tldr/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Develata/chat-tldr/actions/workflows/ci.yml)

正式下载见 [GitHub Releases](https://github.com/Develata/chat-tldr/releases/latest)。本版 `v0.2.0` 的发布范围为 Windows CLI、Windows GUI（含 CLI/QCE manager）、Linux 静态 CLI 和 `ghcr.io/develata/chat-tldr:0.2.0`；tag 流程成功后提供对应产物。首版 `v0.1.0` 继续保留，发布与校验记录见 [ACCEPTANCE](docs/ACCEPTANCE.md)。

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
- 基础话题处理：小批量直接抽取，或按时间、发送者和回复规则形成消息组；跨段回复与明确身份的 @ 关系参与候选排序，[Jev](https://docs.typesafe.ai) 中等置信度的归属交由 LLM 复核
- 话题合并：已完成分析的话题由跨话题回复形成候选，经模型确认后原子合并；保留结论、反馈和证据，记住已否决的话题对，支持中断后继续
- 话题生命周期：按已处理消息的发送时间自动关闭旧话题；历史回填依据当时的活动记录归属，已关闭话题只补内容，保持关闭
- 结构化抽取：@我、待办、截止日期、通知、决策、话题摘要
- 证据校验：LLM 提出，Rust 验证
- P0–P3 分层收件箱 + 有界的反馈校准
- 分析总览：最近热门话题、优先话题、与我有关、截止事项、未读回顾和资料入口；只读 `overview`、原生 GUI 与 HTML 共用结果，见 [分析视图](docs/ANALYSIS_VIEWS.md)
- 控制器先以规则限定合法动作，多候选时由 Jev 选择下一步，失败回退规则；具有步数/费用上限、取消检查和决策日志
- 本地 SQLite 检查点、模型响应缓存与用量记录；部分失败后可继续处理未完成消息
- 同步 Jev 和 DeepSeek 客户端、OpenAI / Anthropic 兼容接口与 Mock；模型协议和流程用合成数据、Mock 及本地假服务器验证
- Rust CLI 的 `analyze/inbox/feedback/resolve/mark-read`，以及 `analyze --html` / `inbox --html` 导出
- 只读 `stats`、`decisions --run`、`jev-log --run`：查询累计用量或历史运行，重放决策与带归属的模型回答
- 更正/取消/冲突与完整/部分回答：独立关系表及只读 `relations`，两端引用重验，不自动更改用户 done/dismissed，见 [ADR-0010](docs/decisions/0010-semantic-relations.md)
- eval 的 `check-stream`、`export-sheet`、`import-sheet`、`score`、`calibrate`：Ours/B0、最优话题匹配/ARI/NMI、ECE/Brier 和 SVG；缺标注不造分数
- 极简非 root scratch [Docker 镜像](docs/DOCKER.md)；push 模拟 CLI/Windows GUI/Docker 发布，正式 tag 才公开 Release 与推送 GHCR

尚未实现 B1/sim-*、embedding、burst/边界与人工支持率指标、`agreement/summarize`。Windows GUI 从 `v0.2.0` 开始纳入正式发布。当前策略为 `--strategy ours|b0`，不代表 [PIPELINE](docs/PIPELINE.md) 的全部策略已经完成。QCE 管理组件支持本机登录/导出，GUI 已接入“从 QQ 获取聊天”；本地真实扫码、近七天导出导入与 CLI 分析已跑通，JSON 文件导入可独立使用。

### 已验证到哪里

| 范围 | 已有证据 | 尚未证明 |
|---|---|---|
| 主线逻辑与协议 | Rust 回归、fmt、严格 clippy；当前证据集中在 ACCEPTANCE | 真实准确率；费用是配置费率估算，非账单 |
| 输入兼容 | 一个 200 条真实 QCE 单文件通过 51 项离线检查；另有 100 条主样本 + 4 条回填合成样本 | 未出现的真实导出形态、JSON 卡片正文、附件内容 |
| GUI | 收件箱/总览合成截图、交互回归、Linux 原生 smoke、Windows release 测试与解包验证 | 真实聊天云分析的完整交互、macOS 原生窗口及各平台 GPU/缩放覆盖 |
| 评估 | 标注往返、Ours/B0 合成流程、话题匹配和校准手算回归 | 独立人工 gold、真实质量比较和真实 Jev 校准曲线 |

CI 按 workspace 模块并行，覆盖 Windows 全部模块、Linux CLI/eval/manager 与 GUI 原生 smoke、macOS CLI/GUI/eval/manager，保留 `fmt`、`clippy`、`test` 必需检查名称；GUI smoke 失败或跳过也不能通过 `test`。Actions 使用 Node.js 24、Linux 使用 Ubuntu 26.04；版本固定完整 SHA。GUI 发布验收及平台边界见 [RELEASING](docs/RELEASING.md#gui-自动化验收)，运行结果见上方 CI。

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

`fixtures/` 全为人工合成数据。适配器另已通过一个 200 条真实单文件样本的离线验收，原始内容不提交 Git 仓库；样本范围及未支持的 JSON 卡片见 [验收记录](docs/ACCEPTANCE.md#首次真实单文件验收2026-09-26)。CSV 需独立人工填写，再用 `import-sheet <CSV> --out <新目录>` 生成 gold；列格式、UTF-8 与不覆盖规则见 [eval 使用说明](eval/README.md)。这一步不生成评分。

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

GUI 保留文件导入，并提供“从 QQ 获取聊天”：连接本机 QCE → 扫码登录 → 选群/时间 → 导出并导入，见 [操作说明](apps/gui/README.md#从-qq-获取聊天)。随附 manager，不安装/捆绑 QCE/NapCat。在“设置 → 模型与 API”填写 OpenAI/Anthropic 格式、服务地址、模型和 API key，LLM/Jev 分别保存；重启后继续使用。密钥按课程阶段决定明文存入本地 config，输入框遮罩，查询和日志不回显。首次云端分析需确认聊天原文会发送给配置的服务；获取、导入和查询不调用模型。`doctor` 检查本地配置和密钥是否就绪，不验证网络连通性。

### 查看分析总览

已导入并分析的群聊可以直接查看总览，不会追加模型请求：

```powershell
cargo run -p chat-tldr -- --data-dir ./private/demo overview --chat qq:group:synthetic-study --html ./private/demo/overview.html
```

默认统计最近 24 小时；历史材料可传带时区的 `--since` / `--until`。GUI 中点击“分析总览”可切换热门、优先、相关、截止、未读和资料六个视图。没有完成分析的消息只显示原始提及/资料和待分析数量，不会产生热榜或模型结论；总览不标已读。

| 视图 | 看什么 |
|---|---|
| 热门话题 | 最近讨论活跃、参与人数多的话题；限制单人重复刷屏贡献 |
| 优先话题 | 按最高有效 P0–P3 排列未处理事项，保留窗口前仍未完成的事项 |
| 与我有关 | 分配给我/全体的事项，以及真实身份匹配的 @我/@全体 |
| 截止事项 | 已逾期、尚未到期和时间待确认的事项，保留实际负责人 |
| 未读回顾 | 证据位于已读游标之后的结论；查看总览不会推进游标 |
| 资料入口 | 窗口内链接、附件元数据与来源消息；不读取附件正文 |

热门与优先使用不同排序。除话题摘要外，任何带截止日期的结论都属于 P0，即使由他人负责。总览按已有分析结果聚合，不额外调用模型；时间窗口和证据边界见 [ANALYSIS_VIEWS](docs/ANALYSIS_VIEWS.md)。

多场景测试可使用 [scenario-analysis.json](fixtures/qce/scenario-analysis.json) 与 [回填样本](fixtures/qce/scenario-analysis-backfill.json)。这是 Codex 补齐的共用测试材料，场景映射和运行顺序见 [样本说明](fixtures/qce/scenario-analysis.README.md)，尚不是独立人工标注的正式 gold。

### 使用真实模型

下面的 `analyze` 会把聊天原文发送到配置的云服务。默认使用 Jev 做分类与话题归属、DeepSeek 做抽取；LLM 密钥必需，Jev 密钥缺失或服务不可用时会警告并改用 LLM。也可显式传入 `--decider llm`。密钥优先从本地 config 的 `api_key` 读取，没有时才使用 `api_key_env` 指定的环境变量。

GUI 中可直接保存；CLI 的等价命令如下（`--key-prompt` 隐藏输入，写入本地明文 config，不需要重启终端）：

```powershell
cargo run -p chat-tldr -- --data-dir private/demo config set llm --api-format openai --base-url https://api.deepseek.com --model deepseek-flash --key-prompt
cargo run -p chat-tldr -- --data-dir private/demo config set jev --key-prompt
cargo run -p chat-tldr -- --data-dir private/demo config show
cargo run -p chat-tldr -- --data-dir private/demo doctor
```

已有 config 时只更新指定字段，密钥不会出现在 `config show` 中。`config set llm --clear-key` 删除保存的密钥并恢复环境变量查找。Anthropic 可使用 `--api-format anthropic --base-url https://api.anthropic.com --model <模型名>`；OpenAI 常见地址为 `https://api.openai.com/v1`。更改地址/格式时重新填写该服务密钥。更多参数见 [CLI 协议](docs/CLI_PROTOCOL.md)。

也可以在本地 `.env` 中填写 [.env.example](.env.example) 的两个变量，然后用 `./scripts/with-env.ps1 cargo run -p chat-tldr -- doctor` 检查配置。脚本只为本次子进程加载密钥，不执行文件内容；CLI 本身不自动读取 `.env`。用法与格式见 [本地模型密钥](docs/ENVIRONMENT.md)。

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

下载完成后，可在 PowerShell 7.2+ 执行 `pwsh -NoProfile -File .\scripts\verify-qce.ps1 -InputFile 'C:\path\group.json'`。先用 `cargo build -p chat-tldr -p chat-tldr-eval --locked` 编译；脚本在新建私有目录中核对重复导入、JSONL 协议、消息数量和游标，保存可追溯回执，全程不调用模型。参数与真实云联调步骤见 [交付验收](docs/ACCEPTANCE.md)。

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
| [ROADMAP](docs/ROADMAP.md) | 当前交付缺口、真实期限、降级预案与历史计划 |
| [decisions/](docs/decisions/) | 架构决策记录 |
| [OPEN_QUESTIONS](docs/OPEN_QUESTIONS.md) | 待定问题 |
| [ANALYSIS_VIEWS](docs/ANALYSIS_VIEWS.md) | 已实现分析视图、统计口径、剩余语义功能与测试范围 |
| [CONTRIBUTING](CONTRIBUTING.md) | 协作流程（写给第一次用 Git 的同学） |

### 团队分工

| 成员/席位 | 当前交付状态 | 接下来负责 |
|---|---|---|
| @Develata | 已确认产品规则与接口方向 | 架构/代码审核、真实结果复核、交付取舍 |
| Codex | 主线代码、六个总览、合成回归材料及 CI 已实现 | 剩余分析语义、集成修复和文档维护 |
| A：QCE 管理 | 仓库仅有接入说明，尚无管理程序 | 复用已验证导出流程，交付组件管理与完整 JSON 路径 |
| B：GUI 设计 | @liguilong256 的 PR #7 已交付界面整理、运行详情与七张合成截图 | Codex 接续修复 review 问题并补自动 smoke/发布测试，B 继续体验复核 |
| C：验收材料 | 共用合成样本/评估工具已提供，未见独立标注或报告 | 人工期望与 gold、演示步骤、验收报告 |

A/C 尚未绑定账号，线下进度待本人确认；B 的交付以 PR #7 为据。具体交付和目录边界以 [TEAM_ASSIGNMENTS](docs/TEAM_ASSIGNMENTS.md) 为准。实际交付截止为 **2026-09-27 23:59，America/Santiago（UTC−3）**。

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
- Topic assignment using temporal bursts, indexed reply/mention links and ranked candidates; ambiguous [Jev](https://docs.typesafe.ai) decisions receive LLM review with the same candidates
- Recoverable, model-confirmed topic merging; message-time closure and historical backfill that keeps closed topics closed
- Structured extraction: mentions of you, to-dos, deadlines, announcements, decisions, topic summaries
- Evidence verification: the LLM proposes, Rust verifies
- P0–P3 tiered inbox with bounded feedback calibration
- Six read-only overview views: popular topics, priority topics, related-to-me items, deadlines, unread recap and resources; shared by CLI, GUI and HTML without additional model requests
- A bounded controller with rule-based action selection, SQLite checkpoints, response caching and usage records
- Synchronous Jev / DeepSeek clients, OpenAI / Anthropic-compatible interfaces, and Mock-based tests
- CLI analysis, inbox, feedback, lifecycle and mark-read commands, plus standalone HTML export
- Read-only run statistics, decision replay and model-answer history; annotation CSV export/import, protocol validation and offline Ours extraction/ranking scores

Both `ours` and `b0` are implemented. Change/cancellation links, unanswered-question semantics, Jev controller action selection, topic metrics and calibration tools are available; real quality scores remain pending independent human labels. The QCE manager and GUI acquisition wizard provide login, export and import using an existing local QCE service. Embedding candidates remain deferred.

Rust regressions use synthetic data, mocks and local fake services; current validation is recorded in docs/ACCEPTANCE.md. The Windows inbox has native interaction evidence; overview screenshots remain pending. One real 200-message QCE export passed offline checks. The shared 100-message scenario and four-message backfill fixture support regression and independent annotation. Real cloud runs are recorded separately from quality evaluation: independent human labels are required before publishing real-data quality or calibration scores.

CI runs Windows modules in parallel, with Linux CLI/eval and macOS CLI/GUI/eval checks. Actions use Node.js 24 and Linux uses Ubuntu 26.04; pinned action revisions are maintained through weekly Dependabot PRs. Required check names remain `fmt`, `clippy` and `test`.

### Usage

See the PowerShell examples above. Import, queries and `analyze --dry-run` need no keys; dry-run does not contact services or write analysis state. Configure API format, URL, model and key in GUI Settings or `config set llm|jev`. For this coursework version, keys are saved as plaintext in local config; they are masked in the GUI and omitted from query output/logs. Saved keys take precedence over environment variables. Without a saved key, `CHAT_TLDR_LLM_API_KEY` supplies the LLM and `TYPESAFE_API_KEY` enables Jev; absent Jev credentials use LLM fallback. Real analysis sends unredacted chat text to the configured services. Synthetic Docker-format input is at `fixtures/qce/template-docker-export.json`; export instructions are in [QCE_DOCKER_EXPORT](docs/QCE_DOCKER_EXPORT.md).

For local `.env` files, use [.env.example](.env.example) and run `./scripts/with-env.ps1 cargo run -p chat-tldr -- doctor`. The wrapper supplies keys only to its child process, preserves existing environment values and treats the file as literal data. The CLI does not load `.env` automatically; see [environment setup](docs/ENVIRONMENT.md).

### Privacy

The program and its database run locally, but chat text is sent unredacted to two cloud services: Jev (TypeSafe) and the LLM you configure (DeepSeek by default). Images are never uploaded. The repository contains no real chat logs and no keys.

### License

MIT. chat-tldr only reads files exported by QQChatExporter (GPL-3.0). It does not link or include any of QQChatExporter's code.
