# CLI 与 JSONL 协议（CLI_PROTOCOL）

> 用途：定义 `chat-tldr` 命令行的命令、参数、stdout 上的 JSON Lines 事件、错误码、退出码和版本兼容规则。
> 读者：@Develata（实现 CLI）、同学 B（GUI 解析输出）、同学 C（eval 调用 CLI）。信封类型的定义见 [DATA_MODEL.md](DATA_MODEL.md) §4。

## 1. 总规则

1. **stdout 只输出机器协议**：每行一个 CliEvent JSON，UTF-8，`\n` 结尾，不输出空行，不做美化缩进。
2. **stderr 只输出给人看的日志**（`tracing`，级别由 `-v` / `RUST_LOG` 控制）。两者绝不混用。GUI 可以把 stderr 显示在“日志”面板里，但不能解析它。
3. 每次调用恰好以一个 `done` 事件结束（进程被强杀的情况除外）。GUI 以 `done` 作为结束判断，退出码只作为兜底。
4. `seq` 从 0 开始连续递增。`run_id` 在一次调用内不变。
5. 例外：带 `--html <FILE>` 时，HTML 写入文件，stdout 仍照常输出 JSONL。

## 2. 命令

二进制名：`chat-tldr`（`apps/cli`）。

### 2.1 全局参数

| 参数 | 默认值 | 说明 |
|---|---|---|
| `--data-dir <DIR>` | 系统数据目录下的 `chat-tldr/`（Windows：`%APPDATA%\chat-tldr`） | 数据库 `chat-tldr.db` 与配置 `config.toml` 所在目录 |
| `--config <FILE>` | `<data-dir>/config.toml` | 配置文件，见 §7 |
| `--self-uid <UID>` / `--self-uin <UIN>` | 取导出文件的 `chatInfo.selfUid` / `selfUin` | 两者都可能缺失，缺失又没传参数时，“@我”判断会降级并发出 `warning` |
| `-v` / `-vv` | | stderr 日志级别 |

### 2.2 命令一览

| 命令 | 作用 | 主要输出事件 |
|---|---|---|
| `version` | 输出版本与协议版本（GUI 启动时的握手） | `ack`, `done` |
| `import <PATH>...` | 导入一个或多个 QCE JSON 文件，幂等 | `progress`, `stats`(scope=import), `warning`, `done` |
| `chats` | 列出已导入的会话及三个游标、未读数 | `chat`*, `done` |
| `analyze --chat <ID>` | 运行智能体控制器，分析待处理消息 | `progress`, `decision`, `topic`, `insight`, `stats`(scope=run), `done` |
| `inbox --chat <ID>` | 查询收件箱（供 GUI 展示） | `inbox`, `topic`*, `insight`*, `done` |
| `messages --chat <ID> [--since] [--until]` | 按时间顺序列出消息及其话题归属（供评估和 GUI 浏览原文） | `message`*, `done` |
| `feedback <INSIGHT_ID> --useful \| --not-important` | 记录反馈 | `ack`, `done` |
| `resolve <INSIGHT_ID> --done \| --dismiss \| --reopen` | 修改结论的 `lifecycle` | `ack`, `done` |
| `mark-read --chat <ID> --up-to <CURSOR>` | 推进 `last_reviewed` | `ack`, `done` |
| `decisions --run <RUN_ID>` | 重放某次运行的决策日志 | `decision`*, `done` |
| `jev-log --run <RUN_ID>` | 导出某次运行的全部 Jev 回答（供校准评估） | `jev_answer`*, `done` |
| `stats [--chat <ID>] [--run <RUN_ID>]` | 运行统计 / 全局统计 | `stats`, `done` |

### 2.3 各命令参数

**`import <PATH>...`**
- 接受 QCE 单文件 JSON（顶层含 `metadata`、`chatInfo`、`statistics`、`messages`）。
- QCE 的 chunked-JSONL 导出（`manifest.json` + `chunks/*.jsonl`）：**TODO(Q-QCE-5)**，MVP 不支持，遇到时报 `E_INPUT_UNSUPPORTED`。
- 同一文件导入多次、两个导出有重叠：结果不变（`inserted=0` 或只插入新消息）。

