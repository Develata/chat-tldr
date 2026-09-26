# 任务 B：egui 图形界面

> 用途：同学 B 的任务 issue 正文。可以直接复制到 GitHub issue。
> 负责人：同学 B。审核：@Develata。

## 目标

做一个桌面面板：用户点“开始汇总”，看到进度；汇总完成后看到三栏收件箱（需要你处理 / 值得知道 / 其他话题）；点开任意结论能看到原始消息，引用片段被高亮；可以反馈、标记完成、标为已读；可以查看本次智能体的决策和运行统计。

## 你可以修改的目录

- `apps/gui/`

**GUI 只依赖 `crates/core`**，永远不直接读写数据库，也不依赖 `engine` / `qce`。所有数据都来自运行 `chat-tldr` 子进程后 stdout 上的 JSON Lines。

## 输入输出

- 协议：[CLI_PROTOCOL.md](../CLI_PROTOCOL.md)（事件列表 §3、版本规则 §6）
- 类型：`chat_tldr_core::CliEvent`（[DATA_MODEL.md](../DATA_MODEL.md) §4）
- 开发初期使用 `fixtures/jsonl/*.jsonl` 的 mock 数据，不需要等 CLI 完成。

GUI 会调用的命令：

| 界面操作 | 命令 |
|---|---|
| 启动 | `chat-tldr version`（检查 MAJOR 版本）→ `chat-tldr chats` |
| 选择群 | `chat-tldr inbox --chat <ID>` |
| 开始汇总 | `chat-tldr analyze --chat <ID>`，结束后自动刷新 `inbox` |
| 有用 / 不重要 | `chat-tldr feedback <INSIGHT_ID> --useful` / `--not-important` |
| 完成 / 忽略 | `chat-tldr resolve <INSIGHT_ID> --done` / `--dismiss` |
| 标为已读 | `chat-tldr mark-read --chat <ID> --up-to <view_cursor>`，其中 `view_cursor` 取自**最近一次** `inbox` 事件 |
| 查看决策 | `analyze` 过程中收到的 `decision` 事件；历史运行用 `chat-tldr decisions --run <RUN_ID>` |
| 导入文件 | 文件选择对话框 → `chat-tldr import <PATH>` |

## 实现要求

1. **从 eframe_template 起步**（github.com/emilk/eframe_template）。
2. **中文字体**：通过 `egui::FontDefinitions` 加载打包进程序的中文字体（`include_bytes!`），放在 proportional 和 monospace 字体族的首位。字体必须是 OFL 等允许再分发的许可证，许可证文件一起放在 `apps/gui/assets/fonts/`。
3. **子进程与线程**：
   - 后台线程用 `std::process::Command` 启动 CLI，`stdout(Stdio::piped())`、`stderr(Stdio::piped())`；
   - 用 `BufReader::lines()` 逐行读取 stdout，`serde_json::from_str::<CliEvent>` 解析，然后通过 `std::sync::mpsc::Sender` 发给 UI 线程；
   - stderr 用另一个线程读取，只用于“日志”面板；
   - UI 线程在 `update()` 中循环 `try_recv()` 直到队列为空，收到事件后调用 `ctx.request_repaint()`；
   - **UI 线程绝不阻塞**：不能在 `update()` 里调用 `wait()`、`read_line()` 或任何网络、文件操作。
4. **协议健壮性**：未知的 `event`、无法解析的行 → 写入日志面板，跳过，不崩溃。以 `done` 事件判断命令结束；进程退出但没有收到 `done` → 显示“CLI 异常退出（退出码 X）”。
5. **版本握手**：`version` 返回的 `schema_version` 中，MAJOR 与编译时的常量不一致 → 显示“CLI 协议版本不兼容，请更新”，并禁用所有操作。
6. **CLI 路径**：默认取与 GUI 可执行文件同目录下的 `chat-tldr`（Windows 上为 `chat-tldr.exe`），可在设置中修改。

