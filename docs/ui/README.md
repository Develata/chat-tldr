# GUI 设计交付目录

同学 B 从这里开始，具体分工与验收见 [TEAM_ASSIGNMENTS](../TEAM_ASSIGNMENTS.md)。图稿只使用 `fixtures/jsonl/` 中的合成内容，并附 Markdown 交互说明。

当前主线 GUI 已由 Codex 实现；本目录记录本轮复核、问题清单和可复现的原生截图。先运行 [现有 GUI](../../apps/gui/README.md)，围绕实际界面提出改进，不需要重新搭建整个 GUI。历史验收材料仍见 [GUI_VERIFICATION](../GUI_VERIFICATION.md)。

## 本轮交付

- [界面问题清单](feedback.md)：每条包含“哪里 / 现象 / 建议”，并标注优先级与本轮处理范围。
- [收件箱](inbox.png)：P0–P3 分栏、阅读进度和合成数据提示。
- [证据高亮](evidence-highlight.png)：证据展开、原文引用和 Unicode 安全高亮。
- [决策日志](decision-log.png)：规则、Jev、降级三步决策及候选动作。
- [运行统计](run-stats.png)：KPI、Token/费用和按阶段模型用量。
- [分析总览](analysis-overview.png)：六个页签、负责人、截止时间和引用入口。
- [窄窗口收件箱](narrow-inbox.png)：窄宽度下改用当前群组选择器，侧栏不会挤压内容。
- [窄窗口统计](narrow-stats.png)：统计卡片在紧凑宽度下自动改为两列，表格可横向滚动。

截图只使用 `--demo` 的固定合成数据，不读取数据库，也不调用云服务。需要重新生成时，在仓库根目录先执行 `cargo build -p chat-tldr-gui --locked`，再运行：

```powershell
target\debug\chat-tldr-gui.exe --demo --demo-view inbox --screenshot docs/ui/inbox.png --quit-after-capture --width 1440 --height 900
target\debug\chat-tldr-gui.exe --demo --demo-view evidence --screenshot docs/ui/evidence-highlight.png --quit-after-capture --width 1440 --height 900
target\debug\chat-tldr-gui.exe --demo --demo-view decisions --screenshot docs/ui/decision-log.png --quit-after-capture --width 1440 --height 1000
target\debug\chat-tldr-gui.exe --demo --demo-view stats --screenshot docs/ui/run-stats.png --quit-after-capture --width 1440 --height 1000
target\debug\chat-tldr-gui.exe --demo --demo-view overview --screenshot docs/ui/analysis-overview.png --quit-after-capture --width 1440 --height 1000
```

首先覆盖：空数据、选群/导入、分析进度、部分失败、P0–P3 收件箱、证据展开、事项完成/恢复，以及 `view_cursor=null` 时不可标为已读。把界面动作映射到 [CLI_PROTOCOL](../CLI_PROTOCOL.md)，运行能力通过 `version.capabilities` 发现。

补充六个总览的入口与近 6/24 小时、7 天窗口：热闹但不紧急的话题与安静但有截止日期的 P0 应清楚区分；展示负责人、待分析状态与时间待确认。总览不能标为已读，不能因切换窗口而隐藏旧的未完成 P0。可复用 [多场景合成样本](../../fixtures/qce/scenario-analysis.README.md)，不要把计划中的更正/取消或待回应能力画成已实现结果。

实际使用的字体与图标放 `apps/gui/assets/` 并随附许可；本目录放设计材料。GUI 不读写 SQLite，不依赖 engine，不把尚未实现的 CLI 命令当作可用功能。

## QQ 获取向导（Codex 接入增量）

- [连接、最近群聊和时间范围](qce-acquisition.png)
- [深色窄窗口扫码](qce-login.png)：二维码为 `example.invalid` 合成内容，不能用于登录。

这两张来自 `scripts/release/gui_smoke.py` 驱动本机假 QCE/NapCat 与真实 GUI/manager/CLI 的原生截图，使用临时数据目录，无真实账号、聊天或云调用；不是 B 的原始交付，也不等于手机扫码验收。复现与结果边界见 [RELEASING](../RELEASING.md#gui-自动化验收)。