**`analyze --chat <ID>`**

| 参数 | 默认值 | 说明 |
|---|---|---|
| `--since <RFC3339>` / `--until <RFC3339>` | 无 | 指定时间范围；不指定时处理全部待切分消息和脏话题（PIPELINE §1.1） |
| `--decider <jev\|llm>` | `jev` | Jev 不可用（未配置 key 或连续失败）时自动降级为 `llm` 并发 `warning` |
| `--strategy <ours\|b0\|b1\|sim-tfidf\|sim-embed>` | `ours` | 评估用的基线开关，见 [EVALUATION.md](EVALUATION.md) §2；GUI 不使用 |
| `--max-steps <N>` | 配置值（默认 64） | 控制器最大步数 |
| `--budget-usd <X>` | 配置值（默认 0.50） | 本次运行费用上限 |
| `--html <FILE>` | 无 | 运行结束后把收件箱渲染成 HTML（演示备用） |
| `--dry-run` | 关闭 | 只做确定性阶段，不调用任何模型 |

**`inbox --chat <ID>`**
- `--include-resolved`：同时返回 `done` / `dismissed` 的结论。
- `--include-rejected`：同时返回 `rejected` 的结论（仅供评估计算幻觉率；GUI 不使用）。
- `--html <FILE>`：同上。
- 收件箱的内容规则见 [PIPELINE.md](PIPELINE.md) §7.4。

**`mark-read --chat <ID> --up-to <CURSOR>`**
- `<CURSOR>` **必须**是 GUI 最近一次收到的 `inbox` 事件里的 `view_cursor`，原样回传。
- 这样可以保证只把用户确实看到的消息标为已读；GUI 渲染之后才导入的新消息不会被误标。
- 如果 `<CURSOR>` 早于当前 `last_reviewed`，则不做任何改动，返回 `ack`（`changed=false`）。

## 3. 事件与 payload

信封：`{"schema_version","run_id","seq","event","payload"}`。下文只列 payload。

### 3.1 `progress`
```json
{"stage":"segment","current":120,"total":1532,"message":"切分话题"}
```
`stage` ∈ `import | segment | decide | extract | verify | rank | store | render`。`total` 可为 `null`（未知）。

### 3.2 `decision`（智能体每一步）
```json
{
  "step": 3,
  "observation": {"pending_messages":412,"interleave":0.38,"active_topics":7,"dirty_topics":3,
                  "pending_verification":0,"merge_candidates":1,"steps_taken":3,"cost_usd":0.012},
  "allowed": [{"action":"analyze_topic","topic_id":"t_a19c3b0d77e2"},
              {"action":"merge_topics","into":"t_a19c3b0d77e2","from":["t_0b2e5f11c9aa"]}],
  "chosen": {"action":"merge_topics","into":"t_a19c3b0d77e2","from":["t_0b2e5f11c9aa"]},
  "method": "jev",
  "probabilities": {"analyze_topic":0.22,"merge_topics":0.78},
  "confidence": 0.56,
  "reason": "两个话题共享 4 条回复边且标题相近"
}
```
`method` ∈ `rule | jev | fallback`。只有一个候选时为 `rule`，`probabilities` 为 `null`。

### 3.3 `chat`
```json
{"chat_id":"qq:group:u_8KxZ2example","display_name":"数据结构课程群","kind":"group",
 "last_ingested":"1790168733000:1532","last_analyzed":"1790168733000:1532","last_reviewed":"1790078700000:1204",
 "unreviewed_messages":328,"open_p0":2}
```

### 3.4 `topic`
```json
{"topic_id":"t_a19c3b0d77e2","chat_id":"qq:group:u_8KxZ2example","title":"实验报告提交",
 "title_is_provisional":false,"state":"active","message_count":46,
 "first_message_at":"2026-09-23T20:41:02+08:00","last_message_at":"2026-09-23T22:13:40+08:00",
 "is_chitchat":0.08,"merged_into":null}
```

