# 路线图（ROADMAP）

> 用途：当前交付缺口、真实期限、降级预案，以及保留供参考的早期三天计划。
> 读者：全体成员。每天开始和结束时对照一次。

## 当前安排（2026-09-26 用户更新）

- **实际交付截止：2026-09-27 23:59，America/Santiago（UTC−3）。**
- 主线编码（包含 QCE JSON 导入适配器）主要由 Codex 承担，Develata 参与设计与架构审核；B（@liguilong256）已通过 PR #7 交付 GUI 设计，A/C 的线下进度待确认。QCE 管理由 Codex 接手，PR #8/#9 已合并，向主线交付本地导出文件路径。下方原始分工保留作历史参考。
- CLI 与文件布局草案已经用户批准，见 [CLI_V1_REVIEW.md](CLI_V1_REVIEW.md)。**当前分工以 [TEAM_ASSIGNMENTS.md](TEAM_ASSIGNMENTS.md) 为准**：Codex 主线编码及 QCE 接入，B 做 GUI 设计复核，C 做独立人工标注与验收材料；A/C 尚未绑定账号。
- 文件存放与组件读写归属见 [FILE_LAYOUT.md](FILE_LAYOUT.md)；QCE 管理组件预留在 `apps/qce-manager/`，主线导入适配器保持在 `crates/qce/`。
- 下方“第 1/2/3 天”是原始相对计划，不代表在实际截止日期之后另有开发时间。

各人当前待办见 [TEAM_ASSIGNMENTS](TEAM_ASSIGNMENTS.md)；[tasks/](tasks/) 保存主线模块的历史规格和验收目标，不是同学当前待办台账。

### 当前实现范围

首版冻结无关新功能。既有六个总览见 [ANALYSIS_VIEWS](ANALYSIS_VIEWS.md)；新增更正/取消/冲突与待回应通过独立关系表和只读 `relations` 提供，按已批准 [ADR-0010](decisions/0010-semantic-relations.md) 执行。

验证与测试数量集中记录在 [ACCEPTANCE](ACCEPTANCE.md)，本页不重复累计数字。真实 Ours/B0 各完成 200 条；质量评分与真实 Jev 校准曲线仍为“待标注”。合成结果只验证工具正确性。

