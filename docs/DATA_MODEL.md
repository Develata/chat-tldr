# 数据模型（DATA_MODEL）

> 用途：冻结跨模块共享的四个类型（UnifiedMessage、Insight、CliEvent、AgentAction），并给出 SQLite 表结构草案。
> 读者：全体成员。qce 适配器、GUI、eval 都只依赖这里定义的类型。

## 0. 冻结规则

- 本文 §2–§5 的类型实现在 `crates/core`，**冻结后只有 @Develata 能修改**。
- 任何修改必须：① 升级 `schema_version`（见 [CLI_PROTOCOL.md](CLI_PROTOCOL.md) §6 版本规则）；② 同步更新本文；③ 在群里通知。
- 其他人发现类型不够用：开 issue 或在群里提，不要自己改 `crates/core`（CODEOWNERS 会拦）。
- Rust 结构体只是草案，放在文档里；实际源码由 @Develata 在第 1 天上午写入 `crates/core`。
- `crates/core` 里还有一些辅助类型（`ImportBatch`、`ChatMeta`、各事件的 payload 结构体），定义见 [ARCHITECTURE.md](ARCHITECTURE.md) §4.1 和 [CLI_PROTOCOL.md](CLI_PROTOCOL.md) §3，同样只由 @Develata 修改。

## 1. 通用约定

| 约定 | 内容 |
|---|---|
| ID 类型 | `ChatId`、`PersonId`、`MessageId`、`TopicId`、`InsightId`、`RunId` 都是 `String` newtype，JSON 中就是字符串 |
| 时间 | 统一用 RFC 3339 字符串，带时区偏移，例如 `"2026-09-26T21:05:33+08:00"`。Rust 侧类型 `chrono::DateTime<FixedOffset>`。默认时区 `+08:00`，可配置 |
| 概率/置信度 | `f32`，范围 [0, 1] |
| 枚举序列化 | JSON 中用 `snake_case` 字符串（如 `"mention_me"`），Priority 例外，用 `"P0"`…`"P3"` |
| 未知字段 | 反序列化时**忽略未知字段**（不要用 `deny_unknown_fields`），以支持 MINOR 版本向后兼容 |
| 可空字段 | JSON 中可为 `null`，Rust 中为 `Option<T>` |

### 1.1 ID 生成规则

| ID | 格式 | 生成方式 |
|---|---|---|
| `ChatId` | `qq:group:<peer>` / `qq:private:<peer>` | `<peer>` 取 QCE `chatInfo.peerUid`，缺失时取 `peerUin`。**TODO(Q-QCE-3)**：群聊导出时 peerUid/peerUin 的实际取值需用真实样本确认 |
| `PersonId` | `qq:<uid>` | 取 QCE `sender.uid`（QQNT 的 uid，稳定）。**不使用昵称、群名片** |
| `MessageId` | `m_<16 位 hex>` | `blake3(chat_id + "\n" + source_identity)` 的前 16 位 hex。同一条消息无论导入多少次，ID 都相同（幂等的基础） |
| `TopicId` | `t_<12 位 hex>` | 创建时由 `blake3(chat_id, run_id, 创建序号)` 生成 |
| `InsightId` | `i_<12 位 hex>` | 创建时由 `blake3(chat_id, run_id, topic_id, 序号)` 生成；之后的更新保持 ID 不变 |
| `RunId` | `r_<yyyymmddThhmmss>_<4 位 hex>` | 每次 CLI 调用生成一个 |

### 1.2 source_identity（消息来源身份）

去重依据。按优先级取第一个可用的：

1. `qce:<id>`：QCE `messages[].id` 非空时使用；
2. `seq:<seq>@<timestamp>`：`seq` 与 `timestamp` 都非空时使用；
3. `hash:<blake3(sender.uid, timestamp, content.text)>`：兜底。

数据库对 `(chat_id, source_identity)` 建唯一约束。`seq` **不保证连续**，只作为游标提示，不作为“没有漏消息”的证明。

### 1.3 Cursor（游标）

```rust
/// 消息在一个 chat 内的全序位置。序列化为字符串 "<sent_at_ms>:<ordinal>"。
pub struct Cursor { pub sent_at_ms: i64, pub ordinal: i64 }
```