### 3.5 `insight`
payload = `{"insight": <Insight>, "evidence_view": [<EvidenceView>...]}`。其中 `Insight` 见 DATA_MODEL §3。

`EvidenceView` 由 CLI 计算，GUI 直接显示，不需要自己做匹配：
```json
{
  "message_id": "m_3f9a1c0b7d2e4a51",
  "sender_display": "班长-小王",
  "sent_at": "2026-09-23T21:05:33+08:00",
  "display_text": "@全体成员 周五前把实验报告交到课代表那里[图片]",
  "highlight": [6, 21],
  "ok": true
}
```
- `display_text` = `render(message, profile)`，即模型看到的同一份文本。
- `highlight` 是 quote 在 `display_text` 中的**字符**下标区间（Unicode scalar，左闭右开），由 `verify::find_quote` 计算；找不到时为 `null`，GUI 只显示原文，不高亮。
- `ok` 是这条证据的校验结果。

### 3.6 `inbox`（`inbox` 命令的第一行）
```json
{"chat_id":"qq:group:u_8KxZ2example","view_cursor":"1790168733000:1532",
 "last_reviewed":"1790078700000:1204","counts":{"P0":2,"P1":3,"P2":6,"P3":4},
 "rejected_insights":1,"generated_at":"2026-09-26T09:10:00+08:00"}
```
之后依次输出本次视图涉及的 `topic` 和 `insight` 事件（insight 按 priority，再按 `rank_score` 降序），最后是 `done`。

### 3.7 `stats`
```json
{"scope":"run","run_id":"r_20260926T090112_4b1e","chat_id":"qq:group:u_8KxZ2example",
 "messages_analyzed":412,"topics_created":5,"topics_updated":3,
 "insights":{"created":9,"updated":2,"verified":8,"unverified":1,"rejected":1},
 "usage":[{"stage":"decide","provider":"typesafe","model":"jev-1.13.0","calls":31,"cache_hits":4,
           "input_tokens":41200,"output_tokens":1900,"cost_usd":0.0017},
          {"stage":"extract","provider":"llm","model":"<配置的模型名>","calls":8,"cache_hits":0,
           "input_tokens":52000,"output_tokens":6100,"cost_usd":0.021}],
 "cost_usd":0.0227,"elapsed_ms":48210}
```
`scope=import` 时的字段为：`files`、`seen`、`inserted`、`duplicate`、`backfilled`、`recalled`、`chats`。`scope=global` 时为各表计数与累计用量。

### 3.8 `ack`
```json
{"command":"resolve","target":"i_7c2d9e01ab34","changed":true,"detail":{"lifecycle":"done"}}
```
`version` 命令的 `ack`：`{"command":"version","target":null,"changed":false,"detail":{"cli_version":"0.1.0","schema_version":"1.0","db_version":1}}`。

### 3.9 `warning`
```json
{"stage":"decide","code":"W_DECIDER_FALLBACK","message":"Jev 连续 3 次失败，本次运行改用 LLM decider"}
```

| code | 含义 |
|---|---|
| `W_SELF_ID_MISSING` | 导出文件没有 selfUid/selfUin，且未传 `--self-uid`/`--self-uin` |
| `W_DECIDER_FALLBACK` | Jev 不可用，已降级 |
| `W_BACKFILL` | 导入的消息早于 `last_analyzed`，下次 analyze 会补分析 |
| `W_UNKNOWN_ELEMENT` | 遇到未识别的 QCE 元素类型，已用占位符 |
| `W_INSIGHT_REJECTED` | 有结论未通过证据校验（附数量） |
| `W_DEADLINE_UNNORMALIZED` | 截止日期无法规范化，只保留原文 |

### 3.10 `error`
```json
{"stage":"extract","code":"E_PROVIDER_RATE_LIMIT","retryable":true,"message":"LLM 返回 429，重试 3 次后放弃","topic_id":"t_a19c3b0d77e2"}
```
`topic_id` 可选。一个 `error` 不一定终止运行：单个话题失败时运行继续，最终 `done.status = partial`。

