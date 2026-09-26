# GUI 设计交付目录

同学 B 从这里开始，具体分工与验收见 [TEAM_ASSIGNMENTS](../TEAM_ASSIGNMENTS.md)。图稿只使用 `fixtures/jsonl/` 中的合成内容，并附 Markdown 交互说明。

首先覆盖：空数据、选群/导入、分析进度、部分失败、P0–P3 收件箱、证据展开、事项完成/恢复，以及 `view_cursor=null` 时不可标为已读。把界面动作映射到 [CLI_PROTOCOL](../CLI_PROTOCOL.md)，运行能力通过 `version.capabilities` 发现。

实际使用的字体与图标放 `apps/gui/assets/` 并随附许可；本目录放设计材料。GUI 不读写 SQLite，不依赖 engine，不把尚未实现的 CLI 命令当作可用功能。
