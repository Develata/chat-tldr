# 原生 GUI 使用说明

GUI 通过 CLI 子进程接入主线功能。Windows 原生截图、合成 QCE 导入后的 CLI→GUI 查询联调、egui 指针交互测试已通过；真实聊天与云模型质量尚未验收。范围与证据见 [GUI 验证记录](../../docs/GUI_VERIFICATION.md)。

在仓库根目录的 PowerShell 中运行：

```powershell
cargo build --workspace
.\target\debug\chat-tldr-gui.exe --demo
.\target\debug\chat-tldr-gui.exe --cli .\target\debug\chat-tldr.exe --data-dir .\private\my-chat
```

- `--demo`：只显示合成内容，不启动 CLI、不保存 GUI 偏好。
- `--cli <FILE>`：指定主 CLI；默认寻找 GUI 可执行文件同目录的 `chat-tldr.exe`。
- `--data-dir <DIR>`：与 CLI 共用的数据目录；Windows 默认 `%APPDATA%\chat-tldr`。
- `--config <FILE>`：指定业务配置；省略时使用数据目录内 `config.toml`。
- `--dark`：使用深色外观。

使用真实导出前，先通过 CLI 准备配置和自己的 QQ 身份：

```powershell
.\target\debug\chat-tldr.exe --data-dir .\private\my-chat config init
.\target\debug\chat-tldr.exe --data-dir .\private\my-chat import "C:\path\group.json" --self-uin "<你的QQ号>"
```

`--self-uin` 是你本人的 QQ 号；已知 QQNT UID 时可额外使用 `--self-uid`。`config init` 不覆盖已有配置。重复导入保留已存身份；身份冲突会报错，不会悄悄切换用户。首次导入缺身份会影响个人 @ 识别，@全体成员仍成立。

GUI 的“导入”只选择已完成且已关闭的单文件 QCE JSON，交给 CLI 导入；不提供密钥或身份编辑入口。导出步骤见 [QCE Docker 说明](../../docs/QCE_DOCKER_EXPORT.md)。API key 只从启动进程的环境变量继承，应在启动 GUI 前设置；不写入配置文件或 `gui-state.json`。CLI `doctor` 仅检查本地配置、路径和环境变量存在性，不测试网络连通性。

首次云端分析需确认：程序和数据库在本地，但聊天原文会发送给配置的云服务（默认 Jev 与 DeepSeek），不做脱敏，图片不上传。打开界面、导入、历史查询和收件箱查询不调用模型。没有待处理消息时分析也不调用模型。

GUI 只依赖 core 协议，通过子进程调用 CLI；不打开 SQLite。界面偏好保存在 `<data-dir>/gui-state.json`，业务状态由 CLI/engine 保存。主线由 Codex 接入，同学 B 继续负责 `docs/ui/` 的交互设计；A 负责 QCE 管理，C 负责合成场景与验收材料，见 [当前分工](../../docs/TEAM_ASSIGNMENTS.md)。

切换数据目录时，GUI 同时更新启动目录的偏好和新目录的偏好。默认启动会沿已选择的目录读取最新设置；显式 `--data-dir` 优先，适合隔离多个数据集。路径保存为绝对路径，设置不含密钥。损坏或循环引用会报错，不静默重置并覆盖原文件。

宽窗口显示三栏，窄窗口用 P0/P1/P2+P3 标签页；每栏每页最多 20 项，切页保留同一收件箱快照。负责人始终单独标注，其他人负责但有截止日期的事项仍可进入 P0。原文高亮按 Unicode 字符索引处理，兼容中文与 emoji。日志及各类历史记录各保留最近 200 条，完整记录可用 CLI 导出。

“停止”会终止本 GUI 启动的 CLI 进程，不冒充优雅的 Ctrl-C；随后重新查询已持久化的群列表和收件箱，已提交检查点保留。只有成功、完整且已经显示的收件箱才提供“标为已读”的游标，失败或未完成流不会授权旧游标。