### 3.11 `done`
```json
{"status":"complete","exit_code":0,"finish_reason":"done","elapsed_ms":48210}
```
`status` ∈ `complete | partial | failed | cancelled`，与 `runs.status` 一致。`finish_reason` 只在 `analyze` 时出现，取值同 `FinishReason`。

### 3.12 `jev_answer`
```json
{"model":"jev-1.13.0","request_key":"b3:5d1e…","question_id":"n1_todo","qtype":"noul",
 "subject":{"kind":"message","id":"m_3f9a1c0b7d2e4a51"},
 "answer":{"type":"noul","p_yes":0.93},"confidence":null}
```
`subject.kind` ∈ `message | burst | topic_pair | controller`。归属问题的 `subject` 为 `burst`，并附 `message_ids` 与 `candidates`（候选话题 ID 列表），供评估对齐。

### 3.13 `message`
```json
{"message_id":"m_3f9a1c0b7d2e4a51","sender":"qq:u_A1b2C3example","sender_display":"班长-小王",
 "sent_at":"2026-09-23T21:05:33+08:00","display_text":"@全体成员 周五前把实验报告交到课代表那里[图片]",
 "recalled":false,"system":false,"reply_to":null,"mentions_me":true,
 "topic_id":"t_a19c3b0d77e2","burst_id":"b_0012","cursor":"1790168733000:1532"}
```
`display_text` 为 `render(message, profile)`；`topic_id`、`burst_id` 在尚未切分时为 `null`。

## 4. 错误码

| code | stage | retryable | 含义 |
|---|---|---|---|
| `E_USAGE` | cli | false | 参数错误 |
| `E_CONFIG` | cli | false | 配置文件缺失或无效、必需的环境变量未设置 |
| `E_INPUT_NOT_FOUND` | import | false | 文件不存在 |
| `E_INPUT_PARSE` | import | false | JSON 解析失败或缺少必需字段 |
| `E_INPUT_UNSUPPORTED` | import | false | 不支持的导出形式（如 chunked-JSONL） |
| `E_CHAT_NOT_FOUND` | cli | false | `--chat` 不存在 |
| `E_INSIGHT_NOT_FOUND` | cli | false | 结论 ID 不存在 |
| `E_CURSOR_INVALID` | cli | false | `--up-to` 格式错误或不属于该 chat |
| `E_DB` | store | false | 数据库错误 |
| `E_DB_BUSY` | store | true | 超过 busy_timeout 仍被锁 |
| `E_RUN_IN_PROGRESS` | cli | true | 该 chat 已有 analyze 在运行 |
| `E_PROVIDER_AUTH` | decide/extract | false | 401/403，key 无效 |
| `E_PROVIDER_BAD_REQUEST` | decide/extract | false | 400/422，请求格式错误（是 bug） |
| `E_PROVIDER_RATE_LIMIT` | decide/extract | true | 429 |
| `E_PROVIDER_OVERLOADED` | decide/extract | true | 5xx（包括 Jev 的 529） |
| `E_PROVIDER_TIMEOUT` | decide/extract | true | 超时或网络错误 |
| `E_LLM_OUTPUT_INVALID` | extract | true | 输出不符合 schema，重试一次后仍失败 |
| `E_BUDGET_EXCEEDED` | cli | false | 达到 `--budget-usd` 上限 |
| `E_CANCELLED` | cli | false | 收到 Ctrl-C |
| `E_INTERNAL` | 任意 | false | 未预期的错误（是 bug） |

重试策略：`retryable=true` 的 provider 错误采用指数退避（0.5s、1s、2s，最多 3 次）；如果响应带 `retry-after`，按它等待。

## 5. 退出码

