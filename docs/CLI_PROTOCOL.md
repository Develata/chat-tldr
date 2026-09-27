# CLI 与 JSONL 协议（CLI_PROTOCOL）

> 用途：定义 `chat-tldr` 命令行的命令、参数、stdout 上的 JSON Lines 事件、错误码、退出码和版本兼容规则。
> 读者：@Develata（实现 CLI）、同学 B（GUI 解析输出）、同学 C（eval 调用 CLI）。信封类型的定义见 [DATA_MODEL.md](DATA_MODEL.md) §4。

> 状态：已采纳的 [CLI v1 评审稿](CLI_V1_REVIEW.md) 及 [ADR-0010](decisions/0010-semantic-relations.md) 已落地。策略为 `ours/b0`，Decider 为 `jev/llm`；新增只读 `relations`。实际能力以 `version.capabilities` 为准。

## 1. 总规则

1. **stdout 只输出机器协议**：每行一个 CliEvent JSON，UTF-8，`\n` 结尾，不输出空行，不做美化缩进。
2. **stderr 只输出给人看的日志**（`tracing`，级别由 `-v` / `RUST_LOG` 控制）。两者绝不混用。GUI 可以把 stderr 显示在“日志”面板里，但不能解析它。
3. stdout 正常可写时，每次调用恰好以一个 `done` 事件结束，且它是最后一行。强杀、崩溃、管道断开不保证能收尾。`done.exit_code` 必须与进程退出码一致；GUI 收到 `done` 后仍检查进程是否正常结束。
4. `seq` 从 0 开始连续递增。`run_id` 在一次调用内不变。
5. 例外：带 `--html <FILE>` 时，HTML 写入文件，stdout 仍照常输出 JSONL。
6. `--help` / `-h`、`--version` 使用标准帮助文本；GUI 握手使用 `version` 子命令。参数错误仍输出 JSONL `error` + `done`，提示放 stderr。每条 JSONL 及时 flush，不出现交互式输入提示。
7. 未知 event 可以忽略并记录，但仍占用一个序号；已知 event 解析失败、序号断裂、重复 `done` 都是协议错误。共享类型与 `EventStreamValidator` 位于 `crates/core`，GUI 和 eval 共用。

## 2. 命令

二进制名：`chat-tldr`（`apps/cli`）。

### 2.1 全局参数

仓库与运行文件的存放位置、路径解析和文件归属见 [FILE_LAYOUT.md](FILE_LAYOUT.md)。

| 参数 | 默认值 | 说明 |
|---|---|---|
| `--data-dir <DIR>` | 系统数据目录下的 `chat-tldr/`（Windows：`%APPDATA%\chat-tldr`） | 数据库 `chat-tldr.db` 与配置 `config.toml` 所在目录 |
| `--config <FILE>` | `<data-dir>/config.toml` | 配置文件，见 §7 |
| `-v` / `-vv` | | stderr 日志级别 |

全局参数允许放在子命令前后。显式参数优先于显式配置文件、数据目录中的默认配置、内置默认值。显式 `--config` 指定的文件缺失或无效时返回 `E_CONFIG`；默认配置不存在不影响离线命令。`version` 和帮助不读取配置、不初始化数据库；空数据目录的 `chats` 返回空列表，也不创建数据库。查询不调用模型、不要求模型 key。聊天时间默认 `+08:00`，与交付截止所在的 America/Santiago 时区分开配置。

### 2.2 命令一览

