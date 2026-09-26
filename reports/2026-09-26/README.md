# 首版分析与发布验收

真实样本尚无人工 gold：**质量评分和真实 Jev 校准曲线均为“待标注”**。不以 LLM 标注替代人工，不把引用存在当作语义正确。

## 真实 200 条：运行与成本

同一 QCE 单文件分别导入两份新数据库，不共享缓存。DeepSeek `deepseek-flash`、temperature=0、thinking 关闭；Ours 使用 Jev `jev-1.13.0`。单次预算 $0.50、最多 256 步。聚合收据见 [cloud-aggregate.json](cloud-aggregate.json)；原始 JSONL、数据库、未标注 CSV 留在 private/。

| 记录 | Ours | B0 |
|---|---:|---:|
| 分析退出码 | 0 | 0 |
| 完成消息 | 200 | 200 |
| 逻辑模型调用 | 128 | 1 |
| 输入 token | 131,882 | 17,194 |
| 输出 token | 22,375 | 4,308 |
| 估算 USD | 0.040238394 | 0.0103278 |
| 分析耗时 ms | 110,468 | 12,813 |
| 保存结论：verified / unverified / rejected | 54 / 4 / 4 | 21 / 1 / 4 |
| 关系抽取覆盖消息 | 200 | 200 |
| 质量评分 | 待标注 | 待标注 |

Ours 有 69 次 TypeSafe、59 次 DeepSeek 调用。控制器 method 为 rule 32、jev 15、fallback 13；一次 `W_DECIDER_FALLBACK` 后其余决策改走 LLM，4 条结论被引用校验拒绝。B0 不调用 Jev，保留被拒结论供评分。结论数量及 rejected 数不能代替准确率/召回率。

calls 是逻辑调用，不等于 HTTP 重试次数；费用按已报告用量和配置费率估算，不是账单。上表只对应最终这两次独立运行，不含此前调试/失败尝试。耗时为单次观测，不是性能基准。收据以 debug CLI 的 SHA-256 标识构建；当时源码在尚未提交的发布工作树，不能冒充已发布二进制的云验收。

真实联调修复了预算预留不结算、Jev 两位小数概率总和、关系条件 schema 和 B0 输出截断。失败记录保留在 private/，未混入成功结果。

## 合成工具验证

[Ours/B0 表](synthetic/README.md)、[Ours CSV](synthetic/ours.csv)、[B0 CSV](synthetic/b0.csv)、[校准 JSON](synthetic/calibration/calibration.json)、[校准 SVG](synthetic/calibration/reliability.svg) 来自实际 CLI+eval 流程、本地固定响应及按合成源文本明确编写的 gold。

Todo/Announcement 各 2 个样本，ECE=0.25、binary Brier=0.0625，与手算一致。曲线重合源于相同的概率/标签设计。此样本只检验映射、公式和绘图，不描述 Jev 真实校准性；更多合成反例由 Rust 回归覆盖。

## 待同学 C 交付

按 EVALUATION §5 标注 200 条真实 CSV，其中 50 条由第二人独立标注/复核。保留源列，填写 thread/todo/announcement/items_json，通过 `import-sheet` 后在同一 profile 快照上运行 score/calibrate。choice 另需 request_key/question_id/正确选项的人工标签。缺项继续写“待标注”。
