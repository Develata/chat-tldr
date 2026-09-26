# 任务 A：QCE 适配器 + 截止日期规范化 + 证据校验

> 用途：同学 A 的任务 issue 正文。可以直接复制到 GitHub issue。
> 当前负责人：Codex 主线实现，@Develata 审核。原“同学 A”分工已调整；本文件的接口与验收要求继续作为实现依据。QCE 管理组件的同学任务见 [apps/qce-manager/README.md](../../apps/qce-manager/README.md)。

## 目标

1. `crates/qce`：把 QCE 导出的 JSON 文件解析成 `ImportBatch`（UnifiedMessage 列表 + 会话元数据）。
2. `crates/engine/src/temporal`：把“周五前”“明晚 8 点”这类中文时间表达式规范化为 `TemporalConstraint`。
3. `crates/engine/src/verify`：校验 LLM 给出的证据是否真实存在。

三个模块都是**纯函数**：不读文件、不访问数据库、不发网络请求。所以测试非常好写，只需要准备输入、比较输出。

## 你可以修改的目录

- `crates/qce/`
- `crates/engine/src/temporal/`
- `crates/engine/src/verify/`

其他目录（尤其是 `crates/core`）不要改。发现类型不够用 → 在群里或 issue 里告诉 @Develata。

## 1. QCE 适配器

**输入**：QCE 单文件 JSON 的字节（`&[u8]`）。顶层结构：`metadata`、`chatInfo`、`statistics`、`messages[]`。

**输出**：`ImportBatch`，接口定义见 [ARCHITECTURE.md](../ARCHITECTURE.md) §4.1，字段定义见 [DATA_MODEL.md](../DATA_MODEL.md) §1–§2。

```rust
pub fn parse_qce_json(bytes: &[u8], opts: &QceOptions) -> Result<ImportBatch, QceError>;
```

**要点**
- 消息 ID：`source_identity` 按优先级取 `qce:<id>` → `seq:<seq>@<timestamp>` → `hash:<…>`；`MessageId` = `m_` + `blake3(chat_id + "\n" + source_identity)` 前 16 位 hex（DATA_MODEL §1.1–§1.2）。
- `text` 按 DATA_MODEL §2.1 的表格从 `content.elements[]` 拼接。
- `mentions`：QCE `at` 元素中 `uid` 可能是 NT uid，也可能是 QQ 号，还可能是 `"unknown"`；@全体成员 的 `uid` 为 `"all"`（依据 QCE `simple_parser.rs`）。
- `reply_to.source_message_id`：优先取 reply 元素的 `data.referencedMessageId`，其次 `data.messageId`。`resolved` 留空（由 import 补全）。
- 撤回消息：`recalled=true`，`text=""`，`mentions=[]`。
- 合并转发：递归展开，最大深度 2（QCE 自己最多展开 3 层，我们只取前 2 层）。
- 未知元素类型：输出 `[<type>]`，并往 `warnings` 里加一条，**不要 panic**。
- 反序列化时忽略未知字段；任何字段缺失都不能 panic，用 `Option` 或默认值处理。

**验收标准**
- [ ] 每种元素类型（text、at、@全体成员、face、market_face、image、video、audio、file、reply、forward、system、未知类型）至少一个单元测试
- [ ] 撤回消息、缺 `selfUid`、缺 `id` 回退到 seq、`seq` 也缺时回退到 hash，各一个测试
- [ ] 同一个文件解析两次，得到完全相同的 `MessageId` 列表
- [ ] 在你自己的真实导出上运行零 panic（真实文件**不要提交**）
- [ ] 把真实样本中观察到的字段形态写进 [OPEN_QUESTIONS.md](../OPEN_QUESTIONS.md) 的 Q-QCE-1 ~ Q-QCE-4（只写字段结构，不写聊天内容）

## 2. 截止日期规范化 `temporal`

```rust
pub fn normalize(raw: &str, anchor: DateTime<FixedOffset>) -> TemporalConstraint;
```

- `anchor` 是这条消息的发送时间，所有相对表达都相对它计算。
- 规范化失败时返回 `bound_date=None, bound_time=None, granularity=Unknown, normalized_by=None`，**不要猜，不要 panic**。
- 一周从周一开始。

**规则表**