- `ordinal` 是 `messages` 表的自增主键 `pk`。排序键 `(sent_at_ms, ordinal)` 是全序：同一毫秒的两条消息也能比较。
- 每个 chat 有三个游标（语义见 [PIPELINE.md](PIPELINE.md) §1）：`last_ingested`、`last_analyzed`、`last_reviewed`。
- GUI 只把 Cursor 当作**不透明字符串**原样回传（`mark-read --up-to`），不要解析它。

### 1.4 RenderProfile（渲染视图）

**证据校验的对象是“发给模型的那份文本”，不是原始文本。** 因为脱敏会替换昵称，图片会变成 `[图片]`，合并转发会被展开。

```rust
/// 序列化为 "r1" 或 "r1-redact"。
pub struct RenderProfile { pub version: u16, pub redact: bool }
```

- `render(msg, profile) -> String` 是 engine 中的纯函数，确定性输出。它既用来生成发给 Jev/LLM 的文本，也用来做证据校验。
- 规则改动必须升级 `version`。`version` 同时进入缓存键和每条 Evidence。
- 脱敏代号的形式为 `⟦U3⟧`（带定界符，见 PIPELINE §2.2）。
- 显示给用户时使用 `redact=false` 的渲染结果；模型生成的文本（话题标题、结论 title/summary、quote）由 CLI 在输出时把代号还原为本地名字（见 PIPELINE §2.2“还原”与 CLI_PROTOCOL 中的 `EvidenceView`）。

---

## 2. UnifiedMessage

qce 适配器的输出，也是 engine 的输入。一个 `UnifiedMessage` 对应 QCE `messages[]` 中的一个元素。

| 字段 | 类型 | 含义 | 约束 |
|---|---|---|---|
| `id` | `MessageId` | 内部稳定 ID | 按 §1.1 生成，适配器负责计算 |
| `chat_id` | `ChatId` | 所属会话 | 同一文件中所有消息相同 |
| `sender` | `PersonId` | 发送者 | 只来自 `sender.uid`；系统消息可为 `qq:system` |
| `sender_display` | `String` | 导入时的展示名 | 可变属性，仅用于显示和别名表，**不参与身份判断** |
| `sent_at` | `DateTime` | 发送时间 | 来自 QCE `timestamp`（毫秒），转为配置时区 |
| `text` | `String` | 基础文本（未脱敏） | 由适配器按 §2.1 规则从 `content.elements` 生成；撤回消息为空串 |
| `mentions` | `Vec<Mention>` | @ 列表 | 保留全部，包括 @全体成员 |
| `reply_to` | `Option<ReplyRef>` | 回复引用 | 允许悬空（被引用消息不在本次导出中） |
| `attachments` | `Vec<Attachment>` | 附件元数据 | 不含二进制内容；图片永不上传 |
| `forward` | `Option<ForwardBundle>` | 合并转发 | 递归深度 ≤ 2 |
| `recalled` | `bool` | 是否已撤回 | `true` 时 `text` 为空、`mentions` 为空 |
| `system` | `bool` | 是否系统消息 | 系统消息保留，不一刀切删除 |
| `source` | `SourceMeta` | 来源元数据 | 见下 |

### 2.1 text 的生成规则（适配器负责）

按 `content.elements[]` 的顺序拼接：

| QCE element `type` | 输出 |
|---|---|
| `text` | 原文 `data.text` |
| `at` | `@<data.name>`（@全体成员 输出 `@全体成员`） |
| `face` | 有名字 → `[表情:<name>]`，否则 `[表情]` |
| `market_face` | `[表情:<data.name>]` |
| `image` | `[图片]` |
| `video` / `audio` / `file` | `[视频]` / `[语音]` / `[文件:<文件名>]` |
| `reply` | 不输出文本，只写入 `reply_to` |
| `forward` | 输出 `[合并转发:<title>]`，内容写入 `forward` |
| `system` / `json` / `location` / 其他 | `[<type>]`，**TODO(Q-QCE-2)**：各类型 `data` 的字段待按真实样本确认，确认后再决定是否输出其中的文字 |

