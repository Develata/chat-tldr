# 合成流程验收（非真实模型效果）

仅两个未撤回消息：报告通知与确认收到。gold 在脚本中按源文本明确编写；模型为本地固定回答。

| 指标 | Ours | B0 |
|---|---:|---:|
| thread_one_to_one | 1 | 1 |
| thread_exact_f1 | 1 | 1 |
| ari | 1 | 1 |
| nmi | 1 | 1 |
| todo_f1 | 1 | 1 |
| announcement_recall | 0 | 0 |
| ndcg_at_5 | 0.6131471927654584 | 0.6131471927654584 |
| snapshot_rejected_rate | 0 | 0 |
| run_calls | 2 | 1 |

synthetic-jev 的 Todo/Announcement 各 2 个样本；手算 ECE=0.25，Brier=0.0625。曲线仅验证概率到标注的映射和计算。

源文件 SHA-256：`fe06359b593e47a3bd145dead537035440d013e4ea648f81851c6665d870113c`。

真实 200 条消息的质量评分与 Jev 校准曲线：**待标注**。