| 退出码 | 含义 | 对应 `done.status` |
|---|---|---|
| 0 | 成功 | `complete` |
| 1 | 内部错误 `E_INTERNAL` | `failed` |
| 2 | 参数错误 `E_USAGE` | `failed` |
| 3 | 输入错误（`E_INPUT_*`、`E_*_NOT_FOUND`、`E_CURSOR_INVALID`） | `failed` |
| 4 | 配置或认证错误（`E_CONFIG`、`E_PROVIDER_AUTH`） | `failed` |
| 5 | 模型服务错误，且无法降级 | `failed` |
| 6 | 部分完成（有话题失败或达到预算/步数上限） | `partial` |
| 7 | 数据库错误或并发冲突（`E_DB*`、`E_RUN_IN_PROGRESS`） | `failed` |
| 130 | 被取消 | `cancelled` |

## 6. 版本兼容规则

- `schema_version = "MAJOR.MINOR"`，当前 `"1.0"`。
- **MINOR 升级**：只允许新增事件类型、新增可选字段、新增枚举值。旧 GUI 必须能继续工作：忽略未知事件和未知字段，未知枚举值按“其他”显示。
- **MAJOR 升级**：删除/重命名字段、改变字段类型或语义。
- GUI 启动时先运行 `chat-tldr version`：MAJOR 不等于 GUI 编译时的 MAJOR → 拒绝运行，并提示“CLI 协议版本 X 与 GUI 不兼容，请更新”；MINOR 更高 → 正常运行。
- 任何对 DATA_MODEL 中冻结类型的修改都要按上述规则升级版本号，并在群里通知。

## 7. 配置文件（`config.toml`）

API key **不写进配置文件**，只写环境变量的名字。

```toml
timezone = "+08:00"

[jev]
base_url = "https://api.typesafe.ai"
model = "jev-1.13.0"                 # 固定版本，避免 jev-latest 漂移导致阈值失效
api_key_env = "TYPESAFE_API_KEY"
timeout_secs = 30

[llm]                                # 团队统一配置（Q-LLM-1）：DeepSeek 官方 API
api_format = "openai"                # "openai" | "anthropic"；DeepSeek 两种都支持
base_url = "https://api.deepseek.com"          # anthropic 格式为 https://api.deepseek.com/anthropic
model = "deepseek-flash"
api_key_env = "CHAT_TLDR_LLM_API_KEY"
temperature = 0.0
json_mode = true                     # 仅 openai 格式：response_format = {"type":"json_object"}
extra_body = { thinking = { type = "disabled" } }   # 原样合并进请求体；关闭 thinking 以便 temperature 生效
price_input_per_mtok = 0.30          # 按高峰价估算（缓存未命中）；非高峰为一半，缓存命中 0.006
price_output_per_mtok = 1.20
timeout_secs = 120

[embedding]                          # 可选；不配置则不启用 embedding 粗筛
# base_url = "..."                   # OpenAI 兼容的 /embeddings
# model = "..."
# api_key_env = "CHAT_TLDR_EMBED_API_KEY"

[agent]
max_steps = 64
budget_usd = 0.50
direct_max = 60                      # 积压 ≤ 此值时允许 AnalyzeDirect
direct_interleave_max = 0.2
segment_batch = 300                  # 每个 Segment 步最多处理的消息数

[segment]                            # 初始值，均待标注数据调优，见 OPEN_QUESTIONS
weak_gap_secs = 300
strong_gap_secs = 1800
same_sender_join_secs = 60
burst_max_messages = 20
topic_close_secs = 21600
tau_high = 0.60
tau_low = 0.25
candidate_k = 5
all_candidates_max = 40
state_token_budget = 24000
alpha = 0.6                          # 候选分：embedding 余弦（未配置 embedding 时视为 0）
beta = 0.3                           # 候选分：回复/@ 连边
gamma = 0.1                          # 候选分：时间衰减
sim_threshold = 0.35                 # 仅 sim-tfidf / sim-embed 基线使用
```

- `api_format = "openai"`：请求 `POST {base_url}/chat/completions`，Header `Authorization: Bearer <key>`。`extra_body` 中的字段原样合并进请求 JSON。
- `api_format = "anthropic"`：请求 `POST {base_url}/v1/messages`，Header `x-api-key: <key>` 与 `anthropic-version: 2023-06-01`。
- 具体取舍见 [decisions/0008-llm-client.md](decisions/0008-llm-client.md)。