| 命令 | 作用 | 主要输出事件 |
|---|---|---|
| `version` | 输出版本与协议版本（GUI 启动时的握手） | `ack`, `done` |
| `config init [--out FILE]` | 创建示例配置，不覆盖现有文件 | `ack`, `done` |
| `config show` | 读取生效的模型配置与密钥来源/就绪状态；不输出密钥 | `ack`, `done` |
| `config set <llm\|jev> [选项]` | 校验并原子保存单个 provider，可通过 stdin 或隐藏输入保存密钥 | `ack`, `done` |
| `doctor` | 离线检查配置、环境变量存在性、路径、数据库版本和能力就绪状态 | `ack`, `done`；失败附 `error` |
| `import <PATH>...` | 整批原子导入一个或多个 QCE JSON 文件，幂等 | `progress`, `warning`, `ack`, `stats`(scope=import), `done` |
| `chats` | 列出已导入的会话及三个游标、未读数 | `chat`*, `done` |
| `analyze --chat <ID>` | 运行智能体控制器，分析待处理消息 | `progress`, `decision`, `topic`, `insight`, `stats`(scope=run), `done` |
| `inbox --chat <ID>` | 查询收件箱（供 GUI 展示） | `inbox`, `topic`*, `insight`*, `done` |
| `overview --chat <ID> [--since TIME] [--until TIME] [--html FILE]` | 只读分析总览：热门、优先、相关、截止、未读、资料 | `ack`（头部及逐行数据）, `done` |
| `relations --chat <ID> [--since TIME] [--until TIME]` | 更正/取消/冲突关系、问题与完整/部分回答；只读 | `ack`（头部及逐行数据）, `done` |
| `messages --chat <ID> [--since] [--until]` | 按时间顺序列出消息及其话题归属（供评估和 GUI 浏览原文） | `message`*, `done` |
| `feedback <INSIGHT_ID> --useful \| --not-important` | 设置当前有效反馈，不重复计票 | `ack`, `done` |
| `resolve <INSIGHT_ID> --done \| --dismiss \| --reopen` | 修改结论的 `lifecycle` | `ack`, `done` |
| `mark-read --chat <ID> --up-to <CURSOR>` | 推进 `last_reviewed` | `ack`, `done` |
| `decisions --run <RUN_ID>` | 重放某次运行的决策日志 | `ack`, `decision`*, `done` |
| `jev-log --run <RUN_ID>` | 导出某次运行的全部 Jev 回答（供校准评估） | `ack`, `jev_answer`*, `done` |
| `stats [--chat <ID> \| --run <RUN_ID>]` | 运行统计 / 全局统计；两个筛选参数互斥 | `stats`, `done` |

任意命令均可能在终止前输出结构化 `error`，不受上表列出的正常输出限制。重放命令的首个 `ack.target` 是查询的历史 RunId；随后事件信封仍使用本次调用的 RunId。

### 2.3 各命令参数

**`config init [--out FILE]` 与 `doctor`**
- 配置默认写入 `<data-dir>/config.toml`；init 不覆盖现有目标，示例没有密钥。config set 写入全局 `--config` 指向的文件（否则用默认配置），允许创建尚不存在的文件。
- `doctor` 不联网、不创建数据库，不打印密钥值；分别报告 `read`、`import`、`analyze` 的就绪情况。密钥按本地 `api_key` 优先、没有时才读取 `api_key_env` 的顺序解析；无效的已保存密钥不会静默回退。缺少必需的 LLM key 返回 `E_CONFIG` / 退出 4；模型 key 就绪不证明云服务连通性或模型效果。
- `doctor` 的 `ack.detail.paths` 包含解析后的 `data_dir`、`config_file`、`database`、`outputs_dir`、`qce_exports_dir`，GUI 复用这些路径。

**`config show` 与 `config set <llm|jev>`**
- show/set 的成功 `ack.detail` 是 core `settings::SettingsSnapshot`：`version=1`、`config_file`、`revision`、`llm`、`jev`。provider 包含地址、模型、格式、超时、温度、JSON 输出、额外请求参数、费用估算和 `credential={source:config|environment,available,problem}`；没有密钥字段。
- set 接受 `--base-url`、`--model`、`--api-format openai|anthropic`（仅 LLM；Jev 固定 SystemOne）、`--api-key-env`、`--timeout-secs`、`--temperature`、`--json-mode true|false`、`--extra-body <JSON对象>`、两项 `--price-*-per-mtok`。省略字段保持原值。
- 密钥入口四选一：`--key-prompt` 在终端隐藏输入；`--key-stdin` 从 stdin 读取原始密钥（去掉末尾换行）；`--key-from-env ENV_NAME` 把指定环境变量迁入 config；`--clear-key` 删除本地密钥并恢复环境变量查找。不提供在 argv 上填写密钥的参数。
- GUI 使用 `config set <provider> --request-stdin`，stdin 为至多 64 KiB 的 `ProviderUpdate` JSON；不能同时传普通更新参数。字段与上述配置项一致，另有可选 `key`、`clear_key`、`expected_revision`。`key` 留空/不提供由 GUI 解释为保留；显式空字符串会拒绝。未知字段、无效值或过期 revision 返回 `E_CONFIG`，不回显输入。
- set 使用独立文件锁和同目录临时文件原子替换，保留无关配置与注释；失败保留原配置，不修改数据库。保存的 API key 按课程阶段决定明文写入本地 config（[ADR-0011](decisions/0011-provider-settings.md)）。更改保存密钥对应的地址/格式时必须重新填写 key 或显式 clear，避免误用旧凭据；未显式传 extra_body 时清空上一个服务的扩展参数，`{}` 可覆盖默认 DeepSeek 参数。
- GUI 只在版本能力包含 `config show`/`config set` 时启用面板，并在完整 ack/done 和成功退出后显示保存成功；设置草稿和 JSON 输入不写入 `gui-state.json`。

