# QCE Docker 导出模板与输入边界

已核对本地源码 `E:/gitclone/qq-chat-exporter`，HEAD 为 `7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c`，受检查的跟踪文件无工作区修改。核对只读取源码及 compose；未读取容器日志、登录令牌、真实聊天或导出文件，未操作容器。因此这里确认的是该源码的格式，尚未确认运行中的镜像、QQ 登录状态和真实导出成功与否。

配套文件是 [template-docker-export.json](../fixtures/qce/template-docker-export.json)，共 7 条人工合成消息，可直接作为后续离线处理输入。

## 从现有界面取得输入

1. compose 发布主机端口 `40653` 和 `6099`；源码主界面入口为 `http://localhost:40653/qce`。按现有界面完成登录，不把令牌写进仓库或交接材料。
2. 在 QCE 导出任务中选择目标群和时间范围，格式选 **JSON**。
3. 在高级选项中关闭 **“流式导出（超大消息量专用）”**。源码明确显示这个开关会把 JSON 切换成分块 JSONL；当前 chat-tldr 接受单文件 JSON。
4. 等任务完成，通过界面下载 JSON 到本机。把完成后的文件路径交给 CLI；不要把容器内路径当成本机路径，也不要读仍在写入的输出。

已存在的界面请求构造和服务端路由相互吻合：普通任务使用 `POST /api/messages/export`，流式 JSONL 使用 `POST /api/messages/export-streaming-jsonl`。本轮不设计新的 API，也不尝试携带登录态调用它们。

compose 的导出目录没有显式主机 bind mount：卷包含会话数据、QCE 数据以及 `./config`。因此当前建议使用界面下载；不能凭 compose 推断某个主机 `exports/` 文件夹已经存在。

## 字段对应

| 上游实际输出 | chat-tldr 处理 | 核对结论 |
|---|---|---|
| `metadata/chatInfo/statistics/messages[]`，可选 `avatars/exportOptions` | 读取 `chatInfo/messages`，忽略未知附加字段 | 顶层一致 |
| `chatInfo.type` 为 `group/private/temp`；`peerUid/peerUin/selfUid/selfUin` 为可选字符串 | 支持 group/private，temp 明确不支持 | 无字段错位 |
| `messages[].timestamp` 为毫秒整数；`time` 在当前 parser 中是 UTC RFC3339 | 只用 timestamp 并转换为配置时区 | 不需要乘 1000；不要用 `time` 猜时区 |
| `sender.uid/uin/name/nickname/groupCard/remark` | 身份仅取 uid；昵称等作为别名 | 无字段错位；普通消息 uid 为 `未知` 时适配器拒绝，不制造身份 |
| 顶层消息 `type` 常见值是 `text/file/video/system/audio/forward/reply/json`，未识别数字才映射 `type_N` | 字符串原样记入来源 | 旧小样例 `type_1` 不是该版本普通文字消息的典型值，但不影响适配器 |
| `content.elements[].type/data` | 按元素顺序规范化 | 与 NapCat `elementType/textElement/...` 原始结构不同 |
| at 的 `data.uid` 可为 NT UID、数字 QQ 号、`unknown` 或 `all`；另有 `data.uin` | NT UID 与 QQ 号分开保存，保留 @全体成员 | 结构一致；真实样本值域仍待验收 |
| reply 的 `referencedMessageId` 可空，备选 `messageId`；reply 内 `timestamp` 为秒 | 仅保存引用 ID，不使用 reply 时间戳 | 不与主消息的毫秒混用 |
| forward 的 `data.messages[]` 为含 `sender/content/timestamp` 的归一化对象，内层 timestamp 也是毫秒 | 最多展开两层 | 不递归解析 NapCat raw |
| `content.resources[]` 含媒体元数据，element 的 `data.filename/size` 对应相同附件 | 从元素构造附件元数据，不读 URL/本地文件 | 当前结构一致 |
| `recalled/system` 为布尔值，匿名系统 sender.uid 为 `未知` | 撤回清空正文；匿名系统归一化为 `qq:system` | 与现有约定一致 |

