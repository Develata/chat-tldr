# CLI v1 合作接口评审稿

> 状态：**2026-09-26 已获用户采纳，首次 v1 的正式契约已同步到 CLI_PROTOCOL.md、DATA_MODEL.md 与 PIPELINE.md。**
> 本文保留评审决定的依据；契约获采纳不代表全部功能已实现。基础 CLI 当前范围是 version、config init、doctor、import、chats、messages，后续能力以 version.capabilities 为准。
> 已确认：交付截止为 2026-09-27 23:59，America/Santiago（UTC−3）；Codex 负责主线编码，Develata 负责设计与架构审核，同学先承担外围任务。

仓库与运行文件的统一位置见 [FILE_LAYOUT.md](FILE_LAYOUT.md)。`--data-dir` 包含业务数据库和各组件自己的子目录；QCE 原始导出与可清理缓存分开存放。`doctor` 的 `ack.detail.paths` 应报告解析后的 `data_dir`、`config_file`、`database`、`outputs_dir` 和 `qce_exports_dir`，GUI 据此展示和编排。

## 1. 已采纳方案

沿用当前的命令名称与 CLI 子进程架构，补全行为约定。GUI、评估工具和人工脚本调用同一个 `chat-tldr` 二进制。

- 每次启动执行一个命令；输入通过参数和本地文件提供，不出现交互式输入提示。
- 业务命令的 stdout 永远是 UTF-8 JSONL；stderr 是日志；程序不根据终端类型自动改变格式。
- GUI 不读数据库，不根据日志文本推断结果。共享协议类型由 `crates/core` 提供。
- 导入、分析、查看、标已读、完成事项是独立操作；查询不会隐式调用模型或修改业务状态。
- 已定模型、P0 规则、证据规则保持现有方案。截止事项即使由别人负责，也进入 P0（TopicSummary 例外）。

数据入口已由用户确认：**QCE JSON 导入适配器由 Codex 实现，属于分析主线。** 另一位同学负责 QCE 的拉取、下载、清理和集成管理，向主线交付导出完成的本地 JSON 文件路径，再调用 `import`。

按组件组合实现“内嵌功能”：GUI 可以编排 QCE 管理组件 → 取得导出文件 → import → analyze；主 CLI 不承担 QCE 安装、登录或进程管理。QCE 管理组件的具体实现与打包方式由外围任务另行设计，本稿没有授权清理任何用户原始数据。

## 2. 命令表

以下名字除 `config init`、`doctor` 外均来自现有 CLI_PROTOCOL。新增两项用于首次配置和排查配置问题。

| 命令 | 输入与行为 | 输出事件 | 业务状态写入 / 模型调用 |
|---|---|---|---|
| `version` | 返回程序、协议、数据库版本及支持的命令 | `ack`, `done` | 无 / 无 |
| `config init [--out FILE]` | 写示例配置，默认 `<data-dir>/config.toml`；目标存在就报错，不覆盖 | `ack`, `done` | 仅显式创建配置 / 无 |
| `doctor` | 检查配置、环境变量是否存在、数据目录及数据库版本；不展示 key，不测试远端 API | `ack`, `done`；失败附 `error` | 无 / 无 |
| `import PATH... [--self-uid UID] [--self-uin UIN]` | 导入 QCE 单文件 JSON，身份覆盖值保存到该会话 | `progress`, `warning`, `ack`, `stats`, `done` | 有 / 无 |
| `chats` | 列出会话及导入、分析、已读进度 | `chat`*, `done` | 无 / 无 |
| `analyze --chat ID` | 分析该会话尚未处理的消息，失败后再次运行同一命令即可续做 | `progress`, `decision`, `warning`, `error`, `topic`, `insight`, `stats`, `done` | 有 / 有 |
| `inbox --chat ID` | 获取一致的收件箱视图及证据 | `inbox`, `topic`*, `insight`*, `done` | 无 / 无 |
| `messages --chat ID [--since TIME] [--until TIME]` | 查询原文的渲染视图，按消息游标排序 | `message`*, `done` | 无 / 无 |
| `feedback ID (--useful \| --not-important)` | 设置该结论当前的偏好反馈（见 §5） | `ack`, `done` | 有 / 无 |
| `resolve ID (--done \| --dismiss \| --reopen)` | 设置事项生命周期，不改变证据校验状态 | `ack`, `done` | 有 / 无 |
| `mark-read --chat ID --up-to CURSOR` | 将已读位置推进到 GUI 展示过的安全位置 | `ack`, `done` | 有 / 无 |
| `decisions --run ID` | 导出指定运行的决策记录 | `ack`, `decision`*, `done` | 无 / 无 |
| `jev-log --run ID` | 导出指定运行的模型判断与概率 | `ack`, `jev_answer`*, `done` | 无 / 无 |
| `stats [--chat ID \| --run ID]` | 无筛选为全局统计；两个筛选参数互斥 | `stats`, `done` | 无 / 无 |

