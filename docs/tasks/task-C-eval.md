# 评估规格与人工验收材料协作

> 原任务 C 的完整评估目标，保留作为规划参考；下方未完成目标不是当前已支持的命令或实测结果。
> 当前安排：Codex 负责评估工具编码，同学的数据、标注与报告任务另行分配。审核：@Develata。

**当前状态（2026-09-26）**：`check-stream`、标注 CSV 往返和 Ours 部分离线指标已实现，eval 当前 44 项测试通过；共用 100 条主样本与 4 条回填样本已提交。独立人工 gold、双人标注、真实效果报告、完整基线和校准仍待完成。README 由 Codex 同步维护，不计作同学 C 的报告交付。当前任务入口见 [TEAM_ASSIGNMENTS](../TEAM_ASSIGNMENTS.md) 和 [eval/synthetic](../../eval/synthetic/README.md)。

源码、合成数据、真实数据、实验运行和报告的存放位置见 [FILE_LAYOUT.md](../FILE_LAYOUT.md) 与 [eval/README.md](../../eval/README.md)。

## 目标

1. 准备评估数据：一个征得同意的真实测试群（约 200 条消息，只存本地），加一份可以提交的合成数据。
2. 按标注规范完成标注，其中 50 条两人独立标注。
3. 实现 `chat-tldr-eval`：标注表格的导入导出、指标计算、校准曲线、汇总表。
4. 跑完所有系统和基线的实验，产出报告需要的数字和图。
5. README 的完善、报告框架、演示视频。

## 当前修改范围

- 同学 C：`eval/synthetic/`、`reports/`；共用 `fixtures/` 修改先协调。
- Codex：`eval/src/`、`eval/tests/` 和 README；需要同学参与源码时另行明确文件。
- 真实导出、标注和运行数据只保存在被忽略的 `eval/private/`。

`eval` 只依赖 `crates/core`，通过运行 `chat-tldr` 子进程拿数据，不直接读数据库。

## 输入输出

- 评估方案与标注规范：[EVALUATION.md](../EVALUATION.md)（**先完整读一遍**）
- 协议：[CLI_PROTOCOL.md](../CLI_PROTOCOL.md)（`insight`、`topic`、`stats`、`jev_answer` 事件）

`chat-tldr-eval` 当前命令与规划：

| 子命令 | 状态 | 输入与输出 |
|---|---|---|
| `check-stream <FILE> --exit-code <CODE>` | 已实现 | 校验 JSONL 流与真实退出码，不产生效果评分 |
| `export-sheet --messages <messages.jsonl> --out sheet.csv` | 已实现 | 成功完整消息流 → 含空标注列的 CSV |
| `import-sheet sheet.csv --out gold/` | 已实现 | 人工填写 CSV → `gold/messages.jsonl`、`gold/items.jsonl`，不覆盖已有目录 |
| `score --gold <DIR> --run <DIR> --out <CSV>` | 部分指标已实现 | Ours 抽取/截止/排序、保存快照 rejected 比例和单次用量；缺失与未实现指标明确标记 |
| `agreement --a <DIR> --b <DIR>` | 未实现 | 规划：两人标注的一致性 |
| `calibrate --gold <DIR> --jev <jev.jsonl> --out <PNG>` | 未实现 | 规划：可靠性曲线、ECE、Brier |
| `summarize <CSV>...` | 未实现 | 规划：跨系统汇总表 |

`score` 读取同一实验的 `messages.jsonl`、`analyze.jsonl`、`inbox.jsonl`，完整参数和统计边界见 [eval/README](../../eval/README.md)。话题/切分指标尚未实现；当前快照 rejected 比例不能当作所有原始提案或人工语义的幻觉率。未知子命令会报错。

## 验收标准

- [ ] 测试群的同意记录（聊天截图或书面说明，只存本地，报告中注明“已征得同意”）
- [ ] 200 条标注 + 50 条双人标注，一致率已计算
- [ ] 每个指标函数都有手算的小例子作为单元测试（例如 4 条消息、2 个话题的 1-to-1 overlap）
- [ ] 至少跑完 B0、B1、Ours、Ours-LLM、Sim-TFIDF 五个系统，结果表填好（EVALUATION §7）
- [ ] Jev 校准曲线（Jev 与 LlmDecider 各一张），附 ECE
- [ ] 增量实验的数字（EVALUATION §3.6）
- [ ] 合成数据集：至少 1 份含交织话题、@、回复、截止日期、撤回、合并转发的 QCE 格式 JSON，附带标注，放在 `eval/synthetic/`
- [ ] README 中文和英文部分都已更新；报告框架（背景、方法、实验、结果、局限性、分工）
- [ ] 3 分钟以内的演示视频（GUI 为主，`--html` 作为备用）

## 参考资料

- Elsner & Charniak, “You Talking to Me? A Corpus and Algorithm for Conversation Disentanglement”, ACL 2008（1-to-1 overlap）
- Kummerfeld et al., “A Large-Scale Corpus for Conversation Disentanglement”, ACL 2019（exact-match F1 等指标）
- NDCG：Järvelin & Kekäläinen, “Cumulated Gain-Based Evaluation of IR Techniques”, ACM TOIS 2002
- 校准：Guo et al., “On Calibration of Modern Neural Networks”, ICML 2017（ECE、可靠性曲线）
- 公开数据集（仅作参考）：VCSum、CSDS
- Rust crates：`csv`、`serde_json`、`plotters`（画图）

## 需要避免的坑

- **绝不提交真实聊天记录**：真实数据、标注和包含原文的运行结果都放在 `eval/private/`（已被 `.gitignore` 忽略）。提交前运行 `git status` 检查。
- 合成数据要避免循环偏差：如果用 LLM 生成，报告中注明使用的模型，并且尽量手工修改。
- 每个系统用独立的 `--data-dir`，否则缓存和话题状态会互相影响。
- 标注时不要先看系统输出，避免被系统“带偏”。
- 指标定义以原论文为准，写报告前核对一遍（Q-EV-1）。