## 布局

```
┌─────────────────────────────────────────────────────────────────────┐
│ [导入文件] [开始汇总] ▓▓▓▓▓▓░░░ segment 120/412     [标为已读] [设置] │
├────────────┬────────────────────────────────────────────────────────┤
│ 群列表      │  需要你处理 (P0)   │  值得知道 (P1)   │  其他话题 (P2/P3) │
│ ● 课程群 2  │  ┌──────────────┐ │                 │  ▸ 闲聊 (4)       │
│   社团群    │  │周五前交实验报告│ │                 │                   │
│            │  │截止 9/25 周五  │ │                 │                   │
│ ────────── │  │[有用][不重要]  │ │                 │                   │
│ 本次运行    │  │[完成][忽略]    │ │                 │                   │
│ 消息 412    │  └──────────────┘ │                 │                   │
│ 话题 7      │  ▾ 证据                                                 │
│ 被拒结论 1  │    班长-小王 21:05  @全体成员 【周五前把实验报告交到…】   │
│ token/费用  │                                                        │
│ [决策日志]  │                                                        │
└────────────┴────────────────────────────────────────────────────────┘
```

- 结论卡片显示：标题、摘要、截止日期（显示 `raw`，有 `bound_date` 时附上日期）、`verification_status = unverified` 时显示“未验证”标记、P0 已过期时显示“已过期”。
- 点开证据：显示 `EvidenceView.display_text`，把 `highlight` 区间（**字符下标**）高亮。`highlight` 为 `null` 时只显示原文。高亮用 `egui::text::LayoutJob` 分段设置颜色。
- 决策日志：表格，列出 step、候选动作、选择的动作、method（rule / jev / fallback）、概率、理由。
- 运行统计：来自 `stats` 事件（消息数、话题数、被拒结论数、token、估算费用、耗时）。
- 首次运行时弹窗说明：程序在本机运行，但聊天文本会发送给 Jev 和 DeepSeek 两个云服务（不做脱敏，图片不上传）。

## 验收标准

- [ ] 只依赖 `chat-tldr-core`（`cargo tree -p chat-tldr-gui` 中没有 rusqlite、reqwest）
- [ ] 中文正常显示，没有方块
- [ ] 用 `fixtures/jsonl/*.jsonl` 能完整展示三栏收件箱、证据高亮、决策日志、统计
- [ ] 真实 CLI 运行 `analyze` 时进度条实时更新，窗口可以拖动、滚动，不卡顿
- [ ] 按钮操作后界面刷新（feedback、resolve、mark-read）
- [ ] 协议 MAJOR 版本不兼容时拒绝运行
- [ ] 至少有“JSONL 行 → 界面状态”这一层的单元测试（把解析和状态更新写成纯函数，不依赖 egui 就能测试）

## 参考资料

- eframe_template：github.com/emilk/eframe_template
- egui demo：egui.rs（左侧菜单里有各个控件的示例和源码链接）
- egui 字体：`egui::FontDefinitions`、`FontData::from_static`
- 子进程：Rust 标准库 `std::process::Command`、`std::sync::mpsc`

## 需要避免的坑

- 在 UI 线程里读子进程输出 → 界面冻结。一定要放在后台线程。
- 忘记 `request_repaint()` → 事件到了，但界面要等到鼠标移动才刷新。
- 按字节下标切中文字符串 → panic。`highlight` 是字符下标，先用 `char_indices()` 转成字节下标再切。
- 解析 stderr → 不要这样做，stderr 的格式随时可能变。
- 自己计算“已读到哪里” → 不要这样做，`mark-read` 只能回传 `inbox` 事件里的 `view_cursor`。
- Windows 上启动子进程会弹出黑色控制台窗口 → 设置 `CREATE_NO_WINDOW`（`std::os::windows::process::CommandExt::creation_flags(0x08000000)`）。
