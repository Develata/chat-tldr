# 处理流水线（PIPELINE）

> 用途：说明从 UnifiedMessage 到 Insight 的每一步算法、规则、阈值和不变量。
> 读者：@Develata（engine 实现）、同学 A（`temporal`、`verify` 两个模块）、同学 C（评估时需要理解每个阶段）。类型定义见 [DATA_MODEL.md](DATA_MODEL.md)。

> 状态：本文是流水线实现契约，不是全量实现报告。基础导入与查询先行；模型、控制器、证据校验、反馈和收件箱按主线计划继续实现。实际可用命令由 `version.capabilities` 报告。

所有阈值都是**初始值**，汇总在 `config.toml` 的 `[segment]` 节（见 [CLI_PROTOCOL.md](CLI_PROTOCOL.md) §7），待用标注数据调优（[OPEN_QUESTIONS.md](OPEN_QUESTIONS.md) Q-TH-1）。

## 0. 总览

```
QCE JSON ─▶ qce 适配器 ─▶ UnifiedMessage ─▶ import（去重、游标、别名、悬空引用补全）
                                                   │
                                                   ▼
                              ┌──────── 智能体控制器（§8，有界循环）────────┐
                              │  观测状态 → 规则算出允许的动作 → 多于一个时 Jev 选 │
                              └──┬────────────┬──────────────┬─────────┬─────┘
                                 ▼            ▼              ▼         ▼
                           AnalyzeDirect   Segment(§3)   AnalyzeTopic  MergeTopics
                                 │      burst→候选→Jev归属  (§5 LLM抽取)
                                 └────────────┴──────┬───────┘
                                                     ▼
                                          Verify（§6 证据校验）
                                                     ▼
                                         排序（§7 分层 + 层内分）
                                                     ▼
                                   SQLite ─▶ CLI JSONL ─▶ GUI
```

## 1. 增量状态：游标与结论生命周期

### 1.1 三个游标（每个 chat 各一组）

| 游标 | 含义 | 何时推进 |
|---|---|---|
| `last_ingested` | 已导入的最大消息位置 | `import` 事务提交时 |
| `last_analyzed` | 最长的“已了结”前缀的末尾：所有 `≤` 它的消息 `analysis_state ≠ pending` | 每个话题的检查点提交后重新计算 |
| `last_reviewed` | 用户明确“标为已读”的位置 | **只在** `mark-read --up-to` 时推进；打开 GUI、运行 analyze 都不推进 |

游标类型是 `Cursor(sent_at_ms, ordinal)`，见 DATA_MODEL §1.3。

`last_analyzed` 的“已了结”允许 failed，只用于分析进度；它不是安全已读边界。`inbox.view_cursor` 从同一快照计算连续 `done/skipped` 前缀的末尾，不能越过 failed/pending，无安全位置时为 null。GUI 只回传已完整显示的非空 view_cursor，CLI 还须校验会话归属和安全上界。

**消息的分析状态** `messages.analysis_state`：

| 值 | 含义 | 何时设置 |
|---|---|---|
| `pending` | 等待分析 | 导入时的默认值 |
| `done` | 已随所在话题完成抽取 | 话题检查点提交时 |
| `skipped` | 不参与分析（撤回消息） | 导入时直接设置，因此撤回消息不会卡住 `last_analyzed` |
| `failed` | 所在话题本次抽取失败 | 见 §8.2；下次 `analyze` 开始时恢复为 `pending` |

**两个“待处理”的定义**（控制器按它们判断还有没有事要做）：
- 待切分消息：还没有 `topic_messages` 记录、且 `analysis_state = pending` 的消息。
- 脏话题（dirty topic）：包含 `analysis_state = pending` 消息的话题。

**回填（backfill）**：两次导出有重叠，或者后导入了更早的文件时，可能插入早于 `last_analyzed` 的消息。
- `import` 统计这类消息数量（`imports.backfilled`），并发出 `W_BACKFILL`。
- 这类消息自然会成为“待切分消息”，下次 `analyze` 会处理。归属候选取**该消息发送时刻**仍然活跃的话题（可能包括现在已经关闭的话题）。
- 此时 `last_analyzed` 会按定义回退，处理完后再前进。不会有消息被静默跳过。

**悬空引用补全**：每次 `import` 后，对 `reply_resolved IS NULL` 的消息，按 `reply_source_id` 查找是否已有被引用的消息，有则补全。

### 1.2 分析范围