> 依据：QCE `qq-chat-export-server/src/parser/simple_parser.rs`、`qq-chat-export-core/src/types.rs`，commit `7fcca88`（2026-09-11）。

### 2.2 子类型

```rust
pub struct Mention {
    pub target: MentionTarget,
    pub display: String,          // 导入时 @ 后面的名字，只用于显示
}
pub enum MentionTarget {
    User { uid: Option<String>, uin: Option<String> },  // 见下方说明
    All,                                                  // @全体成员
}

pub struct ReplyRef {
    pub source_message_id: String,      // QCE reply element 的 data.referencedMessageId 或 data.messageId
    pub resolved: Option<MessageId>,    // 被引用消息已导入时填入；可在后续导入时补全
}

pub struct Attachment {
    pub kind: AttachmentKind,           // image | video | audio | file | other
    pub name: Option<String>,
    pub size: Option<u64>,
}

pub struct ForwardBundle {
    pub title: String,
    pub messages: Vec<ForwardedMessage>,   // 最多展开到深度 2；更深的只保留 title
}
pub struct ForwardedMessage {
    pub sender_display: String,
    pub sent_at: Option<DateTime<FixedOffset>>,
    pub text: String,
    pub forward: Option<Box<ForwardBundle>>,
}

pub struct SourceMeta {
    pub format: String,               // "qce-json"
    pub identity: String,             // §1.2 的 source_identity
    pub qce_id: Option<String>,
    pub qce_seq: Option<String>,      // QCE 的 seq；注意与 CliEvent.seq 无关
    pub qce_type: Option<String>,     // QCE 的 type 字段，如 "type_1"
    pub file_hash: String,            // 来源导出文件的 BLAKE3，"blake3:<hex>"
}
```

**关于 Mention 的 uid/uin（重要）**：QCE 的 `at` 元素中，`uid = atNtUid ?? atUid`，而 `atUid` 实际是 QQ 号（uin），缺失时为 `"unknown"`；另有 `data.uin` 字段。适配器应把能识别的值分别填入 `uid` / `uin`。判断“@我”时同时比较 selfUid 和 selfUin（见 PIPELINE §6.1）。**TODO(Q-QCE-1)**：用真实样本确认 `mentions[].uid` 的取值形态。

**关于 ForwardedMessage**：转发的内部消息不是独立的数据库行，没有自己的 `MessageId`。它们在 `render` 时以缩进形式拼进父消息的文本，因此证据引用转发内容时，`message_id` 指向父消息。**TODO(Q-QCE-4)**：`forward` 元素 `data.messages` 的内部结构待真实样本确认。

### 2.3 示例

```json
{
  "id": "m_3f9a1c0b7d2e4a51",
  "chat_id": "qq:group:u_8KxZ2example",
  "sender": "qq:u_A1b2C3example",
  "sender_display": "班长-小王",
  "sent_at": "2026-09-23T21:05:33+08:00",
  "text": "@全体成员 周五前把实验报告交到课代表那里[图片]",
  "mentions": [{ "target": { "type": "all" }, "display": "全体成员" }],
  "reply_to": null,
  "attachments": [{ "kind": "image", "name": "IMG_001.jpg", "size": 204811 }],
  "forward": null,
  "recalled": false,
  "system": false,
  "source": {
    "format": "qce-json",
    "identity": "qce:7550661840517106706",
    "qce_id": "7550661840517106706",
    "qce_seq": "1024",
    "qce_type": "type_1",
    "file_hash": "blake3:9b1c…"
  }
}
```

`MentionTarget` 的 JSON 形式：`{"type":"all"}` 或 `{"type":"user","uid":"u_…","uin":"12345"}`。

---

## 3. Insight

一条“结论”：系统认为用户需要知道或处理的一件事。每条 Insight **必须**带证据。