系统、JSON 卡片、位置元素目前只输出 `[system]`、`[json]`、`[location]` 并警告；这是现有保守支持范围，不是已读取其所有文字。模板不会证明真实账号场景下的导出完整性或字段稳定性。

分块 manifest 的确切结构现已找到：顶层有 `metadata/chatInfo/statistics/chunked`；`chunked.format="jsonl"`，另含 `chunksDir/chunkFileExt/maxMessagesPerChunk/maxBytesPerChunk/chunks`，没有 `messages[]`。适配器已根据该结构精准识别并返回 `QceError::UnsupportedExport`，由 CLI 映射为 `E_INPUT_UNSUPPORTED`；MVP 仍不读取分块消息。识别仅在单文件解析失败后进行，不凭文件名猜测；格式不完整的 JSON 仍报解析错误，有 `messages[]` 的合法单文件仍兼容额外未知字段。

## 可追溯源码位置

以下链接固定到核对的 commit，行号来自本地对应文件：

- [docker/docker-compose.yml:1](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c/docker/docker-compose.yml#L1)：服务、端口和卷；[server main.rs:259](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c/qq-chat-export-server/src/main.rs#L259)、[main.rs:428](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c/qq-chat-export-server/src/main.rs#L428)：API/页面路由。
- [task-wizard.tsx:1123](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c/qce-v4-tool/components/ui/task-wizard.tsx#L1123)、[task-wizard.tsx:1329](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c/qce-v4-tool/components/ui/task-wizard.tsx#L1329)：JSON 选择和流式开关；[export-request.ts:27](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c/qce-v4-tool/lib/export-request.ts#L27)：实际请求构造。
- [json_templates.rs:52](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c/qq-chat-export-core/src/json_templates.rs#L52)、[json_exporter.rs:328](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c/qq-chat-export-core/src/json_exporter.rs#L328)：单文件骨架以及 CleanMessage 序列化；[json_exporter.rs:716](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c/qq-chat-export-core/src/json_exporter.rs#L716)：exportOptions。
- [types.rs:40](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c/qq-chat-export-core/src/types.rs#L40)、[types.rs:129](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c/qq-chat-export-core/src/types.rs#L129)、[stats.rs:48](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c/qq-chat-export-core/src/stats.rs#L48)：sender、content、message、统计结构。
- [simple_parser.rs:835](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c/qq-chat-export-server/src/parser/simple_parser.rs#L835)、[simple_parser.rs:1121](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c/qq-chat-export-server/src/parser/simple_parser.rs#L1121)、[simple_parser.rs:1194](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c/qq-chat-export-server/src/parser/simple_parser.rs#L1194)：秒转毫秒、系统/撤回、消息 type 的实际值。
- [simple_parser.rs:1283](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c/qq-chat-export-server/src/parser/simple_parser.rs#L1283)、[simple_parser.rs:1371](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c/qq-chat-export-server/src/parser/simple_parser.rs#L1371)、[simple_parser.rs:1470](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c/qq-chat-export-server/src/parser/simple_parser.rs#L1470)、[simple_parser.rs:2382](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c/qq-chat-export-server/src/parser/simple_parser.rs#L2382)、[simple_parser.rs:3055](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c/qq-chat-export-server/src/parser/simple_parser.rs#L3055)：at、媒体、forward、内层消息和 reply 字段。
- [json_exporter.rs:102](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c/qq-chat-export-core/src/json_exporter.rs#L102)、[json_exporter.rs:568](https://github.com/shuakami/qq-chat-exporter/blob/7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c/qq-chat-export-core/src/json_exporter.rs#L568)：分块 manifest 的真实字段与 format 值。

只据此编写独立的合成 JSON 和说明，没有复制上游 GPL 实现代码。