- `analyze` 默认处理全部待切分消息和脏话题（即“自上次分析以来”的增量）。话题状态随之持续维护，不会每次从头重算。
- “自上次查看以来”是**收件箱的展示窗口**（§7.4），不是分析范围。
- `--since/--until` 使用带偏移的 RFC 3339，按 `[since, until)` 限制本次可处理消息；启动时固定快照，期间新增消息留待下次。范围外消息可作上下文，不能随之推进分析状态。这与 `MessageRange(after, up_to]` 的 Cursor 约定不同。
- `--dry-run` 仅输出只读计划，不联网、不写数据库/缓存、不推进任何游标。没有工作时正常结束，不调用模型。

### 1.3 结论生命周期

- `lifecycle` 取值为 `open`、`done`、`dismissed`，只能由 `resolve` 命令修改。
- 话题有新消息并重新抽取时，LLM 能看到该话题已有的 open 结论，可以输出 `update` 去更新它们（追加证据、修改截止日期），ID 和 `lifecycle` 保持不变。这样同一件事不会重复产生结论。
- `done` 或 `dismissed` 的结论不会被重新打开。如果新消息明确“重新提起”了这件事，产生一条新结论。

## 2. 预处理与渲染

### 2.1 导入时（qce 适配器 + import）

多文件 import 是单个原子批次，任一文件无效则全部回滚。提交后才发出 ack（去重排序后的 chat_ids），输入文件本身始终只读。显式自身身份不能与已存值冲突，未提供时保留已存身份，新会话才回退文件元数据。

| 情况 | 处理 |
|---|---|
| 撤回消息 | 保留一行，`recalled=true`，`text` 置空，`analysis_state = skipped`。**不参与切分和抽取**，不能作为证据。GUI 可显示“某消息已撤回” |
| 图片 | `[图片]` 占位，元数据进 `attachments`，永不上传 |
| 表情 | 有名字 `[表情:名字]`，否则 `[表情]` |
| 纯表情刷屏 | 同一 burst 内全部由表情/图片占位组成的消息，在抽取时降权（渲染时合并为 `[表情×N]`） |
| 回复、@ | 必须保留，它们是切分最强的信号 |
| 合并转发 | 递归展开，最大深度 2；更深的层只保留标题 |
| 系统消息 | 保留，不一刀切删除（群公告可能是重要通知）；渲染时加前缀 `[系统]` |
| 昵称、群名片 | 写入 `person_aliases`，不参与身份判断 |

### 2.2 渲染 `render(msg, profile) -> String`

纯函数、确定性输出，是**发给模型的唯一文本来源**，也是证据校验的依据。规则（`r1`）：

1. 以 `UnifiedMessage.text` 为基础。
2. 合并转发展开为：`[合并转发:<title>]` 后跟每条内部消息一行 `  > <sender_display>: <text>`，深度 2 缩进 4 格。
3. 系统消息加前缀 `[系统]`。
4. 规则有任何改动 → `RenderProfile.version + 1`。

不做脱敏（Q-DEC-2 已决定）：发给模型的文本、校验用的文本、显示给用户的文本是同一份 `render` 结果。

发给模型的消息格式：一行一条，`<ref> [<HH:MM>] <发送者显示名>: <渲染文本>`。`<ref>` 是本次请求内的短引用（`n1`、`n2`…，上下文消息为 `c1`…），由 Rust 维护与 `MessageId` 的映射。模型只需要复述短引用，出错时更容易发现。

## 3. 话题切分（混合式，不训练模型）

### 3.1 第一层：确定性 burst

按 `(sent_at, ordinal)` 顺序扫描待切分消息，组成 burst（小段）。**不对单条消息做语义聚类**。

| 信号 | 规则 |
|---|---|
| 强边界 | 与上一条的间隔 ≥ `strong_gap_secs`（1800s）→ 结束当前 burst |
| 弱边界 | 间隔 ≥ `weak_gap_secs`（300s）→ 结束当前 burst，除非本消息有连向当前 burst 的回复边或 @ 边 |
| 回复边 | `reply_to.resolved` 指向当前 burst 内的消息 → 加入当前 burst |
| @ 边 | @ 了当前 burst 内的某个发送者，且间隔 < 弱边界 → 加入 |
| 同一发送者连续发言 | 同一发送者、间隔 ≤ `same_sender_join_secs`（60s）→ 加入 |
| 大小上限 | burst 达到 `burst_max_messages`（20）条 → 强制结束 |

跨 burst 的回复边和 @ 边保留下来，作为第二层的特征。

### 3.2 第二层：候选话题（增量在线聚类）

维护活跃话题列表：`state=active`，并且 `last_message_at` 距当前 burst 不超过 `topic_close_secs`（6 小时）。超过这个时间的话题自动置为 `closed`。

对每个 burst：