| 字段 | 类型 | 含义 | 约束 |
|---|---|---|---|
| `id` | `InsightId` | 稳定 ID | 更新时不变 |
| `chat_id` | `ChatId` | 所属会话 | |
| `kind` | `InsightKind` | 类型 | 见 §3.1 |
| `title` | `String` | 一行标题 | ≤ 40 个汉字；CLI 输出时已还原脱敏代号 |
| `summary` | `String` | 一两句说明 | ≤ 200 个汉字；CLI 输出时已还原脱敏代号 |
| `priority` | `Priority` | 层级 P0–P3 | 由规则决定，**反馈不能改变层级** |
| `rank_score` | `f32` | 层内排序分 | 规则先验 + 个性化修正（修正幅度限制在 ±0.5），见 PIPELINE §7 |
| `confidence` | `Option<f32>` | 该结论成立的模型概率 | `MentionMe`（规则产生）为 `1.0`；`Todo` / `Announcement` 取各证据消息对应的 Decider noul（`n{i}_todo` / `n{i}_announcement`，Jev 或 LlmDecider）中最大的 `p_yes`；`Decision`、`TopicSummary` 为 `null` |
| `assignee` | `Assignee` | 这件事是谁的 | `me` / `all` / `other` / `unknown` |
| `deadline` | `Option<TemporalConstraint>` | 截止时间 | 是属性，不是独立 kind |
| `evidence` | `Vec<Evidence>` | 证据 | 非空；高风险类型必须通过校验 |
| `topic_id` | `Option<TopicId>` | 所属话题 | 正常流程下总有值 |
| `verification_status` | `VerificationStatus` | 证据校验结果 | `verified` / `unverified` / `rejected` |
| `lifecycle` | `Lifecycle` | 用户处理状态 | `open` / `done` / `dismissed` |
| `created_in_run` | `RunId` | 创建它的运行 | |
| `updated_at` | `DateTime` | 最后更新时间 | |

### 3.1 枚举

```rust
pub enum InsightKind { MentionMe, Todo, Announcement, Decision, TopicSummary }
pub enum Priority { P0, P1, P2, P3 }
pub enum Assignee { Me, All, Other, Unknown }
pub enum VerificationStatus { Verified, Unverified, Rejected }
pub enum Lifecycle { Open, Done, Dismissed }
```

- `MentionMe`：@我（或 @全体成员）的消息，且**没有**被同一话题中的 Todo/Announcement/Decision 覆盖（避免同一件事产生两条结论）。
- `verification_status` 与 `lifecycle` 是两个独立维度：前者是“证据是否可信”，后者是“用户是否处理完”。
- `rejected` 的 Insight 只存库、计数，**永远不进收件箱**；`unverified` 只允许出现在 `TopicSummary` 上，GUI 显示“未验证”标记。

### 3.2 TemporalConstraint

```rust
pub struct TemporalConstraint {
    pub raw: String,                     // 原文表达式，如 "下周三前"；必须出现在证据原文中
    pub relation: TemporalRelation,      // before | at | after
    pub bound_date: Option<NaiveDate>,   // 规范化失败时为 None
    pub bound_time: Option<NaiveTime>,   // "周三前" 这种没有具体时刻的为 None
    pub granularity: Granularity,        // minute | hour | half_day | day | week | unknown
    pub anchor: DateTime<FixedOffset>,   // 解释相对时间的参照点 = 证据消息的 sent_at
    pub normalized_by: NormalizedBy,     // rule | llm | none
    pub confidence: f32,
}
```

“下周三前”规范化为 `relation=before, bound_date=<那个周三>, bound_time=None, granularity=day`，**不要**强行补成某个具体时刻。

### 3.3 Evidence

```rust
pub struct Evidence {
    pub message_id: MessageId,
    pub quote: String,                 // 必须是 render(message, render_profile) 的子串（只忽略空白差异）
    pub render_profile: RenderProfile, // 生成该 quote 时模型看到的渲染视图
}
```

### 3.4 示例