`*` 表示零条或多条。所有命令均可在终止前发出结构化 `error`；不能把表中没有列出 `error` 理解为命令不会失败。

### 全局参数与配置

- 全局参数：`--data-dir DIR`、`--config FILE`、`-v` / `-vv`，允许放在子命令前后。
- 优先级：本次显式参数 > 显式配置文件 > 数据目录内的配置 > 内置默认值。
- 显式指定的配置文件不存在或格式错误时，返回 `E_CONFIG`；不静默忽略。
- 默认配置文件不存在时，离线命令仍可用；`analyze` 才校验实际需要的模型配置与密钥。
- `version` 和帮助不读取配置、不初始化数据库；空数据目录下 `chats` 返回空列表。
- `doctor` 报告 `read`、`import`、`analyze` 三类能力分别是否就绪。缺模型 key 时给出 `E_CONFIG` / 退出码 4，但说明离线能力是否仍可用。
- 密钥仍只来自环境变量。配置文件只保存环境变量名；普通输出与错误信息均不包含密钥。
- `--self-uid` / `--self-uin` 仅属于 `import` 参数；缺失时保留已存身份，新会话才回退到文件元数据。显式值与已有身份冲突时拒绝导入并返回 `E_CONFIG`，不在查询或分析时悄悄切换用户身份。
- `import` 成功提交后发出 `ack`，`detail.chat_ids` 返回本次涉及的会话 ID（去重后按字符串排序），便于 GUI 和导出组件选择后续分析对象。`changed` 表示业务数据是否变化，不把运行日志新增算作业务变化。
- 缺少自身身份时，个人 @ 无法识别并告警；`@全体成员` 仍成立。
- 项目交付时间的 UTC−3 与聊天时间解析的默认 `+08:00` 是不同设置；不因此改动聊天时间配置。

### analyze 参数

保留 `--decider jev|llm`、`--strategy ours|b0|b1|sim-tfidf|sim-embed`、`--max-steps N`、`--budget-usd X`、`--since TIME`、`--until TIME`、`--html FILE`。

- 时间参数必须是带偏移的 RFC 3339；查询与分析统一用 `[since, until)`。这与内部 Cursor 的 `MessageRange(after, up_to]` 分开定义。
- 命令启动时固定本次待处理消息集合；运行期间新导入的消息留到下一次。
- 时间范围只允许对应消息推进分析状态；范围外的消息可以作为上下文，但不能顺带被标为已分析。
- 没有工作时正常 `done(complete)`，不调用模型。
- `--max-steps` 必须为正整数；预算必须是有限正数。
- 预算在发起模型请求前按配置单价、输入估算与输出 token 上限预留；不足则停止发起新请求。它限制本地估算费用，不能声称是远端账单的绝对保证。
- `--dry-run` 定义为**只读计划**：不联网、不写业务数据、不推进游标、不写缓存，输出 `ack.detail.plan` 与 `done`。它报告待分析量和配置就绪情况，不声称完成了分析。与 `--html` 同时指定时返回 `E_USAGE`。

### inbox 参数

- 保留 `--include-resolved`、`--include-rejected`、`--html FILE`。
- 增加 `--all`：只关闭“上次查看以来”的展示窗口，生命周期与校验过滤仍由前两项单独控制。便于评估取完整结果。
- 输出顺序固定为 P0 → P3、层内 `rank_score` 降序、最后按 `InsightId` 升序打破同分。
- 元数据、counts、topic 和 insight 必须来自同一个数据库读快照。
- `--html` 与 stdout 使用同一视图，先写临时文件再替换目标。写失败发出 `E_OUTPUT_WRITE`；单独查询命令退出 8，分析已提交结果时退出 6（partial），说明分析数据仍保留。
- 尚无可安全标为已读的位置时，`view_cursor = null`，GUI 禁用“标为已读”。