1. **规则捷径**：如果 burst 的全部回复边都指向同一个活跃话题 → 直接归入该话题，`method=rule_reply`，不问 Jev 的归属问题（仍然问分类问题）。
2. **候选集合**：
   - 若活跃话题数 ≤ `all_candidates_max`（40），**且**估算的 state token 数 ≤ `state_token_budget`（24k，低于 Jev 对“state + 最长问题”的 32k 上限）→ **全部活跃话题都作为候选**（默认路径，不需要 embedding）。
   - 否则取综合分最高的 `candidate_k`（5）个：
     `score = α·cos(emb(burst), emb(topic)) + β·edges(burst, topic) − γ·decay(Δt)`
     `edges` 为 burst 指向该话题的回复/@ 边数（上限 3，归一化到 [0,1]），`decay(Δt) = Δt / topic_close_secs`。
     没有配置 embedding 时 `α = 0`，只按连边和时间排序。初始值 `α=0.6, β=0.3, γ=0.1`。
3. token 估算：中文按每字符 1 token 保守估计。

### 3.3 第三层：Jev 归属判断

每个 burst **只发一次 Jev 请求**，把归属问题和分类问题合并在一起（Jev 支持在一个请求里放多个 question，见 §4.2）。归属问题是一个 `choice`，选项为候选话题加上 `new_topic`。

按 Jev 返回的 `confidence`（已按选项数归一化，不同候选数之间可以比较）分三档：

| 条件 | 处理 | `method` |
|---|---|---|
| 最高项 ≠ `new_topic`，且 `confidence ≥ tau_high`（0.60） | 直接归入 | `jev` |
| `tau_low ≤ confidence < tau_high` | 交给 LLM 复核（同样的候选，LLM 选一个） | `llm_review` |
| 最高项 = `new_topic`，或 `confidence < tau_low`（0.25） | 新开话题 | `new_topic` |

原始的 `probabilities` 和 `confidence` 都写入 `jev_answers`，用于画校准曲线、调阈值。

### 3.4 标题、关闭、合并

- 新话题的临时标题 = 第一条非占位消息的前 20 个字符，`title_is_provisional=true`。`AnalyzeTopic` 时由 LLM 起正式标题。
- 合并候选：两个活跃话题之间有 ≥ 2 条互相指向的回复边；配置了 embedding 时，另加余弦相似度 ≥ 0.85 的话题对。执行 `MergeTopics` 前先用 Jev noul 问“这两个话题是不是同一件事”，`p < 0.5` 就不合并，并记住这一对，以后不再提议。

### 3.5 基线：相似度聚类（不用 Jev）

`--strategy sim-tfidf` / `sim-embed`：第一、二层相同，第三层换成纯阈值：候选中 `score` 最高且 ≥ `sim_threshold` 的话题直接归入，否则新开话题。TF-IDF 向量用 `jieba-rs` 分词，IDF 在本 chat 已导入的消息上统计。这个基线用来衡量 Jev 归属判断带来的增益（见 EVALUATION）。

## 4. 模型分工

| 模型 | 负责 | 不负责 |
|---|---|---|
| Jev（Decider） | 话题归属；每条消息是否通知/待办/需要我行动；burst 紧急程度；是否闲聊；是否合并话题；控制器在多个候选动作间选择 | 生成任何文本；日期计算；计数 |
| 普通 LLM（LlmClient） | 起标题、写摘要、抽取结论与证据、给出截止日期原文、复核中等置信度的归属 | 判断“@我”；日期计算；证据真伪 |
| Embedding（Embedder，可选） | 活跃话题过多时粗筛候选 | 单条消息的语义聚类 |
| Rust 规则 | @我 判定、burst 切分、日期规范化、证据校验、优先级分层、预算与停止条件 | — |

依据：Jev 官方文档说明它擅长“窄而结构化的判断”，不擅长数字、日期比较和生成，并且中文精度低于英文（docs.typesafe.ai/models#language-support、/model-jaggedness/jev-1.13）。因此：日期和计数一律在代码里做；Jev 的判断一律记录概率并做校准评估；Jev 出问题时可以降级到 LlmDecider。

### 4.1 trait 抽象（engine 内部，不属于冻结类型）

