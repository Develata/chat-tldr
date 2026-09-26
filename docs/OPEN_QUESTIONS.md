# 待定问题（OPEN_QUESTIONS）

> 用途：汇总所有 TODO，以及需要 @Develata 拍板的问题。每个问题都给出“当前默认做法”，没有回答之前按默认做法推进，不阻塞开发。
> 读者：全体成员。解决一个问题后，把结论写进对应文档，并在这里标记为 ✅，写明结论与日期。

## A. 需要 @Develata 拍板

| ID | 问题 | 当前默认 | 影响 |
|---|---|---|---|
| Q-DEC-1 ✅ | LLM 客户端：不用 `async-openai`，基于 `reqwest::blocking` 自写 OpenAI / Anthropic 两种客户端，全项目不用 async | **2026-09-26 已接受**，按 ADR-0008 执行 | — |
| Q-DEC-2 ✅ | 是否脱敏 | **2026-09-26 决定：不做脱敏。** 程序与数据库在本机运行；聊天文本仍会发送给 Jev 与 DeepSeek，README 与 GUI 如实告知 | 删除了 `--no-redact`、代号表与还原逻辑 |
| Q-DEC-3 ✅ | 分支保护 | **2026-09-26 已配置**：main 的 ruleset 为 Restrict updates / deletions、必须经 PR、0 个必需审核、只允许 squash、线性历史、禁止 force push，仓库管理员始终可绕过；CI 建好后加必需检查 `fmt`、`clippy`、`test` | 见 ROADMAP 第 1 天、CONTRIBUTING 第 6 步 |
| Q-DEC-4 | 单次 `analyze` 的默认上限：`budget_usd = 0.50`，`max_steps = 64` | 按默认值 | 演示时的成本 |
| Q-DEC-5 | Rust edition 2024，stable 工具链，不设 MSRV | 按默认值 | 骨架 |
| Q-DEC-6 | 评估代码也用 Rust（`chat-tldr-eval` + `plotters` 画图），不用 Python | 按默认值 | 同学 C 的工作量 |

## B. QCE 导出格式（同学 A 用真实样本确认）

依据：QCE commit `7fcca88`（2026-09-11）的源码。以下内容源码中看不出确切形态，需要看真实导出文件。

| ID | 问题 | 当前默认 |
|---|---|---|
| Q-QCE-1 | `content.mentions[].uid` 的实际取值：NT uid、QQ 号，还是 `"unknown"`？`at` 元素的 `data.uin` 是否总是存在？ | uid、uin 都保存；@我 判定同时比较 selfUid 和 selfUin |
| Q-QCE-2 | `system`、`json`、`location`、`av_record` 等元素的 `data` 字段；群公告、入群退群提示分别以什么形式出现 | 输出 `[<type>]` 占位，不提取文字 |
| Q-QCE-3 | 群聊导出时，`chatInfo.peerUid` / `peerUin` 是否就是群号或群 uid？是否总是存在？ | `ChatId = qq:group:<peerUid 或 peerUin>`；两者都没有时报 `E_INPUT_PARSE` |
| Q-QCE-4 | `forward` 元素 `data.messages` 中内部消息的结构（是否与 `messages[]` 的结构相同） | 按与外层相同的结构解析，失败则只保留 `title` |
| Q-QCE-5 | 是否需要支持 chunked-JSONL 导出（`manifest.json` + `chunks/*.jsonl`） | MVP 不支持，报 `E_INPUT_UNSUPPORTED` |
| Q-QCE-6 | 撤回消息在导出中是否保留原文？是否受导出选项影响？`recalled` 字段是否可靠？ | 以 `recalled` 为准，适配器丢弃撤回消息的原文 |
| Q-QCE-7 | 导出时的推荐选项（是否包含系统消息、时间格式等），需要写进 README 的使用说明 | 包含系统消息，JSON 单文件格式 |

## C. Jev

依据：docs.typesafe.ai（2026-09-26 查阅）。请求格式已确认（见 PIPELINE §4.2），以下为未确认事项。

| ID | 问题 | 当前默认 |
|---|---|---|
| Q-JEV-1 | 我们账号的实际限流和额度；`jev-1.13.0` 是否可用（用 `GET /v1/models` 确认） | 按文档：1200 次/分钟；固定版本 `jev-1.13.0` |
| Q-JEV-2 ✅ | 提示词语言 | **2026-09-26 决定**：项目中全部提示词（Jev 的 instructions/criteria、LLM 的模板）一律用英文；聊天内容保持原文（中文或英文），不翻译 |
| Q-JEV-3 | LlmDecider 计算 choice confidence 的公式 `(n·p_max − 1)/(n − 1)` 取自官方 Confidence 页面的交互示例代码，不是正式定义 | 按该公式；报告中注明来源 |
| Q-JEV-4 | 数据处理政策：官方说明不使用客户数据训练，零数据保留（ZDR）只对企业客户提供 | README 与 GUI 首次运行时如实告知 |

## D. 模型与服务

| ID | 问题 | 当前默认 |
|---|---|---|
| Q-LLM-1 ✅ | 评估和演示使用的 LLM | **2026-09-26 决定**：DeepSeek 官方 API，模型 `deepseek-flash`（base URL `https://api.deepseek.com`），所有基线使用同一个；配置见 CLI_PROTOCOL §7 |
| Q-LLM-2 | DeepSeek 的 thinking 模式开不开 | 默认**关闭**：thinking 模式下 `temperature` 不生效，结果无法复现，而且更慢、更贵。如果抽取质量不够，再在消融实验里对比开启的效果 |
| Q-EMB-1 | 如果启用 embedding 粗筛，使用哪个 OpenAI 兼容的 embedding 服务 | 默认不启用；`sim-embed` 基线列为可选 |

## E. 阈值与参数（用标注数据调优）

| ID | 参数 | 初始值 |
|---|---|---|
| Q-TH-1 | `weak_gap_secs` / `strong_gap_secs` / `same_sender_join_secs` / `burst_max_messages` / `topic_close_secs` | 300 / 1800 / 60 / 20 / 21600 |
| Q-TH-2 | `tau_high` / `tau_low`（按 Jev `confidence`） | 0.60 / 0.25 |
| Q-TH-3 | `all_candidates_max` / `state_token_budget` / `candidate_k` / `α, β, γ` | 40 / 24000 / 5 / 0.6, 0.3, 0.1 |
| Q-TH-4 | `direct_max` / `direct_interleave_max` / `segment_batch` | 60 / 0.2 / 300 |
| Q-TH-5 | 层内先验分的权重（PIPELINE §7.2）；反馈的 `η = 0.2`、`λ = 0.05` | 见 PIPELINE |
| Q-TH-6 | quote 最短长度（3 个非空白字符） | 3 |
| Q-TH-7 | `sim_threshold`（相似度基线） | 0.35 |

## F. 评估与协作

| ID | 问题 | 当前默认 |
|---|---|---|
| Q-EV-1 | 1-to-1 overlap、exact-match F1 等指标的准确定义，写报告前核对原论文 | 按 EVALUATION §3.1 的描述实现 |
| Q-EV-2 | 50 条双人标注的第二位标注者是谁 | 同学 C + 同学 A |
| Q-COL-1 | 三位同学的 GitHub 用户名（替换 `.github/CODEOWNERS` 中的占位符） | 占位符 `@TODO-student-a` 等 |
| Q-COL-2 | 测试群成员的同意方式与记录 | 群内公告 + 截图，只存本地 |
| Q-GUI-1 | 打包哪款中文字体（许可证必须允许再分发，如 OFL）以及字体子集化后的体积 | 同学 B 第 1 天确定 |