CI 已升级 checkout v7.0.1 / rust-cache v2.9.2（Node.js 24，完整 SHA 固定），Linux 使用 Ubuntu 26.04，配置了每周 Actions 依赖更新 PR；并行矩阵与必需检查语义不变。维护与验证方式见 [CONTRIBUTING](../CONTRIBUTING.md#ci-依赖维护)。运行结果仍以当前提交的远程检查为准。

- 已写入：六成员 workspace 与 Cargo.lock、core 协议类型/流校验、QCE JSON 适配器、engine 配置/SQLite 迁移/原子导入/查询，以及 CLI `version/config init/doctor/import/chats/messages`。
- 本轮完成 CLI 基础闭环：`analyze`、带原文证据的 `inbox`、`feedback`、`resolve`、`mark-read`，以及 `analyze --html` / `inbox --html`。已读只推进到收件箱返回的安全前缀；反馈替换当前评价，排序调整有界且不改变优先级。
- 分析包含基础话题分配、结构化抽取、时间规范化、证据校验、规则 @我 和 P0–P3 排序。同步 Jev / DeepSeek 客户端、OpenAI / Anthropic 兼容接口及 Mock 已实现；网络协议与异常路径通过本地假服务器验证，未以真实云调用替代测试。
- SQLite 保存按话题提交的检查点、模型响应缓存、决策和用量。未完成消息可继续分析；步数、费用预估、超时、有限重试和取消约束执行。`analyze --dry-run` 无需密钥，只读、不联网、不写缓存。
- QCE Docker 字段核对与导出说明见 [QCE_DOCKER_EXPORT.md](QCE_DOCKER_EXPORT.md)，对应 [template-docker-export.json](../fixtures/qce/template-docker-export.json) 为手写合成模板。本机 CRLF 入口故障已修复、用户已 QQ 登录；QCE 容器服务正常，宿主 40653 发布仍异常，使用仅本机的 40654 独立转发入口。已完成首个真实单文件导出（200 条）及 `scripts/verify-qce.ps1` 51 项离线检查：重复导入新增 0、消息计数/协议/游标与源文件不变检查通过。JSON 卡片正文未归一化；没有据此验收未出现的数据形态或云模型质量，见 [ACCEPTANCE](ACCEPTANCE.md)。
- 实际调用模型需要有效密钥，可通过 CLI/GUI 保存在本地 config（课程阶段明文），或使用环境变量；聊天原文会发送到配置的云服务。无密钥预演、模型配置及 HTML 导出示例见 [README](../README.md)。
- CI 必需检查名保持 `fmt`、`clippy`、`test`。Windows 全量并行检查 core/qce/engine/cli/gui/qce-manager/eval，macOS 并行检查 CLI/GUI/eval/manager，Linux 检查 CLI/eval/manager 并执行原生 GUI smoke。模块使用 `fail-fast: false` 与 `--locked`；共享规划脚本校验 workspace 覆盖，汇总拒绝应跑任务失败、取消或意外跳过。外部 Actions 固定完整 SHA，质量 CI 只读；正式发布 job 申请 Release/GHCR 写权限。当前提交的结果以 CI 为准，平台与显示设备覆盖边界见 [RELEASING](RELEASING.md#gui-自动化验收)。
- `stats/decisions/jev-log` 历史查询 CLI 已实现，查询不建库、不调用模型；新记录保存可重放的决策与真实 subject。旧记录缺失精确计数或归属时明确告警，不能用于伪造评估对齐。
- 当前支持 `--strategy ours|b0`，不代表完整 PIPELINE 策略。话题合并、自动关闭和保持 Closed 的历史回填已实现。控制器由规则确定合法集合，多候选时 Jev 选择，失败回退规则；合并仍安排在抽取/校验之后。embedding、B1/sim-* 后续补齐。
- GUI 通过 CLI 工作，已接入 QCE 获取向导和模型设置；Windows GUI/manager 从 `v0.2.0` 纳入发布，新关系界面后续补齐。eval 已支持 Ours/B0 抽取/排序、话题匹配/ARI/NMI、calibrate 的 ECE/Brier 与 SVG；人工质量、burst/边界、agreement/summarize 尚未完成。当前分工见 [TEAM_ASSIGNMENTS](TEAM_ASSIGNMENTS.md)，验收见 [ACCEPTANCE](ACCEPTANCE.md)。
- 统计 `calls` 是逻辑模型调用数，token/费用是已报告用量，缓存不重复计费；不能据此宣称实际云账单或模型质量已验证。本批已运行 `codegraph sync`，索引保留在本地。
- 审查修复记录见 [REVIEW_FIXES](REVIEW_FIXES.md)。Mock 与本地 HTTP 测试不能证明真实质量；合并以当前提交的 CI 为准。CLI/Docker 每次 push 模拟发布，正式 tag 才创建 Release 和推送 GHCR，见 [RELEASING](RELEASING.md)。
- 下方多日清单保留为原始范围参考，时间与负责人以本页“当前安排”和 TEAM_ASSIGNMENTS 为准；混合多个功能的条目拆开标注，未完成项不作已交付宣传。
- 话题合并、Closed 回填、恢复及输出中断的回归证据见 REVIEW_FIXES；已定决定 Q-DEC-7 保持不变。

## 当前待完成与待验收

| 项目 | 负责人 | 当前状态与退出条件 |
|---|---|---|
| 真实云服务与分析效果 | Codex 联调，Develata 复核 | Ours/B0 新 profile 均完成 200 条；退出码/用量/降级见 ACCEPTANCE，真实质量待标注 |
| 更正/取消关联、待回应语义 | Codex，Develata 审核设计 | 已接入独立关系表/查询，验收事务迁移、历史窗口、撤回、部分回答和用户状态不变 |
| 控制器、embedding、其余 eval/基线 | Codex，Develata 决定交付取舍 | Jev 控制器、B0、话题/校准工具已实现；embedding、B1/sim-* 和其余指标后续补齐 |
| QCE 管理程序 | A 席位 | 目前只有说明；按已核对的导出接口封装获取/启动/导出/受控清理，成功才交付完整 JSON 路径 |
| GUI 设计与补充验收 | B 席位，Codex 接入 | 主线 GUI 已实现；独立图稿、新总览原生截图、真实流程体验复核待交付 |
| 独立 gold、报告与演示 | C 席位，Develata 复核 | 共用样本和工具已就绪；独立标注、双人一致性、真实结果报告/演示仍待交付 |
| 协作权限与合并门槛 | Develata | 线上 Protect main 已有严格的 fmt/clippy/test 必需检查（本轮核验时已存在）；账号绑定与同学实际合并权限仍待确认 |

已有能力、各人下一份具体产物与数据边界以 [README](../README.md)、[TEAM_ASSIGNMENTS](TEAM_ASSIGNMENTS.md)、[ACCEPTANCE](ACCEPTANCE.md) 为准。实际期限前保留已通过回归的功能，剩余范围由 Develata 根据验收结果决定。

<details>
<summary>早期三天计划与降级顺序（历史参考，不作为当前分工或进度）</summary>

以下保留原始相对日程与验收目标；其中 A/B/C 原计划中的适配器、GUI、评估工具代码已由 Codex 承担。勾选不表示挂名同学已经交付，当前完成者、缺口及线上规则以本页上方和 TEAM_ASSIGNMENTS 为准。

## 原始里程碑

| 时间 | 里程碑 | 验收 |
|---|---|---|
| 第 1 天 12:00 | **骨架冻结** | workspace 可编译；CI 绿；`crates/core` 四个类型合入 main；fixtures 可用 |
| 第 2 天 12:00 | **模块可用** | qce 能解析 fixtures；CLI 能 import + 用 Mock 模型 analyze；GUI 能展示 mock JSONL |
| 第 2 天 20:00 | **第一次端到端联调** | 真实 QCE 文件 → CLI（真实 Jev + LLM）→ GUI 显示收件箱并能点开证据 |
| 第 3 天 12:00 | **功能冻结** | 之后只修 bug，不加功能 |
| 第 3 天 20:00 | **交付** | 评估数字、报告、演示视频 |

## 第 1 天

### 上午

**@Develata**
- [x] 建六成员 workspace 并锁定依赖：`crates/core`、`crates/qce`、`crates/engine`、`apps/cli`、`apps/gui`、`eval`；GUI 接入与验收状态见上方
- [x] 在 `crates/core` 实现冻结类型、`ImportBatch` / `ChatMeta`、事件 payload 与流校验，附序列化测试
- [x] `.github/workflows/ci.yml`：保留 `fmt`、`clippy`、`test` 必需检查名称；clippy/test 分模块并行后汇总，覆盖全部 workspace 成员
- [x] QCE 合成小样例、Docker 格式合成模板，以及 `fixtures/jsonl/` 下的分析、收件箱、会话与消息协议样例
- [x] 分支保护（ruleset “Protect main”，已配置）：Restrict updates / deletions；必须经 PR；0 个必需审核；只允许 squash；线性历史；禁止 force push；仓库管理员始终可绕过。按 GitHub 文档，Restrict updates 表示只有具备 bypass 权限的用户能更新 main，合并 PR 预计也受此限制，因此预计只有 @Develata 能合并。待办：用一位同学的账号开一个测试 PR 实测；如果同学也能合并，就把“只由 @Develata 合并”作为约定写进群公告
- [x] ruleset 已有必需检查 `fmt`、`clippy`、`test`（2026-09-26 核验），job 名保持一致
- [ ] 把三位同学加为仓库 collaborator（Write 权限），确认实际任务后把真实用户名追加到 CODEOWNERS 的对应路径
- [ ] 在群里通知：类型已冻结，`schema_version = 1.0`

**同学 A**
- [ ] 读 [ARCHITECTURE.md](ARCHITECTURE.md)、[DATA_MODEL.md](DATA_MODEL.md) §1–§2、[tasks/task-A-qce-temporal-verify.md](tasks/task-A-qce-temporal-verify.md)
- [ ] 用 QCE 导出一份自己的测试群（**不要提交**），对照 DATA_MODEL §2.1 的元素表逐一确认字段，把发现记到 [OPEN_QUESTIONS.md](OPEN_QUESTIONS.md) 的 Q-QCE-* 条目（通过 PR 修改）
- [ ] 按 [CONTRIBUTING.md](../CONTRIBUTING.md) 完成 Git 环境配置，并试着开一个只改文档的 PR

**同学 B**
- [ ] 读 ARCHITECTURE §4.4、[CLI_PROTOCOL.md](CLI_PROTOCOL.md)、[tasks/task-B-gui.md](tasks/task-B-gui.md)
- [ ] 本地跑通 eframe_template 和 egui demo，了解 `SidePanel`、`CentralPanel`、`ScrollArea`、`CollapsingHeader`
- [ ] 选定中文字体（OFL 许可），确认文件大小
- [ ] 同 A，完成 Git 环境配置

**同学 C**
- [ ] 读 [EVALUATION.md](EVALUATION.md)、[tasks/task-C-eval.md](tasks/task-C-eval.md)
- [ ] 联系测试群，取得全体成员同意，并用 QCE 导出约 200 条消息（**只存在本地** `eval/private/`）
- [ ] 熟悉标注规范（EVALUATION §5），用 10 条消息试标一遍，把拿不准的情况记下来
- [ ] 同 A，完成 Git 环境配置

### 下午（开始并行开发）

**@Develata**
- [x] `engine::store`：迁移、表结构、WAL、事务；`import`（去重、别名、游标、悬空引用补全、回填计数）
- [x] `apps/cli`：`version`、`import`、`chats`，JSONL writer（stdout 单写者），错误码 → 退出码映射
- [x] 幂等、重复/重叠导入、回填和批次回滚测试

**同学 A**
- [x] `crates/qce`：`parse_qce_json` 与元素规范化；具体支持边界见 [fixtures/qce/README](../fixtures/qce/README.md)
- [x] 元素、撤回、缺身份、未知字段等合成测试；真实导出验收另行完成

**同学 B**
- [ ] GUI 骨架：三栏布局 + 中文字体 + 读取 `fixtures/jsonl/*.jsonl` 并展示

**同学 C**
- [x] `eval` crate：`export-sheet` / `import-sheet`（由 Codex 主线实现；C 继续负责合成场景与人工验收材料）
- [ ] 开始标注（话题 + 消息级标签）

## 第 2 天

**@Develata**
- [x] `render` 与基础话题处理（burst、时间候选、回复归属和 Decider 归属）
- [x] active 话题候选的回复/@增量索引、连边/时间排序、请求预算裁剪、中等置信度 LLM 复核及可重放的实际归属日志
- [x] `JevDecider`、`LlmDecider`、`OpenAiCompatClient`、`AnthropicCompatClient`、Mock；缓存与用量记录
- [x] `extract`（AnalyzeTopic、AnalyzeDirect、MentionMe 规则）
- [x] 基于规则动作选择的 `agent`、检查点、决策日志与 `rank`
- [x] `MergeTopics`：已完成 active 话题的回复候选、Decider 确认、原子迁移与拒绝记忆、恢复与预算约束
- [x] 消息时间驱动的自动关闭、Closed 历史候选与小积压归属、首次关闭边界持久化及回填不复活
- [ ] 完整策略：控制器 Jev 动作选择、优先合并脏话题与 embedding 候选
- [x] CLI：`analyze`、`inbox`、`messages`、`feedback`、`resolve`、`mark-read`
- [ ] 查询 CLI：`stats`、`decisions`、`jev-log`
- [ ] 审核并合并 A、B、C 的 PR（尽量在 2 小时内响应）

**同学 A**
- [ ] 上午：qce 收尾（真实样本上零 panic）
- [x] `engine::temporal`：已实现高频规则与边界测试，其他表达仍保留原文或标注低置信推测
- [x] `engine::verify`：`verify`、`find_quote` 与 Unicode、撤回、截止日期等边界测试

**同学 B**
- [ ] 子进程运行器：后台线程 + channel + `try_recv` + `request_repaint`；启动时 `version` 握手
- [ ] 收件箱：点开结论显示证据原文并高亮 `highlight` 区间；“有用 / 不重要”、“完成 / 忽略”按钮
- [ ] “开始汇总”按钮 + 进度条；“标为已读”按钮（回传 `view_cursor`）
- [ ] 决策日志面板、运行统计面板（消息数、话题数、被拒结论数、token、费用）

**同学 C**
- [ ] 完成 200 条标注；与另一位同学完成 50 条双人标注
- [x] `chat-tldr-eval score`：Ours 抽取/Deadline/排序、快照 rejected 比例与单次运行统计，含手算和完整 CLI 回归（Codex 实现）
- [ ] score 其余指标：话题/边界、人工支持率、所有原始提案的 unsupported 比例；真实标注与质量验收
- [ ] README 与报告框架

**第 2 天 20:00 联调**（全员）：用 C 的真实导出跑通 import → analyze → GUI，记录问题清单，分配修复。

## 第 3 天

- [x] CLI `--html` 独立收件箱导出，可用浏览器演示
- [ ] **主线**：修联调问题；`--strategy b0 / b1 / sim-tfidf`；按实际截止时间完成冻结
- [ ] **同学 A**：修 bug；补充 temporal、verify 的边界用例；协助 C 跑实验
- [ ] **同学 B**：修 bug；GUI 打磨；准备演示用的数据目录
- [ ] **同学 C**：跑全部系统（EVALUATION §6）、`calibrate`、汇总表；报告；录演示视频（GUI 为主，`--html` 为备用）
- [ ] 全员：报告的“局限性”部分（聊天文本发送到云服务、Jev 中文精度、样本规模小）

## 砍需求顺序（落后时从上往下砍）

1. `sim-embed` 基线与 embedding 粗筛（默认本来就不启用）
2. `MergeTopics`（规则永远不提议，枚举保留）
3. LLM 复核档（`tau_low ≤ confidence < tau_high` 改为直接归入最高项）
4. 反馈个性化（按钮保留，只记录不生效）
5. B1 基线
6. GUI 决策日志面板（CLI `chat-tldr decisions --run <ID>` 已可重放历史；GUI 展示验收另计）

**原定交付底线**：证据校验、幂等导入、@我 规则、JSONL 协议、Ours vs B0 对比、Jev 校准曲线。当前对照与校准工具已实现，合成流程及真实运行/成本结果见 [ACCEPTANCE](ACCEPTANCE.md)；真实质量比较与真实校准曲线仍待人工标注，不能以 Mock 测试替代。

</details>

## 降级预案

| 故障 | 预案 |
|---|---|
| Jev 不可用 | 已支持自动降级并输出 `W_DECIDER_FALLBACK`，或显式 `--decider llm`；效果数字仍需实际评估 |
| 话题切分失败 | 当前保留失败状态后重试；计划中的 `--strategy b1` 尚不可用 |
| 截止日期规范化不确定 | 只保留 `raw` |
| GUI 出问题 | `chat-tldr inbox --chat <ID> --html demo.html` 或 `overview --chat <ID> --html overview.html`，用浏览器演示 |
| 真实测试群数据来不及 | 用合成集完成全部流程，报告中如实注明 |

## 非目标

- 将 QCE 源码链接进主程序，或由主线自行实现 NapCat 实时抓取；微信支持；实时机器人。独立 QCE 管理组件可由同学提供，并通过导出文件与主线集成
- 向量数据库、RAG、Agent 框架（如 Rig、LangChain）、模型微调、训练话题切分模型
- 开箱即用的完整本地模型分析（默认配置使用云服务）；导入和查询本身已可离线运行
- 完整的中文时间解析库（只做高频规则）
- QCE 的 chunked-JSONL 导出格式
- 多用户、多设备同步

## 未来工作

- 支持 QCE chunked-JSONL 与更多导出工具
- 本地 embedding（fastembed-rs）与本地 LLM（Ollama），减少云端依赖
- 更完整的时间表达式解析（农历、节假日、“月底”“学期末”）
- 并行执行 AnalyzeTopic
- 用积累的反馈与标注做离线阈值搜索