## 3. 一次调用的协议

继续使用现有信封：

```json
{"schema_version":"1.0","run_id":"r_20260926T120000_001a","seq":0,"event":"ack","payload":{"command":"version","target":null,"changed":false,"detail":{"cli_version":"0.1.0","schema_version":"1.0","db_version":1,"capabilities":{"commands":["version","config init","doctor","import","chats","messages"],"strategies":[],"deciders":[]}}}}
{"schema_version":"1.0","run_id":"r_20260926T120000_001a","seq":1,"event":"done","payload":{"status":"complete","exit_code":0,"elapsed_ms":1}}
```

以上是示例，不表示这些命令已经实现。`capabilities` 必须反映二进制实际支持的能力，不能把空桩列为可用。

- `run_id` 标识**当前 CLI 调用**，`seq` 从 0 连续递增；每条事件及时 flush。
- 正常可写的 stdout 上，恰好一个 `done`，且它是最后一行。进程被强杀、崩溃、管道断开属于不能保证输出 `done` 的情况。
- `done.exit_code` 与进程退出码必须一致；GUI 读到 `done` 后仍检查进程是否正常结束。
- `error` 描述具体失败；命令总体是否完整成功以 `done.status` 为准。
- 未知 event 可忽略并记录；已知 event 解析失败、序号断裂、重复 `done` 视为协议错误，GUI 不把结果当成完整成功。
- 新增可选字段和未知字段按现有 MINOR 兼容规则处理；共享枚举需要可容纳未知值的反序列化路径。
- `--help` / `-h`、`--version` 是给终端使用的标准文本例外；GUI 握手调用 `version` 子命令。参数错误仍输出 JSONL `error` + `done`，提示放 stderr。
- 重放命令的首个 `ack.target` 指向被查询的历史 RunId，随后事件信封仍使用本次调用的 RunId；不能把导出调用误算为一次分析。
- `analyze` 的结论事件只在对应检查点提交后输出。GUI 在命令结束后重新调用 `inbox`，以完整快照刷新，不靠拼接过程事件维护最终收件箱。

部分成功示例（省略 progress 和完整 stats，仅展示失败与结束事件）：

```json
{"schema_version":"1.0","run_id":"r_20260926T121000_002b","seq":0,"event":"error","payload":{"stage":"extract","code":"E_PROVIDER_TIMEOUT","retryable":true,"message":"一个话题请求超时；已完成话题保留","topic_id":"t_example"}}
{"schema_version":"1.0","run_id":"r_20260926T121000_002b","seq":1,"event":"done","payload":{"status":"partial","exit_code":6,"finish_reason":"error","elapsed_ms":120001}}
```

## 4. 错误与恢复

保留现有退出码 0、1、2、3、4、5、6、7、130；增加 8 表示输出文件写入失败。警告本身不必导致非零退出码。

| 场景 | 推荐行为 |
|---|---|
| 一条命令缺必需参数、互斥参数并用 | `E_USAGE`，退出 2，业务数据不变 |
| `import A.json B.json` 中 B 无效 | 整个批次回滚，退出 3；用户可只重导有效文件 |
| 同一文件/重叠文件重导 | 不重复插入；统计明确区分新消息和重复消息 |
| Jev key 缺失、LLM 可用 | `W_DECIDER_FALLBACK`，按已定方案降级 |
| 分析一部分话题后发生模型失败 | 已提交话题保留，退出 6，刷新收件箱仍可用 |
| 开始分析前发现必需配置不可用 | 退出 4，不标任何消息为分析完成 |
| 同一 chat 第二个 analyze | `E_RUN_IN_PROGRESS`，退出 7；不同 chat 可分别运行 |
| Ctrl-C | 尽快停止后续步骤，保留检查点；能够正常收尾时 `cancelled` / 130 |
| GUI 强杀子进程，未收到 done | GUI 显示异常中断；下次 analyze 从检查点续做 |
| 命令失败后重试 | import、resolve、mark-read 按幂等语义重试；analyze 从持久化状态续做 |

`--dry-run` 和普通查询不调用外部服务，不要求 key，也不修改业务状态。数据目录不存在时的空查询不自动创建数据库；写命令负责显式初始化。

