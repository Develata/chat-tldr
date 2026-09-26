# 本地模型密钥

默认配置使用 `TYPESAFE_API_KEY` 连接 Jev，使用 `CHAT_TLDR_LLM_API_KEY` 连接 DeepSeek。密钥不写入 TOML。CLI 仅读取进程环境变量，不自动加载 `.env`。

将仓库根目录的 `.env.example` 复制为 `.env`，只在 `.env` 中填写密钥；已有 `.env` 时保留原文件。`.env` 及 `.env.*` 本地变体均被 Git 忽略，模板 `.env.example` 可以提交。

PowerShell 7.2 或更高版本可从仓库根目录运行：

```powershell
./scripts/with-env.ps1 cargo run -p chat-tldr -- doctor
./scripts/with-env.ps1 ./target/debug/chat-tldr.exe doctor
```

`doctor` 只检查密钥是否存在，不联网。将后面的程序参数替换为所需命令即可；实际 `analyze` 会按照配置把聊天原文发给云服务。也可在应用程序名前加 `-EnvFile <路径>` 选择另一个本地文件。

包装脚本只为这次启动的子进程设置环境，不修改当前终端环境。当前进程已经设置的变量优先，包括显式空值；需要替换时先从终端环境移除对应变量。标准输入、标准输出、标准错误及子进程退出码原样继承，命令参数通过 `ProcessStartInfo.ArgumentList` 传递。

支持 UTF-8（可带 BOM）、CRLF 或 LF、空行、`#` 注释、可选的 `export` 前缀及单行 `KEY=value`。单双引号只用来界定字面值；不解释转义，不展开变量，不执行 `$()`。不带引号时，空白后的 `#` 开始注释，值内部的 `#` 保留。重复键、缺失闭合引号和其他非法行使脚本退出 2，诊断不回显文件内容。

脚本只接受上述两个默认模型变量，拒绝包括 `PATH` 在内的其他键。自定义 `api_key_env` 时，应由启动程序的环境提供相应变量；当前脚本不会自动信任任意 `.env` 键。模板中的空值不会提供可用的模型凭证。不要把 `.env` 作为 PowerShell 脚本执行或 dot-source。

离线自检使用合成值与本地子进程，不访问云服务：

```powershell
pwsh -NoProfile -File scripts/tests/test-with-env.ps1
```