**`import <PATH>... [--self-uid UID] [--self-uin UIN]`**
- 接受 QCE 单文件 JSON（顶层含 `metadata`、`chatInfo`、`statistics`、`messages`）。
- QCE 的 chunked-JSONL 导出（`manifest.json` + `chunks/*.jsonl`）：MVP 不支持。已按上游确切 manifest 结构识别为 `E_INPUT_UNSUPPORTED`；结构不完整的 JSON 或原始 JSONL 仍可返回 `E_INPUT_PARSE`，均退出 3 且不导入。识别不依赖文件名，字段依据见 [QCE_DOCKER_EXPORT](QCE_DOCKER_EXPORT.md)。
- 同一文件导入多次、两个导出有重叠：结果不变（`inserted=0` 或只插入新消息）。
- 多文件导入采用一个事务；任何文件解析或身份校验失败，整个批次回滚。成功提交后才输出 `ack`，其中 `detail.chat_ids` 是去重后按字符串排序的会话 ID；`changed` 只反映业务数据变化，不计新增运行日志。
- `--self-uid` / `--self-uin` 仅属于 import。缺少显式值时保留已存会话身份，新会话才回退到文件元数据；显式值与已存身份冲突时返回 `E_CONFIG`，不悄悄切换身份。没有自身身份时个人 @ 无法识别并告警，但 `@全体成员` 仍成立。
- 输入文件必须已经写完并关闭。CLI 只读，不因导入成功而移动或删除原始导出；QCE 管理组件向调用方交付完成后的本地文件路径。

**`analyze --chat <ID>`**

| 参数 | 默认值 | 说明 |
|---|---|---|
| `--since <RFC3339>` / `--until <RFC3339>` | 无 | 指定时间范围；不指定时处理全部待切分消息和脏话题（PIPELINE §1.1） |
| `--decider <jev\|llm>` | `jev` | Jev 不可用（未配置 key 或连续失败）时自动降级为 `llm` 并发 `warning` |
| `--strategy <ours\|b0>` | `ours` | B0 需独立新导入的数据目录；B1/sim-* 未实现。见 EVALUATION §2；GUI 不使用 |
| `--max-steps <N>` | 配置值（默认 64） | 控制器最大步数 |
| `--budget-usd <X>` | 配置值（默认 0.50） | 本次运行费用上限 |
| `--html <FILE>` | 无 | 运行结束后把收件箱渲染成 HTML（演示备用） |
| `--dry-run` | 关闭 | 只读计划：不联网、不写业务数据或缓存、不推进游标；输出 `ack.detail.plan` |

时间参数使用带偏移的 RFC 3339，统一表示 `[since, until)`。命令启动时固定本次有资格处理的消息；范围外消息可以作上下文但不标为已分析，运行期间新导入的消息留待下次。没有工作时正常结束且不调用模型。`--max-steps` 为正整数，预算为有限正数；在请求前按输入估算、输出上限和配置单价预留预算，限制的是本地费用估算。`--dry-run` 与 `--html` 互斥，不能把计划视为完成分析。检查点提交后才输出相应结论事件；GUI 在结束后刷新 inbox，不依赖拼接过程事件构建最终视图。

