# QCE 管理组件

`chat-tldr-qce-manager` 把本机 QCE/NapCat 的登录与 JSON 导出封装为独立 Rust CLI。它只依赖共享 core，不调用模型、不打开数据库；导出完成后由用户或编排程序调用主 CLI 的 `import`。

支持 `version`、`status`、`login`、`chats`、`export`、`clean`。不安装 QCE、不启动/停止/重建 Docker、不自动导入，不做账号密码或快速登录。Windows GUI 包随附本管理程序，QCE/NapCat 本身不捆绑。

## 使用

```powershell
cargo build -p chat-tldr-qce-manager --locked
$manager = './target/debug/chat-tldr-qce-manager.exe'
# 本机转发端口是 40654；常规部署默认端口为 40653。
& $manager --docker napcat-qce --base-url http://127.0.0.1:40654 status
& $manager --docker napcat-qce --base-url http://127.0.0.1:40654 login
& $manager --docker napcat-qce --base-url http://127.0.0.1:40654 chats
& $manager --data-dir 'private/我的数据' --docker napcat-qce --base-url http://127.0.0.1:40654 export --type group --peer '<peerUid>' --since '2026-09-26T00:00:00-03:00' --until '2026-09-26T23:59:59.999-03:00'
# 从成功 ack.detail.path 取得完整绝对路径，再显式交给主 CLI：
./target/debug/chat-tldr.exe --data-dir 'private/我的数据' import '<messages.json 绝对路径>'
& $manager --data-dir 'private/我的数据' clean --dry-run
```

Linux/macOS 使用同名无 `.exe` 的程序。`--data-dir` 默认值与主 CLI 相同（Windows `%APPDATA%/chat-tldr`、macOS `~/Library/Application Support/chat-tldr`、Linux `$XDG_DATA_HOME/chat-tldr` 或 `~/.local/share/chat-tldr`）。相对路径相对当前工作目录解析。组件管理目录内部不允许经过符号链接或 Windows reparse point。

全局参数可放在子命令前后：`--base-url`（默认 `http://127.0.0.1:40653`）、`--napcat-url`（默认 `http://127.0.0.1:6099`）、`--timeout-secs`（默认 15，范围 1–3600）、`--docker`、`--security-json-path`、`--qce-config-dir`、`--napcat-config-dir`。

两个服务地址都仅支持 HTTP(S) loopback IP 或 `localhost`，拒绝 URL 凭据、查询参数及路径；localhost 固定解析到 `127.0.0.1`。HTTP 客户端禁用环境代理和重定向，下载地址必须与 QCE 同源，不提供远程访问开关。

## 自动认证与登录

QCE 令牌按首个非空值查找：

1. `CHAT_TLDR_QCE_TOKEN`，供手动覆盖。
2. `--docker` 指定容器中的 `/app/.qq-chat-exporter/security.json`，再尝试 `/root/.qq-chat-exporter/security.json`；`--security-json-path` 显式指定时只尝试该容器路径。
3. `--qce-config-dir`、`QCE_CONFIG_DIR`、`~/.qq-chat-exporter/` 中的 `security.json`。

只解析 `accessToken`，其余字段不保留。缺字段、损坏或读取失败时继续下一个来源；已选 Token 被 HTTP 401/403 拒绝则报认证错误，不盲试其他账号来源。每次命令重新读取，无磁盘缓存。Docker 命令使用参数数组，不经过 shell；输出内部捕获、限长、限时，不回显。

NapCat 使用 `CHAT_TLDR_NAPCAT_TOKEN`，或容器 `/app/napcat/config/webui.json`，或 `--napcat-config-dir` 中的 `webui.json`，只解析 `token`。认证发送 `sha256(token + ".napcat")`，仅在内存保存返回的 Credential。连接失败或拒绝 Credential 时重新认证并重试一次。接口字段以本地核对的上游 `7fcca88880c2eb8c12c51c2b6cc49ee805a53d0c` 为依据，未复制 GPL 源码。