```rust
pub trait Decider {
    fn decide(&self, req: &DecisionRequest) -> Result<DecisionResponse, ProviderError>;
    fn name(&self) -> &str;                       // "jev-1.13.0" / "llm:<model>"
}
pub struct DecisionRequest {
    pub state: serde_json::Value,                 // 对象，字段见 §4.2
    pub questions: BTreeMap<String, Question>,    // key 自定，按原样返回
}
pub enum Question {
    Noul   { instructions: String, criteria: Option<(String, String)> },  // (true 的含义, false 的含义)
    Choice { instructions: String, options: BTreeMap<String, String> },   // 选项 key → 描述，≤255 个
    Score  { instructions: String, levels: Vec<String> },                 // 2–10 级，有序
}
pub enum Answer {
    Noul   { p_yes: f32 },                                                // Jev 的 noul 没有 confidence
    Choice { choice: String, probabilities: BTreeMap<String, f32>, confidence: f32 },
    Score  { score: f32, probabilities: BTreeMap<String, f32>, confidence: f32 },
}
pub struct DecisionResponse { pub model: String, pub answers: BTreeMap<String, Answer>, pub usage: Usage }

pub trait LlmClient {
    fn complete(&self, req: &LlmRequest) -> Result<LlmResponse, ProviderError>;
}
pub trait Embedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, ProviderError>;
}
```

实现：`JevDecider`、`LlmDecider`、`MockDecider`；`OpenAiCompatClient`、`AnthropicCompatClient`、`MockLlm`；`OpenAiCompatEmbedder`、`TfIdfEmbedder`、`MockEmbedder`。测试只用 Mock。

同步接口（`reqwest::blocking`），不引入 async，理由见 [decisions/0008-llm-client.md](decisions/0008-llm-client.md)。

### 4.2 Jev 请求

按官方 API（docs.typesafe.ai/api，2026-09-26 查阅）：`POST https://api.typesafe.ai/v1/systemone`，Header `Authorization: Bearer <TYPESAFE_API_KEY>`，body 为 `{state, model, questions}`。question 的 `type` 有 `noul`、`choice`（criteria 为 选项→描述 的 map，≤255 个）、`score`（criteria 为 2–10 个有序级别）。响应为 `{model, answers, usage:{input_tokens, output_tokens}}`；`choice`/`score` 带 `confidence`，`noul` 只有 `noul` 值。**只按输入 token 计费**（jev-1.13.0：每百万 token 0.042 美元）。429、529 可重试，401、422 不可重试。

每个 burst 的请求结构（示意，question key 由我们命名）：

```json
{
  "model": "jev-1.13.0",
  "state": {
    "candidate_topics": [
      {"key": "T1", "title": "实验报告提交", "recent": ["小李: 报告模板在群文件", "小张: 封面要写学号吗"]},
      {"key": "T2", "title": "周末聚餐", "recent": ["小陈: 周六晚上可以吗"]}
    ],
    "new_messages": [
      {"ref": "n1", "sender": "班长-小王", "time": "21:05", "text": "@全体成员 周五前把实验报告交到课代表那里[图片]"},
      {"ref": "n2", "sender": "小赵", "time": "21:06", "text": "收到"}
    ]
  },
  "questions": {
    "topic": {"type": "choice",
      "instructions": "Which topic in `candidate_topics` do the `new_messages` continue?",
      "criteria": {"T1": "实验报告提交", "T2": "周末聚餐", "new_topic": "None of the candidates; this starts a new topic"}},
    "n1_announcement": {"type": "noul", "instructions": "Is `new_messages[0]` a notice or announcement meant for group members?"},
    "n1_todo": {"type": "noul", "instructions": "Does `new_messages[0]` ask someone to do a concrete task?"},
    "n1_needs_action": {"type": "noul", "instructions": "Does `new_messages[0]` require the mentioned people to take an action?"},
    "urgency": {"type": "score", "instructions": "How time-sensitive are the `new_messages`?",
      "criteria": ["Not time-sensitive", "Should be handled within days", "Must be handled within hours"]},
    "chitchat": {"type": "noul", "instructions": "Are the `new_messages` casual chit-chat with no information value?"}
  }
}
```

- `n{i}_needs_action` 只对 @我 / @全体成员 的消息提问；纯占位消息（如“收到”“[表情]”）不提分类问题。
- instructions 与 criteria 一律用英文（Q-JEV-2 已决定）；state 中的聊天内容保持原文（中文或英文），不翻译。
- 结果缓存（§9），所有答案写入 `jev_answers`。

### 4.3 LlmDecider（降级与消融）

接受同样的 `DecisionRequest`，把 state 和 questions 写进提示词，要求 LLM 为每个问题输出各选项的概率（即“口头报告的概率”），用 serde 校验并归一化。choice 的 `confidence` 按与 Jev 文档示例相同的公式计算：`(n·p_max − 1)/(n − 1)`。LLM 自报的概率通常校准较差，正好作为 Jev 校准曲线的对照。

### 4.4 LLM 调用约定