`ack.detail.plan.merge_candidates` 为当前已完成话题的可处理合并候选数；不预测待分析消息之后产生的候选。`messages=0` 但候选非零时仍有分析工作，按同样的密钥、步数和费用规则执行。候选至少有一条范围内的回复源；经确认合并的是整对话题（可包含窗口外已完成成员），不推进这些成员的消息状态或游标。当前先完成抽取/校验，再合并全部成员均已完成的话题，细节见 [ANALYSIS_EXECUTION](ANALYSIS_EXECUTION.md)。

分析过程中按本次成功处理的消息时间自动关闭过期话题，成功提交后输出 `topic(state=closed)` 并计入 `topics_updated`；关闭不改变消息状态或游标。历史回填可补入首次关闭时间之前相关的消息，保持 Closed。首次边界持久保存，之后的新消息不能因回填而继续进入 Closed；dry-run 和无工作调用不执行关闭维护。

消息归属成功提交时也会输出 `topic`，以区分“尚无持久结果”和“已分配但尚未 Verify”。该事件本身不表示消息已分析；以消息状态及 `stats.messages_analyzed` 为准。后续失败会保留归属和恢复所需草稿，并报告 partial；GUI 仍在结束后刷新 inbox。

**`inbox --chat <ID>`**
- `--include-resolved`：同时返回 `done` / `dismissed` 的结论。
- `--include-rejected`：同时返回 `rejected` 的结论（仅供评估计算幻觉率；GUI 不使用）。
- `--all`：关闭“自上次查看以来”的时间窗口，生命周期与校验过滤仍由前两项单独控制。
- `--html <FILE>`：同上。
- 收件箱的内容规则见 [PIPELINE.md](PIPELINE.md) §7.4。
- 元数据、counts、topic、insight 和 HTML 来自同一读快照；insight 按 P0–P3、层内 rank_score 降序、InsightId 升序排列。
- HTML 先写临时文件再发布。已有普通报告允许更新；拒绝数据库及其辅助文件、配置、GUI 偏好、sources/backups 目录内文件、符号链接以及可识别的数据文件。analyze 在调用模型前预检目标；首次不存在的目标在本次运行中只允许不覆盖发布，期间新出现的文件会保留。发布前重新检查目标，路径无扩展名不影响新报告导出。失败时输出 `E_OUTPUT_WRITE`；预检或单独查询退出 8，analyze 已提交业务结果时退出 6 / partial，并说明分析数据保留。

**`mark-read --chat <ID> --up-to <CURSOR>`**
- `<CURSOR>` **必须**是 GUI 最近一次完整显示的 `inbox` 事件里的非空 `view_cursor`，原样回传；空值时禁用标为已读。
- CLI 校验游标格式、会话归属和连续 `done/skipped` 前缀形成的安全上界，不能跨越 `failed/pending`。CLI 不能仅凭 Cursor 证明用户看过界面，“只回传已展示位置”由 GUI 契约保证。
- 如果 `<CURSOR>` 早于当前 `last_reviewed`，则不做任何改动，返回 `ack`（`changed=false`）。