`status` 只读诊断 QCE/QQ 和可选 Docker 状态，输出带当前端口的 `login_hint`。`login --max-wait-secs N`（默认 180，上限 86400）先检查 QQ 状态：已登录不请求二维码；未登录仅在 **stderr 是交互终端** 时取码并渲染 Unicode 字符画，二维码变化后刷新。stderr 被重定向或管道捕获时，未登录返回 `E_QCE_LOGIN_REQUIRED`，不会取码、输出二维码或写二维码文件。

总等待期限包含认证、扫码和 QCE 启动。QQ 已登录后仍需等 QCE 可达且认证通过；等待时重新读取尚未生成的 Token。等待期间丢失登录直接退出，不再次转入扫码。Ctrl-C 返回 130；正在执行的 blocking HTTP 最迟在请求超时后观察到取消。二维码在真实终端的可扫描性需要用户实际扫码验收。

## GUI 登录接口

`login --qr-events` 是显式授权的 GUI 管道模式，`version.capabilities` 包含 `login.qr-events`。仅此模式允许在非交互进程中请求二维码；普通 `login` 保留上面的终端限制。二维码每次变化时输出扩展事件 `qce_login_qr`，payload 为 `{"version":1,"content":"<二维码原始内容>"}`，不增加 core 的冻结类型。

GUI 只在内存中将 content 编码为二维码，不能把它写入日志、偏好、缓存或诊断回执；收到 `progress.stage=logged_in`、错误、完成、取消或关闭窗口后立即清除。协议不提供过期时间，界面不得捏造倒计时。已有登录仍不会请求二维码；登录成功须同时等待 QCE 就绪。二维码内容最多 4096 字节且必须可编码。

GUI 先通过同版本/同 schema/capabilities 握手，再使用 `status`、`chats` 和 `export`。`status` 可先返回诊断 ack 再以非零状态结束，不能把 ack 单独当成功。`qce_chat` 仅在完整流和实际进程成功后发布；导出路径也必须等成功 `done` 和正常退出后才交给主 CLI。导入失败保留原文件，重试导入不重做导出。

## 导出、发布与清理

`chats` 列出 QCE 最近联系人接口返回的 group/private 会话（查询上限 2000），不声称覆盖全部历史会话；名称和标识属于私有输出。

`export` 使用普通 JSON API，保留系统消息和媒体元数据，关闭资源下载、ZIP、头像嵌入和流式导出。`--since` / `--until` 是带时区、精度最多到毫秒的 **闭区间**，不同于主 CLI 分析窗口的排他终点。`--poll-secs` 默认 1（1–3600）；`--max-wait-secs` 默认 300（1–86400），约束从认证、创建任务到下载结束的等待。

只在上游 `completed` 且 progress 为 100 时下载。下载检查 Content-Length（若有），以固定缓冲区计算 SHA-256，再流式验证单文件 JSON 和统计顶层 `messages` 条数；拒绝损坏 JSON、分块 manifest 和尾随数据。不会把整份聊天复制成内存 DOM。

临时文件写在 `<data-dir>/tmp/qce-manager/<job-id>/`，关闭并同步文件后，将整个目录一次同卷 rename 到 `sources/qce/exports/<export-id>/`。随机 ID 与发布锁防止 manager 并发覆盖既有导出。同级 `<job-id>.lock` 持有到发布完成，`clean` 不会在 rename 前删除活跃目录。强杀残留锁在操作系统释放后可由 clean 回收。

成功 `ack.detail` 包含 `path`、`manifest`、`sha256`、`message_count`。失败不输出成功路径，发布前错误清理本次临时目录；强杀或底层文件系统拒绝清理时可留给后续 clean。rename 后导出已提交，即使 stdout 管道断开也保留原件。超时/取消不取消或删除上游任务，服务端可能继续导出。

