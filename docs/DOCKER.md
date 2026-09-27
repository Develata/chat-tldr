# CLI Docker 镜像

镜像只运行 `chat-tldr` CLI：静态 musl 程序、MIT 许可证和 UID/GID 10001 的必要目录，基础层为 `scratch`。不包含 shell、Rust、Python、GUI、QCE、字体、数据库、聊天或密钥。首版目标为 `linux/amd64`；Windows 使用 Docker Desktop 的 Linux 容器。

## 本地构建与导入

仓库根目录运行：

```powershell
docker compose build cli
docker compose run --rm cli version
# 将 QCE 完整单文件 JSON 放入被忽略的 exports/；此目录只读挂载。
docker compose run --rm cli import /imports/export.json
docker compose run --rm cli chats
docker compose run --rm cli messages --chat <CHAT_ID>
docker compose run --rm cli analyze --chat <CHAT_ID> --dry-run
docker compose run --rm cli analyze --chat <CHAT_ID>
docker compose run --rm cli inbox --chat <CHAT_ID> --all
docker compose run --rm cli relations --chat <CHAT_ID>
```

分析会把原文发送到 Jev/DeepSeek；`.env` 的两个模型密钥由 Compose 注入运行时，不参与构建。只有导入/查询时可以不填密钥。容器以非 root 运行，根文件系统只读、移除 capabilities，`/tmp` 为临时内存目录。`/data` 使用 `chat-tldr-data` 命名卷，默认数据库位于 `/data/chat-tldr/chat-tldr.db`；`--data-dir /data/实验名` 可隔离实验。重复 `run --rm` 保留命名卷数据。

`docker compose down` 保留数据；不要在需要保留数据时使用 `down -v`。备份前结束写入进程，并一起保留 SQLite 主文件、WAL/SHM 或使用 SQLite 一致性备份。

需要自定义配置时可执行 `config init --out /data/config.toml`，通过单独挂载编辑后的 TOML 使用 `--config`。密钥可继续放环境变量；也可用 `config set llm --key-from-env CHAT_TLDR_LLM_API_KEY` 保存到数据卷内的 config（明文），之后保存值优先；`--clear-key` 恢复环境变量查找。只读挂载不能使用 config set。`inbox --html /data/inbox.html` 的输出也保存在数据卷内。容器不读取宿主机任意目录。

## 正式镜像与发布归档

正式 tag 流程成功后使用 `ghcr.io/develata/chat-tldr:0.2.0`；`latest` 指向最近正式稳定版本。正式发布之前，`compose build` 或 Actions 的模拟发布产物可用于验收，不能把尚未发布的版本当作可拉取镜像。

```powershell
$env:CHAT_TLDR_IMAGE = 'ghcr.io/develata/chat-tldr:0.2.0'
docker compose pull cli
docker compose run --rm cli version
```

Actions 提供的 `docker-image.tar` 可用 `docker load -i docker-image.tar` 离线导入。镜像标签中有源码 revision/version，发布时复核 revision 并重用已测试的镜像。依赖缓存只在构建阶段；`.dockerignore` 采用显式源码白名单。

## 验证边界

`scripts/release/smoke.py --image <IMAGE> --version 0.2.0` 在最终 scratch 镜像中运行合成导入、模型假服务、证据/P0、状态操作、历史、HTML 和关系覆盖查询。它保持镜像的 UID/GID 10001，使用一次性命名卷保存输出，通过 Docker 读取私有文件，避免宿主用户与容器 UID 不同导致 Linux 权限错误。只读挂载合成输入，结束后仅移除本次创建的卷。测试服务器仅提供合成响应，不接入云模型。Linux CLI 归档从同一镜像提取并再次运行验收。真实模型质量需要独立人工标注。
