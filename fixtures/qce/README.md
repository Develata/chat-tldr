# QCE 合成样例

这里的 JSON 全部手写合成，没有真实联系人或聊天内容。`synthetic-group.json` 含 3 条消息，覆盖 @全体成员、回复和撤回，供 CLI、GUI 与 eval 联调。会话 ID 为 `qq:group:synthetic-study`；测试不能依赖本机当前目录。

适配器参考项目 DATA_MODEL 契约，并仅核对 [QCE 7fcca88 的字段类型](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88/qq-chat-export-core/src/types.rs) 和 [元素数据字段](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88/qq-chat-export-server/src/parser/simple_parser.rs)，未复制上游实现。

当前支持完整单文件 JSON，`chatInfo.type` 必须为 `group` 或 `private`，身份取 `peerUid`、其次 `peerUin`。`temp`、分块目录、JSONL 和旧版未知导出格式暂不支持。首次 200 条真实单文件已通过离线导入验收，范围见 [ACCEPTANCE](../../docs/ACCEPTANCE.md)；未出现的真实元素形态仍待确认，合成样例不补充真实样本证据。

## 多场景分析样本

[scenario-analysis.json](scenario-analysis.json) 包含 100 条虚构群聊，[scenario-analysis-backfill.json](scenario-analysis-backfill.json) 提供后续历史回填输入。@我 / @全体 / @他人 / 正文伪 @、截止日期、热点闲聊、事项更正、回复、撤回和媒体等场景的消息范围、固定身份与验收界限见 [场景说明](scenario-analysis.README.md)。热门/优先等六种产品视图的实现口径见 [分析视图](../../docs/ANALYSIS_VIEWS.md)，事项更正与待回应仍为语义挑战。

额外约定：

- `id` / `seq` 接受字符串或 JSON 整数；时间戳为毫秒整数，不猜秒数。
- 没有元素而有 `content.text` 时，保留原文并发出降级警告，无法从原文推断提及或附件。
- `hash:` 来源身份使用 JSON 数组编码 `[sender.uid,timestamp,content.text]` 后计算 BLAKE3。此回退无法区分相同发送者、时间和原文的两条不同消息，因此会发出警告；导出应优先保留 `id` / `seq`。
- 合并转发只展开两层，更深处只留标题。多个转发元素在父消息的同一个 `ForwardBundle` 中按元素顺序合并，并发出警告。
- 普通消息缺少发送者身份、时间或已知文本元素的内容时报错；系统消息没有真实发送者时用 `qq:system`。缺少可选字段和新增未知字段不会导致 panic。
- 撤回消息的正文、提及、回复和附件/转发内容全部清空，避免导出的残留原文继续进入处理流程。

## 本地 Docker 上游核对模板

`template-docker-export.json` 是按本地 `E:/gitclone/qq-chat-exporter` 的 commit `7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c` 手写的单文件导出模板，**不是容器实际导出的聊天数据，也不是 NapCat 原始消息**。包含 7 条合成消息、实际顶层 `metadata/chatInfo/statistics/messages/exportOptions`、资源列表、回复、提及、转发和系统/撤回形态。所有 UID、QQ 号、消息 ID、文字、文件名和统计都为人工构造，不读取附件内容。

字段依据及导出步骤见 [QCE_DOCKER_EXPORT.md](../../docs/QCE_DOCKER_EXPORT.md)。模板 `metadata.version=0.1.0` 对应本地 exporter crate 的编译版本默认值，不代表已验证运行中 Docker 镜像的版本。上游可选的 `avatars`、`content.html`、`rawMessage` 在此模板省略。
