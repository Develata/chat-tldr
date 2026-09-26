# 路线图（ROADMAP）

> 用途：3 天的任务清单（按人拆分，可勾选）、里程碑、砍需求的顺序、降级预案，以及非目标与未来工作。
> 读者：全体成员。每天开始和结束时对照一次。

各人详细的任务说明见 [tasks/](tasks/)。

## 里程碑

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
- [ ] 建 workspace：`crates/core`、`crates/qce`、`crates/engine`、`apps/cli`、`apps/gui`（基于 eframe_template）、`eval`；锁定依赖版本
- [ ] 在 `crates/core` 实现四个冻结类型 + `ImportBatch` / `ChatMeta` + 各事件 payload，附序列化往返测试
- [ ] `.github/workflows/ci.yml`：`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`；使用 `Swatinem/rust-cache`；Linux 安装 GUI 依赖（参照 eframe_template：`libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev`）。**必须在 Cargo.toml 合入的同一个 PR 里加入**，否则 CI 在没有 Cargo.toml 的仓库上会一直失败
- [ ] `fixtures/`：2 份合成的 QCE 导出（有重叠，含 @全体成员、回复、撤回、图片、合并转发、系统消息）、`fixtures/jsonl/` 下的 mock 输出（`analyze.jsonl`、`inbox.jsonl`、`chats.jsonl`、`messages.jsonl`）
- [ ] 分支保护：main 禁止直接推送；需要 CI 通过；需要 1 个 Code Owner 审核；只允许 squash merge；@Develata 自己的 PR 通过管理员权限合并（或请一位同学点 Approve）
- [ ] 把三位同学加为仓库 collaborator（Write 权限），把 CODEOWNERS 里的占位符换成真实用户名
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
- [ ] `engine::store`：迁移、表结构、WAL、事务；`import`（去重、别名、游标、悬空引用补全、回填计数）
- [ ] `apps/cli`：`version`、`import`、`chats`，JSONL writer（stdout 单写者），错误码 → 退出码映射
- [ ] 幂等测试：同一 fixture 导入两次、两份重叠 fixture 先后导入

**同学 A**
- [ ] `crates/qce`：`parse_qce_json`，覆盖 text / at / face / market_face / image / video / audio / file / reply / forward / system / 未知类型
- [ ] 每种元素一个单元测试；撤回消息、缺 selfUid 的测试

**同学 B**
- [ ] GUI 骨架：三栏布局 + 中文字体 + 读取 `fixtures/jsonl/*.jsonl` 并展示

**同学 C**
- [ ] `eval` crate：`export-sheet` / `import-sheet`
- [ ] 开始标注（话题 + 消息级标签）

## 第 2 天

**@Develata**
- [ ] `render`（含脱敏）、`segment`（burst、候选、归属、关闭）
- [ ] `JevDecider`、`LlmDecider`、`OpenAiCompatClient`、`AnthropicCompatClient`、Mock 实现；缓存与用量记录
- [ ] `extract`（AnalyzeTopic、AnalyzeDirect、MentionMe 规则）
- [ ] `agent`（观测、规则、Jev 选择、检查点、决策日志）、`rank`
- [ ] CLI：`analyze`、`inbox`、`messages`、`feedback`、`resolve`、`mark-read`、`stats`、`decisions`、`jev-log`
- [ ] 审核并合并 A、B、C 的 PR（尽量在 2 小时内响应）

**同学 A**
- [ ] 上午：qce 收尾（真实样本上零 panic）
- [ ] `engine::temporal`：规则表 + 测试（至少 40 个用例）
- [ ] `engine::verify`：`verify` + `find_quote` + 测试

**同学 B**
- [ ] 子进程运行器：后台线程 + channel + `try_recv` + `request_repaint`；启动时 `version` 握手
- [ ] 收件箱：点开结论显示证据原文并高亮 `highlight` 区间；“有用 / 不重要”、“完成 / 忽略”按钮
- [ ] “开始汇总”按钮 + 进度条；“标为已读”按钮（回传 `view_cursor`）
- [ ] 决策日志面板、运行统计面板（消息数、话题数、被拒结论数、token、费用）

**同学 C**
- [ ] 完成 200 条标注；与另一位同学完成 50 条双人标注
- [ ] `chat-tldr-eval score`（切分、抽取、排序、幻觉指标）+ 单元测试（手算的小例子）
- [ ] README 与报告框架

**第 2 天 20:00 联调**（全员）：用 C 的真实导出跑通 import → analyze → GUI，记录问题清单，分配修复。

## 第 3 天

- [ ] **@Develata**：修联调问题；`--strategy b0 / b1 / sim-tfidf`；`--html` 模板；12:00 功能冻结
- [ ] **同学 A**：修 bug；补充 temporal、verify 的边界用例；协助 C 跑实验
- [ ] **同学 B**：修 bug；GUI 打磨；准备演示用的数据目录
- [ ] **同学 C**：跑全部系统（EVALUATION §6）、`calibrate`、汇总表；报告；录演示视频（GUI 为主，`--html` 为备用）
- [ ] 全员：报告的“局限性”部分（脱敏只替换已知别名、Jev 中文精度、样本规模小）

## 砍需求顺序（落后时从上往下砍）

1. `sim-embed` 基线与 embedding 粗筛（默认本来就不启用）
2. `MergeTopics`（规则永远不提议，枚举保留）
3. LLM 复核档（`tau_low ≤ confidence < tau_high` 改为直接归入最高项）
4. 反馈个性化（按钮保留，只记录不生效）
5. B1 基线
6. GUI 决策日志面板（改用 `chat-tldr decisions` 的输出截图）

**绝不砍**：证据校验、幂等导入、@我 规则、JSONL 协议、Ours vs B0 对比、Jev 校准曲线。

## 降级预案

| 故障 | 预案 |
|---|---|
| Jev 不可用 | `--decider llm`（自动降级 + `W_DECIDER_FALLBACK`）；报告中用 Ours-LLM 的数字 |
| 话题切分失败 | `--strategy b1`：时间间隔 + 固定切块 |
| 截止日期规范化不确定 | 只保留 `raw` |
| GUI 出问题 | `chat-tldr inbox --chat <ID> --html demo.html`，用浏览器演示 |
| 真实测试群数据来不及 | 用合成集完成全部流程，报告中如实注明 |

## 非目标

- 内嵌 QCE 或依赖 NapCat 实时抓取；微信支持；实时机器人
- 向量数据库、RAG、Agent 框架（如 Rig、LangChain）、模型微调、训练话题切分模型
- 纯本地模式（Jev 是云服务）
- 完整的中文时间解析库（只做高频规则）
- QCE 的 chunked-JSONL 导出格式
- 多用户、多设备同步

## 未来工作

- 支持 QCE chunked-JSONL 与更多导出工具
- 本地 embedding（fastembed-rs）与本地 LLM（Ollama），减少云端依赖
- 更完整的时间表达式解析（农历、节假日、“月底”“学期末”）
- 基于正文的人名识别，提升脱敏覆盖率
- 并行执行 AnalyzeTopic
- 用积累的反馈与标注做离线阈值搜索