**`overview --chat <ID>`**
- 用户于 2026-09-26 要求推进分析视图后新增的兼容命令。`--until` 为带偏移的 RFC 3339 排他终点，默认当前时间；`--since` 为包含下界，默认 until 前 24 小时，必须早于 until。GUI 提供 6/24/168 小时窗口。`--html` 使用与 inbox 相同的路径保护和原子发布。
- 不读取密钥、不调用模型、不创建数据库、不保存运行或推进任何游标。单个只读事务中完成查询和证据重验。热榜只统计 done、非系统/撤回、已归属的窗口消息；原始提及及资料也可显示 pending，必须标识。优先/相关/截止保留窗口前 open 事项；未读回顾按 last_reviewed。until 同时用于截止状态，未归一或仅模型猜测的时间列为待确认。
- 首个 `ack.command=overview`，`changed=false`，`target=ChatId`，`detail={overview:<头部>,counts:<各节行数>}`。头部 `version=1`，包括 chat_id、since/until、generated_at、data_start/end、window_messages、pending_messages（全群）、window_pending、last_reviewed，以及空的九节数组。
- 后续每个 `ack.command=overview.rows` 的 target 仍为同一 ChatId、changed=false；`detail={section:<节名>,row:<对应类型>}`。节名为 hot_topics、priority_topics、related、mentions、deadlines、unread_topics、resources、topics、insights；类型见 `core/overview.rs`。归属引用可先于被引用的行，消费者在结束时检查计数和引用完整性。
- v1 信封、既有事件、DB 版本均不变。GUI 通过 capabilities 判断入口，仅在 done/真实进程退出均成功、每节计数匹配且引用有效后发布，不把部分流或重复头部当成完整结果。单行数据上限 900,000 字节，超限返回 E_OUTPUT_WRITE/8；GUI 每节最多 50,000 行并明确拒绝超限，不能截断后宣称完整。
- 总览不返回可用于标已读的 view_cursor。读取和展示它不代表完整阅读收件箱；仍需按既有 inbox 契约标读。报告是当前状态在消息窗口上的视图，不是历史版本快照；完整统计口径见 [ANALYSIS_VIEWS](ANALYSIS_VIEWS.md)。

**`feedback` 与 `resolve`**
- 相同评价重复提交时 `changed=false`；切换评价替换当前有效值，不能累计成多次有效投票。审计记录可另存。
- resolve 设置生命周期，重复设定同一状态无变化成功；reopen 不改变证据校验状态。feedback 与 resolve 互不改变对方状态，也不推进已读游标。

**历史查询**

- `decisions --run <RUN_ID>` 按 step 重放；`jev-log --run <RUN_ID>` 按记录顺序导出所有答案，包含缓存命中和 LLM 降级后的答案。首个 `ack.target` 为历史 ID，`ack.detail.run_status` 为已保存状态；信封 run_id 始终是这次查询的 ID。
- `stats` 无筛选返回全局表计数与累计用量；空数据库路径返回零统计，不建库。`stats --chat <ID>` 返回 `scope=global` 的会话聚合，首个 ack 的 target 与 `detail.chat_id` 标明筛选；无法归属到单个 chat 的 model_cache/preference_weights 不列入该计数。
- `stats --run <RUN_ID>` 返回已保存分析运行的 `scope=run`。查询未知 chat / run 分别返回 `E_CHAT_NOT_FOUND` / `E_RUN_NOT_FOUND`，退出 3。dry-run 和无工作 analyze 不保存运行记录。
- `--chat` 与 `--run` 互斥。以上命令均不要求模型 key、不调用模型、不创建数据目录；旧记录的信息不足通过 warning 标明，不写回修补数据库。

## 3. 事件与 payload

`relations` 首个 ack 的 `command=relations`，detail 含窗口、未撤回消息数 `messages`、已作关系抽取数 `analyzed_messages`、缺口 `uncovered_messages`、关系/问题行数。默认查看全部已导入时间；边界为 `[since,until)`。后续 `command=relations.row` 的 detail 为 `{relation: SemanticRelation}`，`relations.question` 为 `{question: QuestionState}`，target 始终为 chat ID，changed=false，最后 done。

关系种类为 question/answers/replaces/cancels/conflicts；source 是原问题/安排，target 是后续回答/修改/取消/冲突证据。answers 的 answer_completeness 为 full/partial。问题状态 pending/partially_answered/answered，只由当前有效引用且早于 until 的完整回答关闭；普通回复不自动算回答。since 过滤问题源时间及变更目标时间，目标之前的原安排仍随关系保留证据。

`verification_status=verified` 只表示同群、消息顺序、未撤回和逐字引用通过，不等于人工语义确认。无效关系仍输出 rejected，但不参与问题状态。用户 done/dismissed 不受影响。v1 数据库及旧模型未返回 relations 的记录计为未覆盖；0 个待回应不能证明未分析消息没有问题。新关系引用也受未脱敏数据边界约束。

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

