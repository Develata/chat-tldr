# 原生 GUI 使用说明

GUI 通过 CLI 子进程接入主线功能，已提供个人收件箱与六个分析总览。同学 B 补充了界面整理、运行详情和 [合成截图](../../docs/ui/README.md)。自动化覆盖原生窗口 smoke、CLI 联调、错误传播、统计刷新及 release 构建/解包；平台边界见 [发布说明](../../docs/RELEASING.md#gui-自动化验收)。真实聊天与云模型质量尚未验收。

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

Windows GUI 发布包须完整解压，保留两份 EXE 的同目录关系。自动截图可使用隐藏的 `--screenshot <PNG> --quit-after-capture`；CI 额外传 `--smoke-report <JSON>` 保存状态回执，只有截图成功写入后才发布回执。该模式不会自动导入或分析，测试脚本只准备独立临时目录和合成数据。

分析失败或部分完成后的自动刷新保留错误提示，直到主动重试分析/导入或关闭提示。运行详情分别显示“最近查询结果”和“最近一次分析”，累计统计不会被上一次运行遮住。

使用真实导出前，先通过 CLI 准备配置和自己的 QQ 身份：

```powershell
.\target\debug\chat-tldr.exe --data-dir .\private\my-chat config init
.\target\debug\chat-tldr.exe --data-dir .\private\my-chat import "C:\path\group.json" --self-uin "<你的QQ号>"
```

`--self-uin` 是你本人的 QQ 号；已知 QQNT UID 时可额外使用 `--self-uid`。`config init` 不覆盖已有配置。重复导入保留已存身份；身份冲突会报错，不会悄悄切换用户。首次导入缺身份会影响个人 @ 识别，@全体成员仍成立。

GUI 的“导入”只选择已完成且已关闭的单文件 QCE JSON，交给 CLI 导入；不提供密钥或身份编辑入口。导出步骤见 [QCE Docker 说明](../../docs/QCE_DOCKER_EXPORT.md)。API key 只从启动进程的环境变量继承，应在启动 GUI 前设置；不写入配置文件或 `gui-state.json`。CLI `doctor` 仅检查本地配置、路径和环境变量存在性，不测试网络连通性。

首次云端分析需确认：程序和数据库在本地，但聊天原文会发送给配置的云服务（默认 Jev 与 DeepSeek），不做脱敏，图片不上传。打开界面、导入、历史查询、收件箱和总览查询不调用模型。`analyze` 即使没有新消息，也可能继续尚未完成的话题合并并请求模型；是否有工作可先用 CLI 的 `--dry-run` 查看。

选群后点击“分析总览”，可切换热门话题、优先话题、与我有关、截止事项、未读回顾、资料入口，并选择近 6 小时、24 小时或 7 天。热门榜仅统计已完成分析的窗口消息；优先事项保留窗口前仍未完成的 P0；其他人负责的截止事项同样会进入 P0，并显示实际负责人。原文提及和资料可以显示尚未分析的消息，不代表模型已理解其含义。资料入口只展示链接/附件元数据，不下载或读取正文。

总览只读，不授予“标为已读”权限。需要标读时返回收件箱，按下方的完整展示规则操作。刷新失败不会把部分总览当成功结果；完整统计口径见 [ANALYSIS_VIEWS](../../docs/ANALYSIS_VIEWS.md)。

GUI 只依赖 core 协议，通过子进程调用 CLI；不打开 SQLite。界面偏好保存在 `<data-dir>/gui-state.json`，业务状态由 CLI/engine 保存。主线由 Codex 接入，同学 B 继续负责 `docs/ui/` 的交互设计；A 负责 QCE 管理，C 负责合成场景与验收材料，见 [当前分工](../../docs/TEAM_ASSIGNMENTS.md)。

切换数据目录时，GUI 先保存新目录的偏好，成功后再更新启动目录的指向，避免失败切换留下循环引用。默认启动会沿已选择的目录读取最新设置；显式 `--data-dir` 优先，适合隔离多个数据集。路径保存为绝对路径，设置不含密钥。损坏或循环引用会报错，不静默重置并覆盖原文件。改变 CLI、数据目录或配置路径后，下一次分析会重新提示云端发送；待确认窗口绑定所显示的群聊，切换群聊或连接设置会取消该次确认。

宽窗口显示三栏，窄窗口用 P0/P1/P2+P3 标签页；每栏每页最多 20 项，切页保留同一收件箱快照。负责人始终单独标注，其他人负责但有截止日期的事项仍可进入 P0。原文高亮按 Unicode 字符索引处理，兼容中文与 emoji。日志及各类历史记录各保留最近 200 条，完整记录可用 CLI 导出。

“停止”会终止本 GUI 启动的 CLI 进程，不冒充优雅的 Ctrl-C；随后重新查询已持久化的群列表和收件箱，已提交检查点保留。关闭 GUI 会等待其直接 CLI 子进程终止并完成已请求的偏好保存。GUI 只管理直接子进程；自定义包装程序创建的后代进程不在管理范围内。取消时，直接子进程回收后停止等待后代持有的输出管道，旧请求事件不会成为新请求的结果。正常运行仍须完整读完并校验输出。只有成功、完整且所有结论已经显示的收件箱才提供“标为已读”的游标；尚未打开的标签页、未翻到的页面和被滚动区域裁切的内容不会计为已展示，失败或未完成的刷新会撤销旧游标授权。
