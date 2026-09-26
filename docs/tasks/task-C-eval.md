# 任务 C：评估、演示数据、README 与报告

> 用途：同学 C 的任务 issue 正文。可以直接复制到 GitHub issue。
> 负责人：同学 C。审核：@Develata。

## 目标

1. 准备评估数据：一个征得同意的真实测试群（约 200 条消息，只存本地），加一份可以提交的合成数据。
2. 按标注规范完成标注，其中 50 条两人独立标注。
3. 实现 `chat-tldr-eval`：标注表格的导入导出、指标计算、校准曲线、汇总表。
4. 跑完所有系统和基线的实验，产出报告需要的数字和图。
5. README 的完善、报告框架、演示视频。

## 你可以修改的目录

- `eval/`
- `README.md`（与 @Develata 共同负责）
- `fixtures/` 下的合成数据（新增文件即可，不要改已有文件）

`eval` 只依赖 `crates/core`，通过运行 `chat-tldr` 子进程拿数据，不直接读数据库。

## 输入输出

- 评估方案与标注规范：[EVALUATION.md](../EVALUATION.md)（**先完整读一遍**）
- 协议：[CLI_PROTOCOL.md](../CLI_PROTOCOL.md)（`insight`、`topic`、`stats`、`jev_answer` 事件）

`chat-tldr-eval` 的子命令：

| 子命令 | 输入 | 输出 |
|---|---|---|
| `export-sheet --messages <messages.jsonl> --out sheet.csv` | `chat-tldr messages` 的输出 | CSV：message_id、时间、发送者代号、文本、以及空的标注列 |
| `import-sheet sheet.csv --out gold/` | 填好的 CSV | `gold/messages.jsonl`、`gold/items.jsonl`（格式见 EVALUATION §5.1） |
| `score --gold <DIR> --run <DIR> --out <CSV>` | 标注 + 系统输出 | 切分、抽取、排序、幻觉指标 |
| `agreement --a <DIR> --b <DIR>` | 两人的标注 | κ、1-to-1、item F1 |
| `calibrate --gold <DIR> --jev <jev.jsonl> --out <PNG>` | 标注 + Jev 回答 | 可靠性曲线 PNG、ECE、Brier |
| `summarize <CSV>...` | 各系统结果 | Markdown 汇总表 |

`score` 从 `messages.jsonl` 读取每条消息的预测话题（切分指标），从 `inbox.jsonl` 的 `insight` 事件读取结论（抽取、排序、幻觉指标）。

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
