# QCE 合成样例

这里的 JSON 全部手写合成，没有真实联系人或聊天内容。`synthetic-group.json` 含 3 条消息，覆盖 @全体成员、回复和撤回，供 CLI、GUI 与 eval 联调。会话 ID 为 `qq:group:synthetic-study`；测试不能依赖本机当前目录。

适配器参考项目 DATA_MODEL 契约，并仅核对 [QCE 7fcca88 的字段类型](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88/qq-chat-export-core/src/types.rs) 和 [元素数据字段](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88/qq-chat-export-server/src/parser/simple_parser.rs)，未复制上游实现。

当前支持完整单文件 JSON，`chatInfo.type` 必须为 `group` 或 `private`，身份取 `peerUid`、其次 `peerUin`。`temp`、分块目录、JSONL 和旧版未知导出格式暂不支持。尚未用真实导出验收，Q-QCE-1 至 Q-QCE-6 的真实样本问题仍保留。

额外约定：

- `id` / `seq` 接受字符串或 JSON 整数；时间戳为毫秒整数，不猜秒数。
- 没有元素而有 `content.text` 时，保留原文并发出降级警告，无法从原文推断提及或附件。
- `hash:` 来源身份使用 JSON 数组编码 `[sender.uid,timestamp,content.text]` 后计算 BLAKE3。此回退无法区分相同发送者、时间和原文的两条不同消息，因此会发出警告；导出应优先保留 `id` / `seq`。
- 合并转发只展开两层，更深处只留标题。多个转发元素在父消息的同一个 `ForwardBundle` 中按元素顺序合并，并发出警告。
- 普通消息缺少发送者身份、时间或已知文本元素的内容时报错；系统消息没有真实发送者时用 `qq:system`。缺少可选字段和新增未知字段不会导致 panic。
- 撤回消息的正文、提及、回复和附件/转发内容全部清空，避免导出的残留原文继续进入处理流程。