| 表达 | 规则 | granularity |
|---|---|---|
| 今天 / 今晚 / 今早 | anchor 当天；今晚、今早不给具体时刻 | day / half_day |
| 明天 / 明晚 / 明早 / 后天 / 大后天 | anchor + 1 / 1 / 1 / 2 / 3 天 | day / half_day |
| 周X / 星期X / 礼拜X（X = 一…六、日、天） | anchor 当天或之后最近的一个星期 X（当天就是星期 X 时取当天） | day |
| 这周X / 本周X | anchor 所在周的星期 X（可能已经过去，此时 confidence 降到 0.5） | day |
| 下周X / 下个星期X | anchor 所在周的下一周的星期 X | day |
| X点 / X点半 / X点Y分 / X:Y | 与前面的日期组合；上午、早上 → 原值；下午、晚上 → X < 12 时加 12；中午 12 点 → 12:00 | hour / minute |
| 单独的 X 点（没有上午、下午） | X 在 1–6 → 当作下午（+12），7–11 → 原值；confidence 0.6 | hour |
| X 天后 / X 天内 | anchor + X 天 | day |
| M 月 D 日 / M 月 D 号 / D 号 | 当年（D 号取当月）；如果早于 anchor 超过 7 天 → 下一年（下一月） | day |
| 前 / 之前 / 以前 / 内 / 以内 / 截止 / 截至 / ddl | relation = before | — |
| 后 / 之后 / 以后 | relation = after | — |
| 其他 | relation = at | — |

数字支持阿拉伯数字，以及一、二、两、三…十、十一…三十一。

**必须通过的例子**（anchor = 2026-09-23 周三 21:00 +08:00）

| raw | relation | bound_date | bound_time | granularity |
|---|---|---|---|---|
| 周五前 | before | 2026-09-25 | — | day |
| 下周三前 | before | 2026-09-30 | — | day |
| 明晚8点 | at | 2026-09-24 | 20:00 | hour |
| 今晚 | at | 2026-09-23 | — | half_day |
| 后天下午3点半之前 | before | 2026-09-25 | 15:30 | minute |
| 3天内 | before | 2026-09-26 | — | day |
| 10月1号 | at | 2026-10-01 | — | day |
| 周三 | at | 2026-09-23 | — | day |
| 尽快 | at | None | — | unknown |

**验收标准**
- [ ] 上表全部通过，外加你自己补充的用例，总数 ≥ 40
- [ ] 跨月、跨年的用例（例如 anchor = 12 月 30 日时的“下周一”）
- [ ] 任何输入都不 panic（可以用一组随机字符串测试）

## 3. 证据校验 `verify`

完整签名和校验规则见 [PIPELINE.md](../PIPELINE.md) §6.2。核心：

```rust
pub fn verify(input: &VerifyInput, lookup: &dyn MessageLookup) -> VerifyReport;
pub fn find_quote(haystack: &str, quote: &str) -> Option<(usize, usize)>;
```

- `find_quote`：忽略空白（`char::is_whitespace`，包括全角空格 U+3000）后做**精确**子串匹配，返回原文中的**字符**下标区间 `[start, end)`。不做任何模糊匹配。
- 测试里自己写一个 `HashMap` 实现 `MessageLookup` trait 即可，不需要数据库。

**验收标准**
- [ ] 每种 `VerifyFailure` 至少一个测试
- [ ] `find_quote`：含空格、换行、全角空格、emoji、中英混排的用例；下标按字符而不是字节计算
- [ ] `TopicSummary` 部分证据通过 → `Unverified`；`Todo` 同样情况 → `Rejected`
- [ ] 带截止日期的结论，`deadline_raw` 不在证据消息中 → `Rejected`

## 参考资料

- QCE 仓库：github.com/shuakami/qq-chat-exporter（参考文件：`qq-chat-export-core/src/types.rs`、`qq-chat-export-server/src/parser/simple_parser.rs`）。**只看字段结构，不要复制代码**（QCE 是 GPL-3.0）。
- `chrono` 文档：`NaiveDate`、`Datelike::weekday()`、`Duration`。
- `blake3` crate：`blake3::hash(bytes).to_hex()`。

## 需要避免的坑

- 不要用昵称或群名片做身份。
- 不要把真实聊天记录放进测试或 fixtures，测试数据一律手写或合成。
- 字符串下标：Rust 的 `&s[a..b]` 按字节切片，切到中文字符中间会 panic。计算位置时用 `char_indices()`。
- 时区：`sent_at` 用配置的时区（默认 `+08:00`），不要用 UTC 日期来判断“今天”。
- 不要在 `temporal` 或 `verify` 里调用任何模型。
