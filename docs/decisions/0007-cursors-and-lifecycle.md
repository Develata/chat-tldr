# ADR-0007 三游标与结论生命周期

**背景**：最初的设计只有 `last_ingested` 和 `last_reviewed` 两个游标。但话题状态是在线维护的，必须随“已经分析到哪里”推进；否则两次 analyze 之间如果用户没有标记已读，就只能从头重新聚类，结果还可能变化。另外，如果收件箱只是“上次查看以来”的窗口，标记已读后，还没完成的待办会从收件箱消失。

**决策**
- 三个游标：`last_ingested`、`last_analyzed`、`last_reviewed`。话题状态随 `last_analyzed` 增量推进；“自上次查看以来”只是收件箱的展示窗口。
- `last_reviewed` 只由 `mark-read --up-to <view_cursor>` 推进，且 `view_cursor` 必须来自 GUI 实际显示过的 `inbox` 事件。
- 结论带 `lifecycle`（open / done / dismissed），由 `resolve` 命令修改。P0 一直显示，直到被处理；P1 在截止日期之前一直显示；其余结论按展示窗口显示。
- 重新分析一个话题时，LLM 能看到它已有的 open 结论，可以更新而不是重复创建。

**理由**
- 直接体现“持续维护状态”和“行动收件箱”的立意。
- 回填、崩溃恢复、重复运行都有明确定义：每条消息有 `analysis_state`（pending / done / skipped / failed）；待切分消息 = 没有话题归属的 pending 消息；脏话题 = 含有 pending 消息的话题；撤回消息在导入时即为 skipped，不会卡住 `last_analyzed`。

**代价**
- 多一个命令（`resolve`）和一个字段（`lifecycle`），GUI 多两个按钮。