- **所有提示词模板一律用英文**（system prompt、指令、schema 说明、示例说明）；插入其中的聊天内容保持原文，不翻译；要求模型输出的 title/summary 使用聊天的主要语言（中文群输出中文）。
- 评估与演示统一使用 DeepSeek `deepseek-flash`，并关闭 thinking 模式（`{"thinking": {"type": "disabled"}}`）：thinking 模式下 `temperature` 不生效，结果无法复现（依据 api-docs.deepseek.com，2026-09-26 查阅）。`api_format = "openai"` 时另外开启 `response_format = {"type": "json_object"}`（DeepSeek 支持；要求提示词中出现 “json” 并给出示例）。
- 不依赖 provider 的 `json_schema` 严格模式：用 `schemars` 从 Rust 类型生成 JSON Schema 写进提示词；输出用 `serde_json` 解析和校验；失败时把错误信息附在提示词后**重试一次**，仍失败 → `E_LLM_OUTPUT_INVALID`，该话题记为失败。
- 两种接口格式（`openai` / `anthropic`），见 CLI_PROTOCOL §7。
- temperature 固定为 0（provider 支持时），以便缓存和复现。

## 5. LLM 抽取

### 5.1 AnalyzeTopic

输入：话题当前标题；该话题的 open 结论（`e1`…：kind、title、截止日期原文）；新消息（`n1`…）；最多 5 条已分析过的上下文消息（`c1`…）。

输出 schema（Rust 类型 `TopicExtraction`，schemars 生成）：

```json
{
  "title": "实验报告提交",
  "summary": "班长通知周五前交实验报告；有人问封面格式，已答复需写学号。",
  "items": [
    {
      "op": "new",
      "existing_ref": null,
      "kind": "todo",
      "title": "周五前交实验报告",
      "summary": "交给课代表，封面写学号。",
      "assignee": "all",
      "deadline_raw": "周五前",
      "deadline_date_guess": "2026-09-25",
      "evidence": [{"ref": "n1", "quote": "周五前把实验报告交到课代表那里"}]
    }
  ]
}
```

- `op` ∈ `new | update`；`update` 时 `existing_ref` 为 `e1` 这类引用。
- `kind` 只能是 `todo | announcement | decision`。`MentionMe` 由规则生成（§6.1），`TopicSummary` 由 `title`/`summary` 生成，其证据取模型在 items 中引用过的消息，以及标题相关的前两条消息。
- `deadline_date_guess` 仅在规则规范化失败时作为兜底（§5.3）。

### 5.2 AnalyzeDirect

积压少时，一次调用处理全部待切分消息：输出话题列表（每个话题列出包含的 `ref`）以及每个话题的 items（格式同上）。话题归属写入 `topic_messages`，`method=direct`。

### 5.3 截止日期

流程：LLM 给出 `deadline_raw` → `temporal::normalize(raw, anchor)` 用规则规范化 → 规则失败时，使用 `deadline_date_guess`（`normalized_by=llm`，`confidence ≤ 0.5`）→ 仍没有 → `bound_date=None`，`normalized_by=none`，只保留原文，并发出 `W_DEADLINE_UNNORMALIZED`。

`anchor` = 证据消息的 `sent_at`（有多条证据时，取包含 raw 的那条）。

规则覆盖的高频表达（由同学 A 实现，规则表见 [tasks/task-A-qce-temporal-verify.md](tasks/task-A-qce-temporal-verify.md)）：今天、今晚、明天、明晚、后天、大后天；周X、星期X、礼拜X、这周X、下周X；X点、X点半、X点Y分，配合上午/下午/晚上；X天后、X天内；M月D日、D号；以及关系词：前、之前、以前、内、以内、截止、截至、ddl → `before`，后、之后、以后 → `after`，其余 → `at`。

签名（冻结给同学 A）：

```rust
// crates/engine/src/temporal/mod.rs —— 纯函数，无 IO
pub fn normalize(raw: &str, anchor: DateTime<FixedOffset>) -> TemporalConstraint;
```

规范化失败时返回 `bound_date=None, normalized_by=None`，**不要 panic、不要猜**。

## 6. 确定性信号与证据校验

### 6.1 @我 判定（只用规则，不交给任何模型）

一条消息“@我”当且仅当它的 `mentions` 中存在：
- `target = All`（@全体成员 视同 @我）；或
- `target = User`，且 `uid == self_uid`，或者 `uin == self_uin`，或者 `uid == self_uin`（QCE 可能把 QQ 号放进 uid）。