## 5. 需要明确的用户操作语义

### 反馈与事项状态

`feedback` 表示**设定当前评价**：连续发送两次 `--useful`，第二次 `ack.changed=false`，不会重复放大偏好。切换到 `--not-important` 时替换当前有效评价；审计记录可以保留，但不能把一次切换当成两次独立有效投票。具体权重重算由 engine 实现。

`resolve` 同样是设置状态：重复 `--done` 是无变化成功；`--reopen` 只改变 lifecycle，不把 rejected 变成 verified。反馈不修改 lifecycle，resolve 不修改反馈，两者都不推进已读游标。

### 已读边界

GUI 原样回传最近完整显示的 `inbox.view_cursor`。CLI 检查游标格式、所属会话以及允许推进的边界；旧游标返回 `changed=false`。

**已采纳的边界**：`view_cursor: Option<Cursor>` 止于连续 `done/skipped` 前缀，不能越过 `failed/pending` 消息。`last_analyzed` 把 failed 也算“已了结”，不能直接作为已读上界；无安全位置时返回 null。PIPELINE 与 inbox payload 已同步，不增加第四个持久化游标。

CLI 能校验位置是否合法，不能仅凭一个 Cursor 证明人确实看过界面；“只回传已展示的位置”由 GUI 契约保证，不虚构服务端的展示凭证。

## 6. GUI 与同学的最短接入路径

QCE 管理组件交付约定：文件必须已经写完并关闭，推荐由临时文件原子改名后才通知调用方；失败时不触发 import。主 CLI 只读输入文件，不移动或删除它。清理仅由管理组件按另行确认的规则处理，不能把 import 成功解释成可删除用户原始导出。

1. 开发期直接读取 `fixtures/jsonl/` 的版本握手、会话列表、收件箱、分析成功、分析部分失败样例。
2. 运行时：`version` → `chats` → 选群后 `inbox`。
3. 导入：文件选择或 QCE 管理组件返回文件 → `import` → 从 `ack.detail.chat_ids` 取得会话 ID，刷新 `chats` 和当前 `inbox`。
4. 分析：`analyze` 的 progress 更新进度，decision 进入日志；complete/partial 后刷新 `inbox`。
5. 反馈、完成、忽略、重新打开：调用对应命令，成功后刷新 `inbox`。
6. 标为已读：传回非空 `view_cursor`，成功后刷新 `chats` 和 `inbox`。
7. 两条管道分别读取；后台收到事件时唤醒 UI，UI 线程不等待子进程。

GUI 初版只需要消费 `ack/chat/inbox/topic/insight/progress/warning/error/done`；运行统计与决策日志可随后接入。未知事件不能让前期界面崩溃。

评估继续使用独立数据目录、同一个 CLI。`inbox --all --include-resolved --include-rejected` 用于当前结果的完整导出。B0/B1 的模型原始排序、校验前后结果不能从重排后的 inbox 反推；它们需要独立定义分析事件中的原始输出记录，在实现基线前补齐，不能宣称现在的事件已经覆盖全部评估需求。

## 7. 采纳与实现边界

对外合作前优先冻结：命令与参数、事件信封、GUI 必需 payload、错误/结束语义、变更操作的幂等规则。内部算法阈值保持可配置。

本次已采纳三项：

1. 默认 JSONL、标准帮助文本例外，新增 `config init` / `doctor`。
2. `feedback` 采用“当前评价”，防止重复调用改变效果。
3. 多文件 import 整批原子化、dry-run 全只读、失败消息不跨越已读边界。

正式文档已同步，共享类型已落地；`fixtures/jsonl` 提供版本握手、会话、空/非空收件箱、正常/部分分析结束的纯合成样例。当前尚无已发布的协议使用方；本次作为首次 v1 冻结处理，后续严格按 MAJOR.MINOR 升级。

完整实现验收仍需覆盖：无配置/无 key 的离线调用、错误参数的 JSONL、空收件箱、正常与部分失败结束、导入批次回滚、幂等操作、未知事件兼容、失败消息的已读边界。core 的协议序列化、未知事件/枚举兼容、已知事件畸形拒绝、流完整性和六份合成样例解析已有测试；样例解析不等于实际分析、收件箱或 mark-read 的功能验收。各命令的实际验证以对应集成测试为准。