manifest 组件 schema 为 `1`，包含 `export_id`、`created_at`（UTC）、`tool_version`、`qce_base_url`、`chat_type`、`peer_uid`、`since`、`until`、`task_id`、`message_count`、`file_size`、`sha256`。它包含私有会话标识，不可提交；共享 core 与主 CLI 协议不变，manifest 的长期冻结另行审核。

`clean` 只删除本组件受控名称下的非活跃临时任务及 `cache/qce/downloads/` 内容，不接收任意删除路径，不跟随链接；未知临时目录保留。`--dry-run` 不创建目录/文件、不删除内容。组件协调锁 `sources/qce/state/manager.lock` 保留；已发布导出、数据库、会话状态、用户输出不清理。

## JSONL 与错误

stdout 使用共享 `CliEvent` 信封，连续 seq、固定 run_id，正常可写时最后恰好一个 done，退出码与 done 一致。`--help` / `--version` 是文本例外；参数错误仍输出 error + done。强杀、崩溃及管道断开不保证收尾。`qce_chat` 是组件自己的扩展事件，payload 为 `chat_type`、`peer_uid`、`display_name`；复用 core 的未知事件支持，不改变主协议的 chat。

| 错误 | 退出码 | 含义 |
|---|---|---|
| `E_USAGE` | 2 | 参数、非本机 URL 或时间范围无效 |
| `E_INPUT_PARSE` / `E_QCE_CHUNKED` / `E_QCE_DOWNLOAD` | 3 | 文件格式或下载地址不符合约束 |
| `E_CONFIG` / `E_QCE_AUTH` / `E_NAPCAT_AUTH` / `E_QCE_DOCKER` / `E_QCE_LOGIN_REQUIRED` | 4 | 配置、认证、Docker 或交互登录不可用 |
| `E_QCE_UNREACHABLE` / `E_QCE_TIMEOUT` / `E_QCE_LOGIN_TIMEOUT` / `E_QCE_TASK_FAILED` | 5 | 外围服务或任务失败、等待超时 |
| `E_QCE_RESPONSE` | 3 / 5 | 非法响应结构 / HTTP 错误状态 |
| `E_QCE_DOWNLOAD` | 5 | 下载中断或长度不符 |
| `E_OUTPUT_WRITE` | 8 | 输出、文件或锁操作失败 |
| `E_RUN_IN_PROGRESS` | 7 | 另一管理进程正在协调目录操作；稍后重试 |
| `E_CANCELLED` | 130 | 收到 Ctrl-C |
| `E_INTERNAL` | 1 | 客户端或信号处理初始化失败 |

诊断不回显服务响应、配置原文、Token、Credential 或 secretKey。

## 本机部署与验收范围

QCE 的已有手工启动记录见 [QCE_DOCKER_EXPORT](../../docs/QCE_DOCKER_EXPORT.md)。Windows 获取上游建议 `git clone -c core.autocrlf=false`，避免 CRLF shell 入口进入 Linux 镜像。本机已登录容器保持原状；下列命令仅供用户日后手工启动，不由 manager 执行：

```powershell
docker compose -p docker -f E:/gitclone/qq-chat-exporter/docker/docker-compose.yml -f E:/gitclone/chat-tldr/private/qce-runtime/crlf-entrypoint/compose.override.yml up -d --no-build napcat-qce
docker compose -f E:/gitclone/chat-tldr/private/qce-runtime/local-access/compose.yml up -d --no-build --pull never
```

40653 发布异常的根因未解决；本机调用显式使用 40654。自动化测试全部使用合成数据、本地假 HTTP 服务和可注入的 Docker 接口，不访问真实云服务。真实联调仅在已有 QQ 登录有效时执行，只导出预先授权群并离线 import；扫码路径在用户亲自扫描前保持“真机未验证”。本次具体验证与 review 结果随 PR 报告记录。
