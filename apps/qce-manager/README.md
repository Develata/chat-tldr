# QCE 管理组件任务边界

这是预留给外围协作的目录，尚无可执行程序，也尚未加入 Cargo workspace。

2026-09-26 状态：同学 A 席位尚未绑定账号，本目录还没有组件源码。主线已具备完整 JSON 导入适配器，并保留一次本机 Docker 导出 200 条消息、51 项离线检查通过的 [验收记录](../../docs/ACCEPTANCE.md#首次真实单文件验收2026-09-26)。组件可从 [已核对的 QCE 导出流程与接口](../../docs/QCE_DOCKER_EXPORT.md) 接入，交付目标是把操作封装为可重复使用的管理程序。

负责 QCE 的取得、下载、导出流程封装、组件状态和受控清理。实现与主线分开推进，交付时作为 Rust 程序加入现有 workspace，使用同一份 Cargo.lock。

## 与主线的交接

1. 管理组件把完整导出放到 `<data-dir>/sources/qce/exports/<export-id>/messages.json`。
2. 写入期间使用自己的临时目录；完成写入并关闭文件后才通知调用方。
3. 向调用方提供绝对文件路径。来源、完成时间、文件哈希等放同目录 `manifest.json`；其具体结构随管理组件接口一起审核。
4. GUI 或编排调用方执行 `chat-tldr --data-dir <DIR> import <绝对路径>`。
5. 后续分析、查询、反馈和已读操作由主 CLI 完成。

`crates/qce` 是主线负责的纯 JSON 导入适配器；本目录是外部工具与文件管理。管理组件不访问 chat-tldr 的业务数据库，不实现第二套消息去重或话题分析。

## 文件归属

- 安装的 QCE 程序：`<data-dir>/components/qce/<version>/`。
- 下载缓存：`<data-dir>/cache/qce/downloads/`。
- 当前任务临时文件：`<data-dir>/tmp/qce-manager/<job-id>/`。
- 持久状态：`<data-dir>/sources/qce/state/`。
- 原始导出：`<data-dir>/sources/qce/exports/`。
- 诊断日志：`<data-dir>/logs/qce-manager/`。

导入成功不等于允许删除导出原件。清理只针对组件明确拥有、且确认不被活跃任务使用的缓存/临时文件；原始导出、会话状态、业务数据库和用户指定的外部文件不属于缓存清理范围。

具体 QCE 调用方式、登录和打包仍需按真实工具能力设计；本说明没有声称这些能力已经验证。

相关依据：[文件布局](../../docs/FILE_LAYOUT.md)、[CLI 评审稿](../../docs/CLI_V1_REVIEW.md)、[当前安排](../../docs/ROADMAP.md)。