合并事务成功后输出目标、源各一个 `topic`。源状态为 `merged`、`message_count=0`、`merged_into` 指向目标，时间字段保留历史范围；目标计数包含迁移后的全部成员。结论 ID 不变，结束后查询 inbox 获取最终 topic_id。

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
`view_cursor` 的 Rust 类型是 `Option<Cursor>`。它来自本次快照的连续 `done/skipped` 前缀，不直接复制包含 failed 的 `last_analyzed`；无安全位置时为 `null`。
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

历史 `usage` 按 stage/provider/model 聚合，`calls` 为逻辑 provider 调用数（包括失败），不展开内部 HTTP 重试或 JSON 修正。token/费用仅含响应已报告用量；缓存命中只增加 `cache_hits`，本轮 token/费用记 0。因此费用是本地估算，不能用于服务端账单对账。

新分析运行保存精确计数与耗时。旧运行或未完成持久化的记录可能没有计数快照，此时发送 `W_HISTORY_INCOMPLETE`：message/created 计数来自仍存在的数据库行，无法恢复的 insight 更新/验证计数为 0，不能视为历史事实上的零；elapsed_ms 截止到保存的结束/heartbeat 时间。

### 3.8 `ack`
```json
{"command":"resolve","target":"i_7c2d9e01ab34","changed":true,"detail":{"lifecycle":"done"}}
```
`version` 命令的 `ack`：`{"command":"version","target":null,"changed":false,"detail":{"cli_version":"0.2.0","schema_version":"1.0","db_version":2,"capabilities":{"commands":["version","config init","config show","config set","doctor","import","chats","messages","analyze","inbox","overview","relations","feedback","resolve","mark-read","stats","decisions","jev-log"],"strategies":["ours","b0"],"deciders":["jev","llm"]}}}`。capabilities 只列实际已实现的能力，不能广告空桩。

### 3.9 `warning`
```json
{"stage":"decide","code":"W_DECIDER_FALLBACK","message":"Jev 连续 3 次失败，本次运行改用 LLM decider"}
```

| code | 含义 |
|---|---|
| `W_SELF_ID_MISSING` | 显式参数、已存会话身份与文件元数据均不能提供自身身份；个人 @ 不可识别，@全体成员仍成立 |
| `W_DECIDER_FALLBACK` | Jev 不可用，已降级 |
| `W_BACKFILL` | 导入的消息早于 `last_analyzed`，下次 analyze 会补分析 |
| `W_UNKNOWN_ELEMENT` | 遇到未识别的 QCE 元素类型，已用占位符 |
| `W_INPUT_NORMALIZED` | 输入按兼容规则规范化，如缺 ID 使用稳定哈希、缺元素回退 content.text；详见 message |
| `W_INSIGHT_REJECTED` | 有结论未通过证据校验（附数量） |
| `W_DEADLINE_UNNORMALIZED` | 截止日期无法规范化，只保留原文 |
| `W_HISTORY_INCOMPLETE` | 旧记录缺少精确运行计数、decision confidence 或模型名；message 明确说明可用范围，不能把缺失当作已知的零 |
| `W_SUBJECT_UNAVAILABLE` | 旧答案不能可靠对齐 subject；返回 unknown/unavailable，不猜测消息 ID |

### 3.10 `error`
```json
{"stage":"extract","code":"E_PROVIDER_RATE_LIMIT","retryable":true,"message":"LLM 返回 429，重试 3 次后放弃","topic_id":"t_a19c3b0d77e2"}
```
`topic_id` 可选。一个 `error` 不一定终止运行：单个话题失败时运行继续，最终 `done.status = partial`。

### 3.11 `done`
```json
{"status":"complete","exit_code":0,"finish_reason":"done","elapsed_ms":48210}
```
`status` ∈ `complete | partial | failed | cancelled`，与 `runs.status` 一致。已知值必须与退出码匹配：complete 为 0、partial 为 6、cancelled 为 130，failed 不得为 0/6/130。新版本未知状态可保留为 Unknown，但不能显示为完整成功。`finish_reason` 只在 `analyze` 时出现，取值同 `FinishReason`。