```json
{
  "id": "i_7c2d9e01ab34",
  "chat_id": "qq:group:u_8KxZ2example",
  "kind": "todo",
  "title": "周五前交实验报告",
  "summary": "班长通知全体成员周五前把实验报告交给课代表。",
  "priority": "P0",
  "rank_score": 2.35,
  "confidence": 0.91,
  "assignee": "all",
  "deadline": {
    "raw": "周五前",
    "relation": "before",
    "bound_date": "2026-09-25",
    "bound_time": null,
    "granularity": "day",
    "anchor": "2026-09-23T21:05:33+08:00",
    "normalized_by": "rule",
    "confidence": 0.9
  },
  "evidence": [
    {
      "message_id": "m_3f9a1c0b7d2e4a51",
      "quote": "周五前把实验报告交到课代表那里",
      "render_profile": "r1-redact"
    }
  ],
  "topic_id": "t_a19c3b0d77e2",
  "verification_status": "verified",
  "lifecycle": "open",
  "created_in_run": "r_20260926T090112_4b1e",
  "updated_at": "2026-09-26T09:01:40+08:00"
}
```

---

## 4. CliEvent

CLI 在 stdout 上输出的每一行都是一个 CliEvent（JSON Lines）。完整的事件列表、payload 字段和示例见 [CLI_PROTOCOL.md](CLI_PROTOCOL.md) §3，这里只冻结信封。

| 字段 | 类型 | 含义 | 约束 |
|---|---|---|---|
| `schema_version` | `String` | 协议版本 `"MAJOR.MINOR"` | 当前 `"1.0"` |
| `run_id` | `RunId` | 本次 CLI 调用的 ID | 同一进程输出的所有行相同 |
| `seq` | `u64` | 行序号 | 从 0 开始，每行 +1，无空洞 |
| `event` | `String` | 事件类型 | 见下方枚举 |
| `payload` | `object` | 事件内容 | 结构由 `event` 决定 |

```rust
#[serde(tag = "event", content = "payload", rename_all = "snake_case")]
pub enum EventBody {
    Progress(ProgressPayload),
    Decision(DecisionPayload),
    Chat(ChatPayload),
    Message(MessagePayload),
    Topic(TopicPayload),
    Insight(InsightPayload),
    Inbox(InboxPayload),
    Stats(StatsPayload),
    Ack(AckPayload),
    JevAnswer(JevAnswerPayload),
    Warning(WarningPayload),
    Error(ErrorPayload),
    Done(DonePayload),
}
pub struct CliEvent {
    pub schema_version: String,
    pub run_id: RunId,
    pub seq: u64,
    #[serde(flatten)]
    pub body: EventBody,
}
```

GUI 反序列化时遇到未知的 `event` 值应忽略该行（并写日志），不能崩溃。

```json
{"schema_version":"1.0","run_id":"r_20260926T090112_4b1e","seq":0,"event":"progress","payload":{"stage":"import","current":0,"total":1532,"message":"读取导出文件"}}
```

---

## 5. AgentAction

智能体控制器每一步选择的动作。决策规则与停止条件见 [PIPELINE.md](PIPELINE.md) §8。

```rust
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AgentAction {
    /// 积压少且交错低：跳过聚类，一次 LLM 调用直接产出话题和结论
    AnalyzeDirect { chat_id: ChatId, range: MessageRange },
    /// 对待处理消息做三层话题切分（burst → 候选 → Jev 归属）
    Segment { chat_id: ChatId, range: MessageRange },
    /// 对一个有新消息的话题做 LLM 抽取（标题、摘要、结论）
    AnalyzeTopic { topic_id: TopicId },
    /// 校验待验证的结论（失败的重试一次）
    Verify { insight_ids: Vec<InsightId> },
    /// 合并被判定为同一话题的多个话题
    MergeTopics { into: TopicId, from: Vec<TopicId> },
    /// 结束本次运行
    Finish { reason: FinishReason },
}

pub struct MessageRange { pub after: Option<Cursor>, pub up_to: Cursor }  // 左开右闭

pub enum FinishReason { Done, MaxSteps, BudgetExceeded, Cancelled, Error }
```

控制器的观测（写入决策日志，也是 Jev 的 state 来源）：

```rust
pub struct AgentObservation {
    pub pending_messages: u32,          // 尚未分配话题的消息数（不含撤回消息）
    pub interleave: f32,                // 交错度：跨 burst 的回复/@ 边占全部边的比例
    pub active_topics: u32,
    pub dirty_topics: u32,              // 有新消息但未重新抽取的话题数
    pub pending_verification: u32,      // 本次运行已抽取、尚未校验的草稿结论数（不含库中已保存的 unverified 结论）
    pub merge_candidates: u32,
    pub steps_taken: u32,
    pub cost_usd: f64,
}
```