`self_uid`/`self_uin` 是 import 时保存的会话身份；显式参数优先，缺少显式值时保留已存值，新会话回退导出元数据。两者都没有时，个人 @ 无法判断并发出 `W_SELF_ID_MISSING`；`target = All` 仍算“@我”。昵称不参与身份判断。

对 @我 的消息，如果它没有被同话题的 Todo、Announcement、Decision 结论引用为证据，就生成一条 `MentionMe` 结论，证据就是这条消息本身（quote 取渲染文本的前 60 个字符）。

### 6.2 证据校验（系统不变量：LLM 提出，Rust 验证）

签名（冻结给同学 A）：

```rust
// crates/engine/src/verify/mod.rs —— 纯函数，数据库访问通过 trait 注入
pub trait MessageLookup {
    /// 消息在给定渲染视图下的文本；消息不存在返回 None；撤回消息返回 Some("") 并由 is_recalled 标出
    fn rendered(&self, id: &MessageId, profile: RenderProfile) -> Option<String>;
    fn is_recalled(&self, id: &MessageId) -> bool;
}
pub struct DraftEvidence { pub message_id: MessageId, pub quote: String }
pub struct VerifyInput<'a> {
    pub kind: InsightKind,
    pub deadline_raw: Option<&'a str>,
    pub evidence: &'a [DraftEvidence],
    pub profile: RenderProfile,
}
pub enum VerifyFailure {
    NoEvidence,
    MessageNotFound { index: usize },
    RecalledMessage { index: usize },
    QuoteTooShort { index: usize },
    QuoteNotFound { index: usize },
    DeadlineRawNotInEvidence,
}
pub struct VerifyReport {
    pub status: VerificationStatus,
    pub evidence_ok: Vec<bool>,          // 与 evidence 一一对应
    pub failures: Vec<VerifyFailure>,
}
pub fn verify(input: &VerifyInput, lookup: &dyn MessageLookup) -> VerifyReport;

/// 忽略空白（char::is_whitespace，包括全角空格）后查找子串；返回原文中的字符下标区间 [start, end)。
pub fn find_quote(haystack: &str, quote: &str) -> Option<(usize, usize)>;
```

校验规则：

| # | 检查 | 失败类型 |
|---|---|---|
| 0 | evidence 非空 | `NoEvidence` |
| 1 | `message_id` 存在于数据库 | `MessageNotFound` |
| 1b | 消息未被撤回 | `RecalledMessage` |
| 2a | quote 去空白后至少 3 个字符 | `QuoteTooShort` |
| 2b | quote 是 `render(message, profile)` 的子串，只忽略空白差异，不做模糊匹配 | `QuoteNotFound` |
| 3 | 若有 `deadline_raw`，它必须出现在至少一条证据消息的渲染文本中（同样只忽略空白） | `DeadlineRawNotInEvidence` |

判定：
- `failures` 为空 → `Verified`。
- `kind = TopicSummary`，且至少有一条证据通过 → `Unverified`（进收件箱，但带“未验证”标记）。
- 其他情况 → `Rejected`。**高风险类型**（MentionMe、Todo、Announcement、Decision，以及任何带截止日期的结论）只可能是 `Verified` 或 `Rejected`。

调用方（engine 的 Verify 动作）负责重试：对 `Rejected` 的条目，把失败原因发回 LLM **重试一次**；仍失败则保存为 `Rejected`，计入 `stats.insights.rejected` 和 `W_INSIGHT_REJECTED`，GUI 统计面板展示这个数字。

## 7. 优先级与反馈

“重要”的定义：用户漏掉这条信息，可能承担行动成本、时间成本，或显著的信息损失。

### 7.1 分层（规则决定，反馈不改变层级）

| 层级 | 条件（满足任一） |
|---|---|
| **P0 必须处理** | `MentionMe` 且 `needs_action` 的 p ≥ 0.5；`Todo` 且 `assignee ∈ {me, all}`；除 `TopicSummary` 外，任何带截止日期（`deadline ≠ None`）的结论 |
| **P1 应该知道** | 不需要行动的 `MentionMe`；`Announcement`；`Decision`；`assignee = unknown` 的 `Todo` |
| **P2 可能感兴趣** | 非闲聊话题的 `TopicSummary`；`assignee = other` 的 `Todo` |
| **P3 闲聊** | 闲聊话题（`is_chitchat ≥ 0.5`）的 `TopicSummary` |

@全体成员 视同 @我。

### 7.2 层内先验分（冷启动规则打分）

`rank_prior = 1.0·deadline_proximity + 0.8·confidence + 0.5·urgency_norm + 0.3·recency`