### 3.12 `jev_answer`
```json
{"model":"jev-1.13.0","request_key":"b3:5d1e…","question_id":"n1_todo","qtype":"noul",
 "subject":{"kind":"message","id":"m_3f9a1c0b7d2e4a51"},
 "answer":{"type":"noul","p_yes":0.93},"confidence":null}
```
`subject.kind` ∈ `message | burst | topic_pair | controller`。归属问题的 `subject` 为 `burst`，并附 `message_ids` 与 `candidates`（候选话题 ID 列表），供评估对齐。

分类的逐消息问题使用真实 message ID；整组 urgency/chitchat 使用 burst。旧记录没有可靠映射时，使用共享类型已有的 `unknown` 值与 `id="unavailable"`，`ack.detail.unaligned_answers` 报告数量，并发送 `W_SUBJECT_UNAVAILABLE`；这些答案不能参与需要消息对应关系的校准。model 优先取实际记录，其次取该 request_key 的精确缓存；都没有则为 `unknown` 并告警。

### 3.13 `message`
```json
{"message_id":"m_3f9a1c0b7d2e4a51","sender":"qq:u_A1b2C3example","sender_display":"班长-小王",
 "sent_at":"2026-09-23T21:05:33+08:00","display_text":"@全体成员 周五前把实验报告交到课代表那里[图片]",
 "recalled":false,"system":false,"reply_to":null,"mentions_me":true,
 "topic_id":"t_a19c3b0d77e2","burst_id":"b_0012","cursor":"1790168733000:1532"}
```
`display_text` 为 `render(message, profile)`；`topic_id`、`burst_id` 在尚未切分时为 `null`。

`reply_to` 为 `null` 或 `ReplyRef` 对象，例如 `{"source_message_id":"synthetic-qce-1","resolved":"m_0000000000000001"}`；尚未导入被引用消息时 `resolved=null`，保留来源 ID 供后续补全。

## 4. 错误码

| code | stage | retryable | 含义 |
|---|---|---|---|
| `E_USAGE` | cli | false | 参数错误 |
| `E_CONFIG` | cli | false | 配置缺失/无效、必需密钥不可用、配置版本冲突或保存正被其他进程占用 |
| `E_INPUT_NOT_FOUND` | import | false | 文件不存在 |
| `E_INPUT_PARSE` | import | false | JSON 解析失败或缺少必需字段 |
| `E_INPUT_UNSUPPORTED` | import | false | 不支持的导出形式（如 chunked-JSONL） |
| `E_CHAT_NOT_FOUND` | cli | false | `--chat` 不存在 |
| `E_RUN_NOT_FOUND` | cli | false | 查询的已保存分析运行不存在 |
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
| `E_OUTPUT_WRITE` | cli/render | false | 配置或 HTML 等输出文件写入失败；失败不破坏原配置 |
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
| 8 | 输出文件写入失败 `E_OUTPUT_WRITE`（若分析已提交则用 6） | `failed` |
| 130 | 被取消 | `cancelled` |

## 6. 版本兼容规则

- `schema_version = "MAJOR.MINOR"`，当前 `"1.0"`。
- **MINOR 升级**：只允许新增事件类型、新增可选字段、新增枚举值。旧 GUI 必须能继续工作：忽略未知事件和未知字段，未知枚举值按“其他”显示。
- **MAJOR 升级**：删除/重命名字段、改变字段类型或语义。
- GUI 启动时先运行 `chat-tldr version`：MAJOR 不等于 GUI 编译时的 MAJOR → 拒绝运行，并提示“CLI 协议版本 X 与 GUI 不兼容，请更新”；MINOR 更高 → 正常运行。
- 任何对 DATA_MODEL 中冻结类型的修改都要按上述规则升级版本号，并在群里通知。
- 本次采纳评审稿时尚无已发布的协议使用方，作为首次 `1.0` 冻结；此后的兼容性修改才按上述规则升级。完整合成样例在 `fixtures/jsonl/`，包括空收件箱与部分失败；这些样例验证协议解析，不证明分析功能实现。

## 7. 配置文件（`config.toml`）

课程阶段允许通过 CLI/GUI 将 `api_key` 明文写入本地配置；它优先于 `api_key_env` 指定的环境变量。示例配置不含真实密钥，配置查询只报告来源与是否就绪，见 ADR-0011。CLI 不自动加载 `.env`。

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
