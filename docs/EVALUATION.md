# 评估（EVALUATION）

> 用途：定义要对比的基线、指标、标注规范和实验流程，保证 3 天后能拿出可信、可复现的数字。
> 读者：同学 C（主要执行者）、@Develata（提供 `--strategy` 等开关）、写报告的所有人。

## 当前可运行范围

`chat-tldr-eval` 已实现 `check-stream`、`export-sheet`、`import-sheet`、`score` 和 `calibrate`：支持 Ours/B0 的抽取/Deadline/排序、最优一对一话题匹配、Exact thread F1、ARI、NMI、快照 rejected 比例及单次用量；校准单独生成 ECE、binary Brier 与 SVG。burst/边界、人工支持率、agreement/summarize 与其他基线未实现。Codex 负责工具，同学 C 负责人工标注和材料；测试分数不代表真实效果。命令见 [eval/README.md](../eval/README.md#已实现离线评分)。

主 CLI 的 `stats`、`decisions --run`、`jev-log --run` 提供实际记录。真实 200 条已完成 Ours/B0 云调用，见 ACCEPTANCE；质量评分和真实 Jev 校准曲线为**待标注**。禁止 LLM 生成的标注充当真实 gold。

## 1. 要回答的问题

1. 话题拆分比“不拆”或“固定切块”更好吗？Jev 归属判断比纯相似度聚类更好吗？
2. 结构化抽取（Todo / 截止日期 / 通知）的准确率如何？
3. 排序是否把用户真正需要处理的事放在前面？
4. 证据校验能把幻觉降到多少？
5. 增量处理的成本和延迟比每次重新总结低多少？
6. Jev 的概率是否校准，能否用作阈值依据（中文场景下尤其要验证）？

## 2. 系统与基线

**对照使用同一 LLM（DeepSeek `deepseek-flash`，关闭 thinking）、配置、temperature=0。** 输出共享 items/证据契约。当前 CLI 接受 `--strategy ours|b0`；B1/sim-* 待实现。每个系统必须单独导入同一输入到新 profile，不共享缓存或分析状态。

B0 一次处理整个待分析窗口，有界格式重试，不使用 Jev 分类、证据修复或话题合并。输入以渲染长度加每条开销估计控制在 96,000 字符，超限移除最早整条消息并记录数量，输出上限 16,384 token。此输入估计不是服务商 tokenizer 的精确上限。B0 流显式记录策略与模型顺序；score 保留其被拒条目。

| 名称 | CLI 参数 | 状态与说明 |
|---|---|---|
| **B0** 整段总结 | `--strategy b0` | **已实现**。全部待处理消息一次性交给 LLM，输出话题和 items。超过上下文时截断最早的消息，并记录截断比例 |
| **B1** 固定切块 | `--strategy b1` | **规划，当前 CLI 拒绝**。先按强时间间隔切开，再按固定 token 数（默认 3000）切块，逐块抽取，最后用一次 LLM 调用合并去重 |
| **Ours** 当前主线 | `--strategy ours --decider jev` | **已实现**。burst + 候选 + Jev 归属 + 受限 Jev 控制器 + 校验 + 排序；故障行为见下 |
| **Ours-LLM**（对照） | `--strategy ours --decider llm` | **可运行**。消息/话题判别使用 LlmDecider，控制器采用规则回退；不能描述为仅替换判别模型、其余完全相同的严格消融 |
| **Sim-TFIDF**（对照） | `--strategy sim-tfidf` | **规划，当前 CLI 拒绝**。第三层不用 Jev，改用 TF-IDF 相似度阈值（PIPELINE §3.5） |
| **Sim-Embed**（可选） | `--strategy sim-embed` | **规划，当前 CLI 拒绝**。同上，改用 embedding；只有配置了 embedding 服务才跑 |

Jev 不可用时，消息/话题判别可降级到 LlmDecider；控制器采用规则允许集合中的默认动作，不让 LLM 接管调度。报告必须区分这两种回退，按日志的实际 provider/model 与 `decision.method` 统计。

B0、B1 本身不做证据校验。评估时对它们的输出**事后**运行同一个 `verify`，只用于计算幻觉率，不过滤它们的结果（`score` 在计算抽取和排序指标时，对 B0/B1 保留 `rejected` 条目，对 Ours 系列则排除）。所有系统都用 `inbox --include-rejected` 导出结论。

## 3. 指标

### 3.1 话题拆分（disentanglement）

我们做的是交织话题拆分：一条消息可能接续几小时前的话题，线性“边界”在这里没有明确定义。因此以聚类指标为主：

| 指标 | 定义 | 地位 |
|---|---|---|
| **1-to-1 overlap** | 预测话题与标注话题做最优一对一匹配后，被匹配覆盖的消息比例（Elsner & Charniak 2008/2010） | 主指标 |
| **Exact-match thread F1** | 与标注话题**完全一致**的预测话题的 P/R/F1（Kummerfeld et al., ACL 2019） | 主指标 |
| **ARI / NMI** | 消息级聚类一致性 | 辅助 |
| **Burst purity** | 第一层 burst 中，全部消息属于同一个标注话题的比例 | 检验第一层的假设 |
| **Boundary P/R/F1** | 仅用于第一层：标注中“相邻两条消息属于不同话题”的位置作为真边界，与 burst 边界比较 | 辅助 |
| Pk / WindowDiff | 同上，线性分段指标 | 可选 |

> 参考：Elsner & Charniak, “You Talking to Me? A Corpus and Algorithm for Conversation Disentanglement”, ACL 2008；Kummerfeld et al., “A Large-Scale Corpus for Conversation Disentanglement”, ACL 2019。写报告前请核对原文中的指标定义（Q-EV-1）。

### 3.2 结构化抽取

分别对 Todo、Deadline、Announcement 计算 P/R/F1。

- **匹配规则**：预测 item 与标注 item 的 `kind` 相同，且预测的证据消息集合与标注的锚点消息集合有交集，即视为候选匹配；再按交集大小降序贪心做一对一匹配。相同交集按预测收件箱位置、gold item_id 决定先后；同消息的重复引用只计一次。抽取、截止日期和排序共用这次匹配。
- **Deadline 正确**：在 Todo 已匹配的前提下，`relation` 与 `bound_date` 都和标注一致；如果标注本身无法规范化，则比较 `raw` 是否一致。
- MentionMe 由规则产生，只报告正确率（预期接近 100%），用来检查实现有没有 bug。

### 3.3 排序

- 标注的重要性：P0 = 3、P1 = 2、P2 = 1、P3 = 0。
- 系统输出按收件箱顺序排列（先按层级，再按 `rank_score`）。B0/B1 要求模型按“对我的重要性”排序输出。
- 未匹配到标注的预测 item，相关度记为 0。
- 指标：**NDCG@5、NDCG@10**，增益 `2^relevance-1`、第 r 位折损 `log2(r+1)`，IDCG 由全部 gold 构造；**Recall@K**（K = 5、10）= 标注中的 P0 item 出现在前 K 位的比例。零分母或零 IDCG 记 undefined。

### 3.4 幻觉

- **Unsupported claim rate（自动）** = 证据校验失败的 item 数 / 输出的 item 总数。Ours 报告两个数：过滤前（LLM 原始输出）和过滤后（进入收件箱的部分，按设计应为 0）。
- **Unsupported claim rate（人工）**：每个系统随机抽 30 条，由人判断“证据是否真的支持这条结论”（引用存在但语义不支持，也算不支持）。

当前 `score` 只能计算保存的 inbox 快照中 `rejected` 的比例（过滤前/后），命名为 `snapshot_rejected_rate` / `snapshot_rejected_rate_after_filter`。更新/合并后的已存条目不能重建所有 LLM 原始提案，因此 `raw_unsupported_rate` 记不可用；不能把过滤后的零值当成语义无幻觉证明。人工指标仍需独立判断。

### 3.5 人工打分（1–5 分）

对每个系统生成的收件箱，两名评分者在**不知道是哪个系统**的情况下打分：

| 维度 | 5 分 | 1 分 |
|---|---|---|
| Faithfulness | 全部内容都能在原消息中找到依据 | 多处编造 |
| Coverage | 所有需要我处理或知道的事都在 | 漏掉关键事项 |
| Non-redundancy | 没有重复条目 | 同一件事出现多次 |
| Actionability | 看完就知道要做什么、什么时候做 | 看完仍需翻原聊天 |

### 3.6 成本与延迟

- 输入/输出 token、估算费用（来自 `stats` 事件）、端到端耗时。
- **增量实验**：把一段聊天分成前后两批（两次导出有 20% 重叠），先导入并分析第一批，再导入第二批。比较 Ours 的第二次 `analyze` 与 B0 对“全部未读”重新总结的成本和耗时。同时验证幂等：重叠部分的 `inserted` 应为 0。
- 缓存：报告冷启动（空缓存）数字；热缓存数字只作参考。
- `stats.usage.calls` 是逻辑模型调用数，包括失败调用，不展开内部 HTTP 重试；token/费用仅含服务已报告用量，缓存命中为零新增费用。费用属于本地估算，不能宣称与云账单完全相等。旧运行缺精确计数时的 `W_HISTORY_INCOMPLETE` 必须保留并排除不适用的比较。

### 3.7 Jev 校准

- 收集全部 Jev 回答（`chat-tldr jev-log --run <RUN_ID>` 输出 `jev_answer` 事件，见 CLI_PROTOCOL §3.12）并与标注对齐：
  - 话题归属 `choice`：正确答案是标注话题对应的候选；标注话题不在候选中时，正确答案为 `new_topic`；
  - `n{i}_todo`、`n{i}_announcement` 等 noul：与消息级标注对齐。
- 画**可靠性曲线**（10 个等宽区间，横轴预测概率，纵轴实际正确率），报告 **ECE** 和 **Brier score**。
- 同样的图也画 LlmDecider 的“口头概率”，作为对照。
- 按实际 `model` 区分 Jev 与降级 LLM 的回答；缓存命中也有本轮记录。`W_SUBJECT_UNAVAILABLE` 对应的 `kind=unknown,id=unavailable` 不能参与消息/话题对齐，须单独报告缺失数量，不能猜测 ID 或当成负例。
- 用于调 `tau_high` / `tau_low`：选择在验证集上使切分 F1 最大、且 LLM 复核比例 ≤ 20% 的组合。
- **风险**：Jev 官方文档说明中文精度低于英文（docs.typesafe.ai/models#language-support）。如果 ECE 明显偏高，报告里如实写明，并考虑：调整阈值、提高 LLM 复核比例。（提示词已统一用英文，见 Q-JEV-2。）

## 4. 数据

| 数据 | 来源 | 放在哪 | 用途 |
|---|---|---|---|
| **评估集** | 一个征得全体成员同意的真实测试群，约 200 条消息 | **只放本地** `eval/private/`（已加入 `.gitignore`） | 全部指标 |
| **合成集** | 手写或用 LLM 生成的群聊（含交织话题、@、回复、截止日期），附带标注 | `fixtures/`、`eval/synthetic/`（可提交） | 冒烟测试、演示、CI |
| 公开数据集 | VCSum（中文会议，含话题边界与分段摘要）、CSDS（中文客服对话摘要） | 不下载进仓库 | 仅作写报告时的参考与讨论，不作主评估 |

报告中必须写明：主要数字来自真实测试群；合成集的结果偏乐观，而且生成合成数据的 LLM 与被评估系统可能是同一个，存在循环偏差。

## 5. 标注规范（一页）

**单位**：一条消息（撤回消息不标注）。

**A. 话题（thread）**：给每条消息一个话题编号 `th1`、`th2`…。同一话题 = 围绕同一件事的讨论。“好的”“+1”“收到”归入它回应的话题。纯表情、刷屏归入它所在的话题；实在无法判断时单独编为 `th0`（杂项）。

**B. 消息级标签**：
- `todo`：这条消息要求某人（或全体）去做一件**具体的事**。“记得交报告” ✅；“报告好难” ❌；“谁能帮我带个饭” ✅（assignee = other/unknown）。
- `announcement`：面向群成员的通知或规则变化。“明天实验课改到 3 教” ✅；“我明天不去了” ❌。

**C. item（结论）**：同一件事在多条消息中出现时只标一个 item，列出全部锚点消息。
- `kind`：todo / announcement / decision。decision = 群里**已经达成**的决定（“那就定周六晚上”）；仍在讨论中的不算。
- `assignee`：me / all / other / unknown。@全体成员 → all。
- `deadline`：记录原文 `raw`，以及能确定时的 `relation`（before/at/after）和 `bound_date`（相对于消息发送时间计算）。“下周三前” → before + 那个周三。“尽快” → 只记 raw，不给日期。
- `importance`：
  - **P0**：我必须处理：@我且需要我行动、分配给我或全体的任务、任何带截止日期的事项；
  - **P1**：我应该知道：通知、已达成的决定、规则变化、@我但不需要行动；
  - **P2**：可能感兴趣的普通话题；
  - **P3**：闲聊。

**D. 例子**：

| 消息 | 标注 |
|---|---|
| `@全体成员 周五前把实验报告交到课代表那里` | todo ✅，announcement ✅；item：todo，assignee = all，deadline = {raw:"周五前", before, <周五>}，P0 |
| `周六晚上聚餐大家看看哪天有空` → … → `那就定周六晚上 7 点` | 前者不是 item；后者 item：decision，deadline = {raw:"周六晚上 7 点", at, <周六>}，P0（带截止日期） |
| `这题好难啊` / `+1` | 无 item；归入所在话题 |

**E. 一致性**：200 条消息中随机抽 50 条，由两人独立标注。报告：
- 消息级 `todo`、`announcement` 的 Cohen's κ；
- 两人话题划分之间的 1-to-1 overlap；
- 两人 item 之间的 F1（以一人为“标准答案”）。

**F. 标注工具（已实现）**：先保存一次主 CLI `messages` 命令的完整 UTF-8 JSONL，包括末尾 done 与实际退出码；再执行 `chat-tldr-eval export-sheet --messages <JSONL> --out <新CSV> [--exit-code N]`。它不读数据库、不调用模型，跳过撤回消息，不预填模型判断。人工填写后用 `chat-tldr-eval import-sheet <CSV> --out <新目录>` 输出 `messages.jsonl` 和 `items.jsonl`。

CSV 的 `thread/todo/announcement/items_json` 必须明确填写；无结论写 `[]`。原文列的 `text:` 防公式前缀应原样保留。两个命令都拒绝覆盖既有目标；表格字段、验证边界及 UTF-8 保存方法见 [eval/README.md](../eval/README.md)。省略 `--exit-code` 只能证明文件内部一致，不能证明真实子进程成功。

### 5.1 标注文件格式

`messages.jsonl`：
```json
{"message_id":"m_3f9a1c0b7d2e4a51","thread":"th3","todo":true,"announcement":true}
```
`items.jsonl`：
```json
{"item_id":"g12","kind":"todo","assignee":"all","anchors":["m_3f9a1c0b7d2e4a51"],
 "deadline":{"raw":"周五前","relation":"before","bound_date":"2026-09-25"},"importance":"P0"}
```

## 6. 实验流程

每个系统使用独立的数据目录，避免缓存和状态互相影响。

按 [FILE_LAYOUT.md](FILE_LAYOUT.md) 存放：`runs/<experiment-id>/<system>/profile/` 是 CLI 的 `--data-dir`，上一层保存供评分使用的 JSONL。下面用 `exp-01`、`course-demo` 作为示例实验/数据集 ID；先准备好对应的输出目录。

```bash
# 1. 导入（每个系统一份独立的数据目录）
chat-tldr --data-dir eval/private/runs/exp-01/ours/profile import eval/private/data/course-demo/group.json
# 2. 分析，保存 JSONL
chat-tldr --data-dir eval/private/runs/exp-01/ours/profile analyze --chat <CHAT_ID> --strategy ours --decider jev > eval/private/runs/exp-01/ours/analyze.jsonl
chat-tldr --data-dir eval/private/runs/exp-01/ours/profile inbox --chat <CHAT_ID> --all --include-resolved --include-rejected > eval/private/runs/exp-01/ours/inbox.jsonl
chat-tldr --data-dir eval/private/runs/exp-01/ours/profile messages --chat <CHAT_ID> > eval/private/runs/exp-01/ours/messages.jsonl
chat-tldr --data-dir eval/private/runs/exp-01/ours/profile jev-log --run <RUN_ID> > eval/private/runs/exp-01/ours/jev.jsonl
# 3. 已实现的 Ours 离线评分（其余指标显式不可用）
chat-tldr-eval score --gold eval/private/gold/course-demo --run eval/private/runs/exp-01/ours --out eval/private/results/exp-01/ours.csv
# 4. 未来校准入口；当前版本不支持
chat-tldr-eval calibrate --gold eval/private/gold/course-demo --jev eval/private/runs/exp-01/ours/jev.jsonl --out eval/private/results/exp-01/calibration_ours.png
```

- 未来其他系统替换 `--strategy` / `--decider` 和目录名；当前只可切换 ours 的 decider，不能据此声称已完成各基线。
- 规划中的汇总表命令（未实现）：`chat-tldr-eval summarize eval/private/results/exp-01/*.csv > eval/private/results/exp-01/summary.md`。
- 结果表中只放聚合数字，**不放消息原文**，这样汇总表可以放进报告。
- `score` 要求成功的完整流和与 gold 完全相等的未撤回消息 ID 集，拒绝 partial / dry-run；保存每条 CLI 的实际退出码，用 `check-stream --exit-code` 验证后再评分。评分目前只做流内退出码自检，且不能证明跨文件快照相同或调用者确实使用了完整视图参数；导出期间不要修改 profile。用量指标只描述 `analyze.jsonl` 里该次运行，不能当作历史累计成本。

## 7. 报告里的结果表模板

| 系统 | 1-1 | Thread F1 | Todo F1 | DDL F1 | Ann. F1 | NDCG@5 | Recall@5 | Unsupported（自动） | Tokens | 费用 | 耗时 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| B0 | | | | | | | | | | | |
| B1 | | | | | | | | | | | |
| Sim-TFIDF | | | | | | | | | | | |
| Ours-LLM | | | | | | | | | | | |
| **Ours** | | | | | | | | | | | |
