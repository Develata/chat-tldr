# CLI、Windows GUI 与 Docker 发布

`.github/workflows/release.yml` 在每次分支 push、PR 和手动运行时模拟发布：并发构建 Windows x86_64 CLI/GUI、Linux x86_64 musl CLI 和 scratch Docker 镜像。模拟只上传短期 Actions artifacts，不创建 Release、不推 GHCR、不创建 tag。GUI 归档从下一次正式版本开始提供，不追补或替换已发布的 `v0.1.0` 产物。

## 发布产物

- `chat-tldr-<VERSION>-windows-x86_64.zip`，包含 `chat-tldr.exe`。
- `chat-tldr-gui-<VERSION>-windows-x86_64.zip`，包含同目录的 `chat-tldr-gui.exe` 和 `chat-tldr.exe`，以及 GUI 使用说明和内嵌中文字体的 OFL 许可证。
- `chat-tldr-<VERSION>-linux-x86_64-musl.tar.gz`，包含静态 `chat-tldr`。
- `docker-image.tar`，为已验证镜像的离线副本。
- 每个归档的 SHA-256 文件及正式发布的 `SHA256SUMS`。

CLI 归档还含许可证、README、示例配置和 Docker/发布说明。发布程序以显式白名单打包，拒绝覆盖已有归档。构建不需要真实聊天或模型密钥；最终二进制和最终镜像都经过本地假服务验收。

## GUI 自动化验收

- 必需检查 `test` 包含独立并发的 `GUI smoke (Linux)`：Xvfb/Mesa 打开真实 GUI 窗口，覆盖收件箱、证据、决策、统计、总览、窄窗口和深色；另外通过同目录 CLI 导入合成消息，验证 GUI 自动握手与收件箱查询。缺失 CLI 的负例必须明确报错。
- Windows 发布任务运行 `cargo test --release -p chat-tldr-gui`，验证优化构建中的协议、失败传播与 egui 指针交互；生成 ZIP 后，校验 SHA-256 和文件白名单，解压到独立目录检查两份 EXE 的版本和 CLI 导入/查询。
- `gui-native-smoke` artifact 保存原生截图、状态回执和二进制哈希；`gui-release-test` 保存 Windows 解包验证记录。目录必须全新，缺失回执、截图损坏、超时或错误状态都会失败。

Windows runner 的解包检查使用 `--package-only`，明确不代表原生窗口渲染；Linux 原生 smoke 也不代替 Windows/macOS GPU、缩放、系统文件选择器或真实云质量验收。Windows 有可用图形桌面时，可以去掉 `--package-only` 对解包后的 release EXE 跑相同原生 smoke。

```powershell
cargo build --release --locked -p chat-tldr -p chat-tldr-gui
python scripts/release/package.py --binary target/release/chat-tldr.exe --gui-binary target/release/chat-tldr-gui.exe --target windows-x86_64 --version 0.1.0
python scripts/release/gui_smoke.py --archive dist/chat-tldr-gui-0.1.0-windows-x86_64.zip --version 0.1.0 --out tmp/gui-release-native
```

## 正式 tag

用户确认首版范围包括更正/取消关系与待回应识别；此外必须补齐真实服务联调、B0/评估/校准工具和 Jev 控制器。真实 gold 尚未回收时，报告明确标记质量与真实校准曲线“待标注”，仅提交运行与费用事实及合成工具验证。

1. 当前提交通过 `fmt`、`clippy`、`test` 和模拟发布，审核变更后合并 main。
2. 确认 `Cargo.toml` 的工作区版本与 tag 相同；首版 `0.1.0` 对应 `v0.1.0`。
3. 在已验收的 main 提交创建 tag 并推送；不要移动或重用发布过的 tag。

```powershell
git switch main
git pull --ff-only
git tag -a v0.1.0 -m 'chat-tldr 0.1.0: CLI and Docker'
git push origin v0.1.0
```

tag push 才触发正式发布。计划任务校验 tag/version/commit 一致，且提交已属于远端 main。发布工作流重新跑完整质量检查和两个打包任务，全部成功后才创建 draft Release、上传已验证归档、推 GHCR 的版本/latest 标签，最后公开 Release。只有发布 job 申请 `contents:write` 与 `packages:write`；其余 job 只读。外部 Actions 固定完整 SHA，Node Actions 使用 Node 24。

首次 GHCR 发布后检查包的可见性和匿名拉取；仓库公开不自动证明包已公开。不要为了失败的模拟构建创建正式 tag。

## 失败恢复

构建或验收失败时修复代码，重新在分支模拟。发布中途失败可能留下 draft Release 或已推送的镜像；先检查 Actions 日志、Release 草稿、SHA256SUMS 和镜像 revision，再处理。发布脚本拒绝覆盖已有 Release，不能盲目重跑替换不同内容；已公开版本的修复使用新版本/tag。任何清理仍遵循仓库的数据和 Git 约束。

本地验证：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked -p chat-tldr
python scripts/release/smoke.py --binary target/release/chat-tldr.exe --version 0.1.0
docker build -t chat-tldr:release-test .
python scripts/release/smoke.py --image chat-tldr:release-test --version 0.1.0
```
