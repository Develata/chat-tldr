# 评估工具与数据

本目录同时容纳 `chat-tldr-eval` crate 和评估资料，以子目录隔开：

- `src/`、`tests/`：Rust 源码与测试。
- `synthetic/<dataset-id>/`：可提交的合成聊天、gold 和来源说明。
- `private/data/`、`private/gold/`、`private/consent/`：真实导出、真实标注与同意记录，全部忽略。
- `private/runs/<experiment-id>/<system>/profile/`：每个实验、每个系统独立的 CLI 数据目录。
- `private/runs/<experiment-id>/<system>/*.jsonl`：供评分工具读取的事件流。
- `private/results/<experiment-id>/`：指标和图表，审核后才把聚合材料复制到仓库 `reports/`。

`--data-dir` 指向 `profile/`；评分工具的 `--run` 指向它的上一层，即包含 `analyze.jsonl`、`inbox.jsonl`、`messages.jsonl` 和 `jev.jsonl` 的目录。

算法/标注规范见 [EVALUATION.md](../docs/EVALUATION.md)，完整目录树见 [FILE_LAYOUT.md](../docs/FILE_LAYOUT.md)。

## 已实现：事件流检查

```powershell
cargo run -p chat-tldr-eval -- check-stream fixtures/jsonl/version.jsonl
cargo run -p chat-tldr-eval -- check-stream fixtures/jsonl/analyze-partial.jsonl --exit-code 6
```

`check-stream <PATH> [--exit-code N]` 离线逐行读取 UTF-8 JSONL，复用 `core::EventStreamValidator` 检查协议版本、事件结构、连续序号、同一 run_id、唯一末尾 `done` 和退出码。未知事件类型按核心协议兼容规则接受；已知事件的错误结构、空行、截断或 `done` 之后的事件均失败。

不传 `--exit-code` 时，以文件里 `done.exit_code` 自检，只能证明文件内部一致，**不能证明真实子进程退出码**。传入该参数后才检查它与 `done.exit_code` 是否一致；调用方应保存实际退出码。合法的失败/部分完成运行也可以通过检查，因为这里验证协议完整性，不判断分析是否成功。

stdout 恰好输出一条 JSON 诊断摘要（`valid`、成功接受的 `events` 数量、`run_id`、`done_exit_code`、`process_exit_code`、`exit_code_source`、`error`），**这不是主 CLI 的 CliEvent 协议**。错误详情同时写 stderr。检查通过退出 0，文件/协议错误退出 1，命令用法错误退出 2；`--help` 和 `--version` 输出文本。

## 已实现：标注表格往返

```powershell
# messages.jsonl 必须保存一次 messages 调用的完整 UTF-8 stdout，含末尾 done。
chat-tldr --data-dir eval/private/runs/exp-01/ours/profile messages --chat <CHAT_ID> > eval/private/runs/exp-01/ours/messages.jsonl
$messagesExit = $LASTEXITCODE
chat-tldr-eval export-sheet --messages eval/private/runs/exp-01/ours/messages.jsonl --out eval/private/sheets/course-demo.csv --exit-code $messagesExit

# 在表格软件里填写标注列，保存为 UTF-8、逗号分隔 CSV，再导入到一个尚不存在的目录。
chat-tldr-eval import-sheet eval/private/sheets/course-demo.csv --out eval/private/gold/course-demo
```

`export-sheet --messages <PATH> --out <CSV> [--exit-code N]` 只读完整的 `message` / `done` 事件流，不打开数据库，也不调用模型。必须成功完成（`done.status=complete`、`exit_code=0`）；截断、序号错误、混入其他命令的已知事件或重复消息 ID 均失败。兼容的 MINOR 版本未知事件忽略，但仍校验其序号、run_id 和协议版本。`--exit-code` 的来源与 `check-stream` 相同：省略时只检查文件内部一致性，不能证明真实进程成功。文件必须是 UTF-8；旧版 PowerShell 的默认重定向可能写成 UTF-16，应显式选择 UTF-8 保存。

每条未撤回消息占一个 CSV 记录；撤回消息跳过，成功摘要报告 `recalled_skipped`。不预填模型话题、待办或重要性，避免影响独立人工标注。