- `deadline_proximity = 1 / (1 + 距截止的天数)`，没有截止日期或已过期取 0；
- `confidence` 缺失时取 0.5；
- `urgency_norm = Jev urgency score / 2`；
- `recency = exp(−距最后一条证据的小时数 / 24)`。

### 7.3 反馈：有界的在线偏好校准

这是**在线偏好校准，不是训练推荐模型**：只调整少数几个特征的权重，幅度有上限，并且会向先验回归。

- 特征：`kind:<kind>`、`topic:<topic_id>`、`sender:<首条证据的发送者>`、`has_deadline:<bool>`。
- 每个结论只有一个当前有效评价：`y = +1`（有用）或 `−1`（不重要）。相同评价重复提交无变化；切换评价替换旧值，不算两次有效反馈。对有效评价涉及的每个特征，初始更新式为：
  `w ← clamp((1 − λ)·w + η·y, −0.5, 0.5)`，其中 `η = 0.2`，`λ = 0.05`。
- 每次 `analyze` 开始时，所有权重乘以 `(1 − λ)`，即向 0（先验）回归。
- `rank_score = rank_prior + clamp(mean(命中特征的 w), −0.5, 0.5)`。
- 更换或删除评价后必须依据当前有效集合重算或撤销旧贡献，不能直接再累加一票；具体重算顺序和衰减记录在实现该模块时固定并测试。审计历史不能直接作为有效票集。
- **不变量**：层级只由 §7.1 决定，反馈永远不能把 P0 降级；“不重要”只会让结论在 P0 层内靠后。

### 7.4 收件箱内容（`inbox` 命令）

显示满足以下全部条件的结论：`verification_status ≠ rejected`（带 `--include-rejected` 时不限，仅供评估），`lifecycle = open`（带 `--include-resolved` 时不限），并且满足下列之一：

1. `priority = P0`：一直显示，直到被 resolve；截止日期已过的标记为“已过期”；
2. `priority = P1` 且截止日期未过；
3. 至少一条证据消息晚于 `last_reviewed`（即“自上次查看以来”的窗口）。

`view_cursor: Option<Cursor>` 是本次快照中连续 `done/skipped` 前缀的末尾，不直接取 `last_analyzed`；不能越过 `pending/failed`。没有安全位置时返回 null，GUI 禁用标为已读。CLI 检查合法位置，但不把 Cursor 当成用户确实阅读过界面的凭证。

`--all` 关闭上述展示窗口，生命周期/校验过滤仍独立生效。元数据、计数、topic、insight 和可选 HTML 均来自同一读快照；排序按 P0–P3、层内分数降序、InsightId 升序。查询不写数据库。

已知且接受的行为：展示窗口按消息的 `sent_at` 判断。回填进来的、早于 `last_reviewed` 的消息，其 P2/P3 结论不会出现在默认视图中（P0 与未过期的 P1 不受影响）。

GUI 三栏：“需要你处理”= P0，“值得知道”= P1，“其他话题”= P2 + P3（P3 默认折叠）。

## 8. 智能体控制器

### 8.1 形式

有界控制循环：每一步 ① 计算 `AgentObservation`；② 规则算出**允许的动作集合**；③ 只有一个候选 → 直接执行（`method=rule`）；多个候选 → 用 Jev `choice` 在其中选择（`method=jev`），Jev 失败时取规则默认项（`method=fallback`）；④ 执行，提交检查点，写决策日志。

给 Jev 的 state 使用**分档后的类别**，不放原始数字（Jev 不擅长数值比较）：积压 `none/small/medium/large`，交错度 `low/medium/high`，以及候选动作的自然语言描述、相关话题标题。数值阈值留在规则里。

### 8.2 决策规则（按顺序匹配第一条）

| # | 条件 | 允许的动作 | 规则默认项 |
|---|---|---|---|
| R0 | `steps_taken ≥ max_steps` / `cost_usd ≥ budget_usd` / 收到取消 | `Finish(MaxSteps / BudgetExceeded / Cancelled)` | — |
| R1 | `pending_verification > 0` | `Verify(全部草稿结论)` | — |
| R2a | `0 < pending ≤ direct_max(60)` 且 `interleave < 0.2` | `AnalyzeDirect` | — |
| R2b | `0 < pending ≤ direct_max` 且 `interleave ≥ 0.2` | `AnalyzeDirect`、`Segment` | `Segment` |
| R2c | `pending > direct_max` | `Segment`（每步最多处理 300 条） | — |
| R3 | `dirty_topics > 0` 或 `merge_candidates > 0` | `AnalyzeTopic(信号最强的脏话题)`，以及存在合并候选时的 `MergeTopics(最佳一对)` | 互相指向的回复边 ≥ 3 时选 `MergeTopics`，否则选 `AnalyzeTopic` |
| R4 | 以上都不满足 | `Finish(Done)` | — |