JSON 示例：

```json
{ "action": "analyze_topic", "topic_id": "t_a19c3b0d77e2" }
{ "action": "finish", "reason": "done" }
```

---

## 6. SQLite 表结构草案

> 只有 CLI（`crates/engine` 的 store 模块）访问数据库，GUI 永远不直接读写。
> 连接参数：`journal_mode=WAL`、`busy_timeout=5000`、`foreign_keys=ON`；所有写操作在事务中完成。
> 迁移：`meta.db_version` 整数 + 启动时按序执行迁移脚本。

```sql
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);   -- db_version 等

CREATE TABLE chats (
  chat_id        TEXT PRIMARY KEY,
  kind           TEXT NOT NULL,            -- group | private
  display_name   TEXT NOT NULL,
  self_uid       TEXT, self_uin TEXT,      -- 来自导出文件或 --self-uid/--self-uin
  last_ingested  TEXT,                     -- Cursor 字符串
  last_analyzed  TEXT,
  last_reviewed  TEXT,
  updated_at     TEXT NOT NULL
);

CREATE TABLE persons (person_id TEXT PRIMARY KEY, uin TEXT, first_seen TEXT NOT NULL);
CREATE TABLE person_aliases (                 -- 昵称/群名片历史，只是可变属性
  person_id TEXT NOT NULL REFERENCES persons, chat_id TEXT, alias TEXT NOT NULL,
  alias_kind TEXT NOT NULL,                   -- name | nickname | group_card | remark
  first_seen TEXT NOT NULL, last_seen TEXT NOT NULL,
  PRIMARY KEY (person_id, chat_id, alias, alias_kind)
);
CREATE TABLE redaction_codes (                -- 脱敏代号表，只存本地
  chat_id TEXT NOT NULL, original TEXT NOT NULL, code TEXT NOT NULL,
  PRIMARY KEY (chat_id, original), UNIQUE (chat_id, code)
);

CREATE TABLE imports (
  import_id TEXT PRIMARY KEY, file_path TEXT NOT NULL, file_hash TEXT NOT NULL,
  chat_id TEXT NOT NULL, run_id TEXT NOT NULL,
  seen INTEGER NOT NULL, inserted INTEGER NOT NULL, duplicate INTEGER NOT NULL,
  backfilled INTEGER NOT NULL,                -- 插入位置早于 last_analyzed 的消息数
  imported_at TEXT NOT NULL
);

CREATE TABLE messages (
  pk               INTEGER PRIMARY KEY AUTOINCREMENT,   -- Cursor.ordinal
  message_id       TEXT NOT NULL UNIQUE,
  chat_id          TEXT NOT NULL REFERENCES chats,
  source_identity  TEXT NOT NULL,
  sender           TEXT NOT NULL,
  sender_display   TEXT NOT NULL,
  sent_at_ms       INTEGER NOT NULL,
  text             TEXT NOT NULL,
  reply_source_id  TEXT, reply_resolved TEXT,           -- 悬空引用在后续导入时补全
  recalled         INTEGER NOT NULL, system INTEGER NOT NULL,
  body_json        TEXT NOT NULL,                        -- 完整 UnifiedMessage
  analysis_state   TEXT NOT NULL DEFAULT 'pending',      -- pending | done | skipped | failed，见 PIPELINE §1.1
  analyzed_run     TEXT,                                 -- 置为 done 的那次运行
  UNIQUE (chat_id, source_identity)
);
CREATE INDEX idx_messages_order ON messages(chat_id, sent_at_ms, pk);
CREATE TABLE message_mentions (message_id TEXT NOT NULL, target_kind TEXT NOT NULL, uid TEXT, uin TEXT);

CREATE TABLE topics (
  topic_id TEXT PRIMARY KEY, chat_id TEXT NOT NULL,
  title TEXT NOT NULL, title_is_provisional INTEGER NOT NULL,
  state TEXT NOT NULL,                    -- active | closed | merged
  merged_into TEXT, last_message_at TEXT NOT NULL,
  is_chitchat REAL,                       -- Jev noul 概率
  created_in_run TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE TABLE topic_messages (
  topic_id TEXT NOT NULL, message_id TEXT NOT NULL, burst_id TEXT NOT NULL,
  method TEXT NOT NULL,                   -- rule_reply | jev | llm_review | new_topic | direct | similarity
  confidence REAL, run_id TEXT NOT NULL,
  PRIMARY KEY (message_id)
);

CREATE TABLE insights (
  insight_id TEXT PRIMARY KEY, chat_id TEXT NOT NULL, topic_id TEXT,
  kind TEXT NOT NULL, priority TEXT NOT NULL, rank_prior REAL NOT NULL,
  verification_status TEXT NOT NULL, lifecycle TEXT NOT NULL,
  body_json TEXT NOT NULL,                -- 完整 Insight
  created_in_run TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE TABLE evidence (
  insight_id TEXT NOT NULL REFERENCES insights, message_id TEXT NOT NULL,
  quote TEXT NOT NULL, render_profile TEXT NOT NULL, ok INTEGER NOT NULL
);

CREATE TABLE feedback (
  feedback_id INTEGER PRIMARY KEY, insight_id TEXT NOT NULL,
  label TEXT NOT NULL,                    -- useful | not_important
  created_at TEXT NOT NULL
);
CREATE TABLE preference_weights (         -- 个性化修正；见 PIPELINE §7.3
  feature TEXT PRIMARY KEY, weight REAL NOT NULL, n INTEGER NOT NULL, updated_at TEXT NOT NULL
);

CREATE TABLE model_cache (
  cache_key TEXT PRIMARY KEY,             -- BLAKE3，见 PIPELINE §9
  stage TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL,
  response_json TEXT NOT NULL, input_tokens INTEGER, output_tokens INTEGER,
  created_at TEXT NOT NULL
);

CREATE TABLE runs (
  run_id TEXT PRIMARY KEY, command TEXT NOT NULL, chat_id TEXT,
  status TEXT NOT NULL,                   -- running | partial | complete | failed | cancelled
  pid INTEGER, args_json TEXT NOT NULL,
  started_at TEXT NOT NULL, heartbeat_at TEXT NOT NULL, finished_at TEXT
);
CREATE TABLE run_checkpoints (            -- 按话题的检查点
  run_id TEXT NOT NULL, topic_id TEXT NOT NULL, stage TEXT NOT NULL,
  status TEXT NOT NULL, updated_at TEXT NOT NULL, PRIMARY KEY (run_id, topic_id, stage)
);
CREATE TABLE decisions (                  -- 智能体决策日志
  run_id TEXT NOT NULL, step INTEGER NOT NULL,
  observation_json TEXT NOT NULL, allowed_json TEXT NOT NULL,
  chosen_json TEXT NOT NULL, method TEXT NOT NULL,   -- rule | jev | fallback
  probabilities_json TEXT, reason TEXT NOT NULL, created_at TEXT NOT NULL,
  PRIMARY KEY (run_id, step)
);
CREATE TABLE jev_answers (                -- 全部 Jev 概率，供校准曲线
  run_id TEXT NOT NULL, request_key TEXT NOT NULL, question_id TEXT NOT NULL,
  qtype TEXT NOT NULL, answer_json TEXT NOT NULL, confidence REAL, subject_json TEXT NOT NULL
);
CREATE TABLE usage (
  run_id TEXT NOT NULL, stage TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL,
  calls INTEGER NOT NULL, cache_hits INTEGER NOT NULL,
  input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, cost_usd REAL NOT NULL,
  PRIMARY KEY (run_id, stage, provider, model)
);
```

说明：
- `body_json` 存完整的冻结类型，常用字段另外拆列以便查询。以 `body_json` 为准。
- `runs.status = running` 但 `heartbeat_at` 超过 60 秒未更新的，下次启动时标为 `cancelled`（GUI 可能直接杀掉子进程）。
- 同一 chat 同时只允许一个 `analyze` 运行；第二个会得到错误码 `E_RUN_IN_PROGRESS`。