| 列 | 填写方式 |
|---|---|
| `sheet_version` | 固定 `1`，保留不改 |
| `message_id` | 保留原 ID，格式为 `m_` 加 16 位十六进制字符 |
| `sent_at`、`sender`、`sender_display`、`display_text` | 原始时间、发送者 ID、展示名和消息原文；保留不改 |
| `thread` | 必填，任意非空稳定话题标签；例如 `th1`、`课程报告`，同话题使用相同值 |
| `todo`、`announcement` | 必须明确填 `true` 或 `false`；空白不等于 `false` |
| `items_json` | 必须填 JSON 数组；无结论明确写 `[]`，有结论按下面示例填写 |

CSV 带 UTF-8 BOM，中文、逗号、双引号以及单元格内换行由 CSV 编解码保留。四个原始文本列统一添加一层 **`text:`** 前缀，防止表格软件执行公式、改写日期或数值；导入时只移除这一层。即使原内容已经以 `text:` 开头，也会导出成 `text:text:...`，不会丢失原前缀。原文中的 `=...`、`+...`、`-...`、`@...`、前导制表符等始终位于这个文字前缀之后。不要手工删除前缀；不要让表格软件转换这些列的类型。可重排列，但不能重命名、删除或重复列。

下面的内容可直接粘入一格 `items_json`；将锚点 ID 换成表中实际消息 ID。**每个 item 只在其中一个锚点所在行定义一次**，其他行填 `[]`。同一格可放多个对象，同一消息也可成为多个 item 的锚点：

```json
[{"item_id":"g12","kind":"todo","assignee":"other","anchors":["m_3f9a1c0b7d2e4a51"],"deadline":{"raw":"周五前","relation":"before","bound_date":"2026-09-25"},"importance":"P0"}]
```

无截止信息时明确写 `"deadline":null`。无法确定日期时只写原文，例如 `"deadline":{"raw":"尽快"}`；可确定时 `relation` 与 `bound_date` 必须同时填写，分别使用 `before` / `at` / `after` 与真实存在的 `YYYY-MM-DD` 日期。工具不猜日期、不推导真假标注、不自动修改重要性。`kind` 仅为 `todo` / `announcement` / `decision`，`assignee` 仅为 `me` / `all` / `other` / `unknown`，`importance` 仅为 `P0` 至 `P3`。

`import-sheet <CSV> --out <DIR>` 在全部记录验证通过后，输出 EVALUATION §5.1 约定的 `messages.jsonl` 和 `items.jsonl`。它拒绝空标注、未知字段/枚举、非法日期、重复消息 ID、重复 item ID（即使内容一致）、重复锚点和表中不存在的锚点。`item_id` 是非空稳定字符串，不允许首尾空白或控制字符；没有规定必须以 `g` 开头。标注时不要删除消息行；导入器只掌握这张表，不能与未提供的原始消息流核对被删除的行或被改写的原文。

导出和导入都**不覆盖**已有目标，也没有强制覆盖参数。先完整验证输入，再在同一父目录暂存、刷新并发布；CSV 使用不覆盖的文件发布，gold 两个文件使用不覆盖的原子目录重命名一起发布，发布前临时出现的空目录也会导致失败。gold 发布支持 Windows、Linux 和 macOS，其他平台返回明确错误。目标旁的 `.chat-tldr-eval.lock` 防止本工具并发写入同一目标；正常结束自动移除。若进程被强制终止，确认没有写入者后再人工处理残留锁与临时目录。输出父目录可以自动创建。

两条命令与 `check-stream` 一样，stdout 只输出一条工具 JSON 摘要（不是主 CLI `CliEvent`）；失败同时写 stderr，退出 1。未填完的表不会产生半套 gold。合法空消息流可以导出只有表头的 CSV，并导入为两个空 JSONL 文件。

## 已实现：离线评分

```powershell
chat-tldr-eval score --gold eval/private/gold/course-demo --run eval/private/runs/exp-01/ours --out eval/private/results/exp-01/ours.csv
```

