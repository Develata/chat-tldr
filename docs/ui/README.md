# GUI 设计交付目录

同学 B 从这里开始，具体分工与验收见 [TEAM_ASSIGNMENTS](../TEAM_ASSIGNMENTS.md)。图稿只使用 `fixtures/jsonl/` 中的合成内容，并附 Markdown 交互说明。

当前主线 GUI 已由 Codex 实现；本目录尚无独立图稿或设计复核交付。先运行 [现有 GUI](../../apps/gui/README.md)，围绕实际界面提出改进，不需要重新搭建整个 GUI。新增总览尚待原生截图验收，已有收件箱画面见 [GUI_VERIFICATION](../GUI_VERIFICATION.md)。

首先覆盖：空数据、选群/导入、分析进度、部分失败、P0–P3 收件箱、证据展开、事项完成/恢复，以及 `view_cursor=null` 时不可标为已读。把界面动作映射到 [CLI_PROTOCOL](../CLI_PROTOCOL.md)，运行能力通过 `version.capabilities` 发现。

补充六个总览的入口与近 6/24 小时、7 天窗口：热闹但不紧急的话题与安静但有截止日期的 P0 应清楚区分；展示负责人、待分析状态与时间待确认。总览不能标为已读，不能因切换窗口而隐藏旧的未完成 P0。可复用 [多场景合成样本](../../fixtures/qce/scenario-analysis.README.md)，不要把计划中的更正/取消或待回应能力画成已实现结果。

实际使用的字体与图标放 `apps/gui/assets/` 并随附许可；本目录放设计材料。GUI 不读写 SQLite，不依赖 engine，不把尚未实现的 CLI 命令当作可用功能。