- `interleave`：待切分消息中，已解析的回复边里，源消息和目标消息之间夹着 ≥ 3 条其他人消息的边所占的比例。
- “信号最强的脏话题”：按话题内消息的 `max(p_todo, p_announcement, needs_action)` 以及 @我 数量排序。
- `pending_messages` 只统计本次运行**有资格处理**的待切分消息（受 `--since/--until` 限制），否则 R2c 会对永远不会处理的消息反复选择 `Segment`。
- **草稿结论**：`AnalyzeTopic` / `AnalyzeDirect` 的输出先作为草稿保存在本次运行的内存中，`pending_verification` 就是草稿数。`Verify` 处理全部草稿：通过的写入数据库（`verified` 或 `unverified`），不通过的重试一次，仍不通过的以 `rejected` 写入。之后草稿数归零。已经存进数据库的 `unverified` 结论**不算**草稿，不会再次触发 R1。进程在两步之间被杀时草稿丢失，对应话题仍是脏话题，下次重新抽取（有缓存，不会重复付费）。
- 单个话题抽取失败（重试后仍失败）：该话题本次的待分析消息 `analysis_state` 标为 `failed`，不再算作脏话题，本次运行最终状态为 `partial`。**下次 `analyze` 开始时**，`failed` 恢复为 `pending`，重试一次。

### 8.3 有界执行与待验证的终止性

必须实施的边界是正整数 `max_steps`、请求超时和有限次重试、取消检查，以及在请求前预留的费用预算。每个已执行动作（含失败动作）计入 steps；达到上限后结束为 partial，保存已有检查点。这些约束仍需在控制器实现与测试中逐项验证，不能由文档直接视为已实现。

早期草稿提出按 `(待切分消息数, 活跃话题数, 合并候选数 + 脏话题数, 草稿结论数)` 字典序递减，但当前状态定义不足以成立：AnalyzeTopic 成功后尚未 Verify 提交，消息仍是 pending，脏话题数未减少而草稿数增加。因此撤回“每个动作都严格减小、无硬上限也必终止”的断言。后续实现必须明确抽取中/待校验状态、空抽取结果和合并后的候选更新，再给出与实际状态机一致的论证；本轮不声称完成该证明。

### 8.4 检查点

- `Segment` 每处理完一批 burst 提交一次（`topic_messages`）。
- 每个话题的 `AnalyzeTopic` + 对应的 `Verify` 完成后提交：结论、证据、该话题消息的 `analysis_state = done` 与 `analyzed_run`、`run_checkpoints` 一条记录。
- 进程中途被杀：已提交的部分保留；下次 `analyze` 重新计算“待切分消息”和“脏话题”，从断点继续。结合缓存，重跑已完成的步骤不会重复付费。

## 9. 缓存、用量与隐私

**缓存键**：`BLAKE3(stage ‖ provider ‖ model ‖ 规范化的模型参数 ‖ prompt_template_hash ‖ schema_hash ‖ normalized_input_hash ‖ render_profile ‖ ALGO_VERSION)`。
- 提示词模板、schema 或渲染规则一改，缓存键就变，旧结果自动失效。
- `normalized_input_hash` 对 JSON 做键排序后计算哈希。

**用量**：每次调用记录输入/输出 token 和估算费用（按 `config.toml` 中的单价计算；Jev 只收输入 token 的费用）。缓存命中计入 `cache_hits`，不计费用。

**隐私**：
- 不做脱敏（Q-DEC-2 已决定）；图片永不上传；
- 程序本身在本机运行、数据库只在本机，但聊天文本会发送给 Jev（TypeSafe）和 DeepSeek 两个云服务，README 和 GUI 首次运行时都要明确告知；
- 数据库、导出文件放在 `.gitignore` 覆盖的目录，仓库只放合成数据。

## 10. 降级路径

| 故障 | 降级 |
|---|---|
| Jev 不可用（未配置 key、连续 3 次失败） | 本次运行切换为 `LlmDecider`，发 `W_DECIDER_FALLBACK` |
| 话题切分整体失败 | `--strategy b1` 路径：按时间间隔 + 固定长度切块 |
| 截止日期规范化不确定 | 只保留 `raw`，`bound_date=None` |
| LLM 输出无效 | 重试一次，仍失败则该话题的待分析消息标为 `failed`，运行最终为 `partial`，下次运行重试 |
| GUI 不可用 | `analyze --html` / `inbox --html` 生成静态页面演示 |