输入为 gold 目录的 `messages.jsonl` / `items.jsonl`，以及 run 目录的 `messages.jsonl` / `inbox.jsonl` / `analyze.jsonl`。gold 可由 `import-sheet` 生成；运行目录按 [EVALUATION](../docs/EVALUATION.md#6-实验流程) 导出。`inbox` 必须使用 `--all --include-resolved --include-rejected`，保存完整快照。消息范围应覆盖整个独立实验 profile，不能只导出一页或某个子时间窗。

目前只评分 **Ours（含 LLM decider）**：排除 rejected 后，在其余条目的原始收件箱顺序上计算。其他基线未实现；文件格式不记录 strategy，工具无法自动识别把 B0/B1 输出误放到该目录的情况，不能据此做基线比较。工具不读 SQLite、不联网、不调用模型。

输入检查包括：三份主 CLI 事件流成功且完整、连续 seq/同一 run_id/唯一 done、gold 完整且无重复、所有未撤回消息 ID 与 gold **集合完全相等**、证据引用范围、inbox 顺序与计数、分析统计的 run/chat 身份。未来 MINOR 未知事件可以忽略，但仍验证信封。partial、截断、dry-run、计数与输出不符的快照不能评分；缺少运行统计或出现 `W_HISTORY_INCOMPLETE` 时成本指标记不可用。

事件流不记录 `--all` / `--include-resolved`，因此计数一致**不能证明所有事项已导出**；也无法证明三份文件来自同一数据库快照、真实进程退出码或其语义正确性。调用方须遵循上述导出参数、保存实际退出码并单独 `check-stream --exit-code`，实验期间不要修改该 profile。工具检查引用 ID 的覆盖，不重新执行 engine 的逐字证据验证；合法 Unverified 话题摘要可以包含部分失效证据。

CSV 每行一个聚合指标，固定列为 `score_version,system,metric,value,numerator,denominator,status`，当前版本 `1`、system 为 `ours`。不包含正文、证据原文、账号、消息/结论 ID 或输入路径。输出采用与 CSV 标注相同的不覆盖发布，现有文件原样保留。stdout 恰好一条工具 JSON 摘要，退出 0 表示完成评分；输入/输出错误退出 1，用法错误退出 2。**完成评分不表示效果达到验收门槛**。

| 指标 | 当前计算规则 |
|---|---|
| `todo/announcement/decision_{precision,recall,f1}` | 同 kind、证据 ID 集与 gold 锚点有交集；按交集大小降序贪心一对一。交集相同时按预测在收件箱的位置、gold item_id 排序；重复引用同消息只计一次 |
| `deadline_{precision,recall,f1}` | 仅 Todo；已匹配 Todo 的 relation + 日期一致才算 TP，gold 无规范化日期时逐字比 raw。所有带 deadline 的预测/标注 Todo 分别进入 P/R 分母 |
| `ndcg_at_5/10` | P0/P1/P2/P3 对应 3/2/1/0，增益 `2^relevance-1`，第 r 位折损 `log2(r+1)`；IDCG 包括所有 gold。未匹配、MentionMe、TopicSummary 相关度为 0，仍占排序位置 |
| `p0_recall_at_5/10` | 前 K 位匹配到的 gold P0 数 / 所有 gold P0 数 |
| `snapshot_rejected_rate`、`snapshot_rejected_rate_after_filter` | 当前保存的 inbox 条目中 rejected 的比例，分别在过滤前/后计算；**不是所有 LLM 原始提案的幻觉率，也不是人工语义支持率** |
| `run_input_tokens/output_tokens/calls/cache_hits/elapsed_ms/estimated_cost_usd` | `analyze.jsonl` 中单次运行的已报告统计；不推断整个增量历史总和，费用不等于账单 |

零分母用空 `value` + `status=undefined`，包括无 gold 相关条目的 NDCG。F1 直接计算 `2TP/(预测数+标注数)`，两边都为空时同样 undefined。缺失运行统计标为 `unavailable_run_stats`；无法由现存快照重建的原始提案比例标为 `unavailable_raw_proposals`。话题匹配/ARI/NMI/burst/边界、MentionMe 正确率、人工支持率和校准指标目前列为 `not_implemented`，不填 0 或伪造分数。

匹配通过 kind + 锚点倒排索引生成候选，再排序贪心；不为互无交集的条目逐对扫描。NDCG 的理想排序只需四档直方图。没有进行真实性能基准，不宣称提速比例。

`agreement`、`calibrate`、`summarize`、其余评估指标和实验自动执行尚未实现，未知子命令会报错。测试中的合成预期分数只验证算法，不能写成真实群聊或模型效果。
