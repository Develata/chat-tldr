# 交付验收

验收分为输入兼容性、真实云调用和原生平台三部分。每次使用独立数据目录，并记录实际运行的二进制；合成样本通过不代表真实群聊质量通过。

## QCE 单文件离线验收

先在 QCE 中选择 JSON、关闭流式导出，等待完成并下载。不要把尚在写入的文件或 chunked manifest 当成完整导出。

在 PowerShell 7.2 或更高版本中运行：

```powershell
cargo build -p chat-tldr -p chat-tldr-eval --locked
pwsh -NoProfile -File .\scripts\verify-qce.ps1 -InputFile 'C:\path\group.json'

# 先验证合成输入；不能据此宣称真实导出已经验收
pwsh -NoProfile -File .\scripts\verify-qce.ps1 -InputFile .\fixtures\qce\template-docker-export.json
```

脚本只调用主 CLI 与 eval，不直接访问 SQLite；不调用模型，不推进分析/已读状态。它执行：

1. 检查版本能力、创建隔离配置并运行离线 doctor。移除子进程里的模型密钥；doctor 因无密钥退出 4 是预期结果。
2. 导入两遍，核对第二遍 `inserted=0`、`changed=false` 和重复计数。
3. 查询群列表和消息，核对存储消息数；执行 `analyze --dry-run` 与空收件箱查询。
4. 对每次 CLI 调用，用 `chat-tldr-eval check-stream --exit-code` 校验 JSONL 和实际进程退出码；核对三个游标与未读数未改变。
5. 核对源文件、CLI 和 eval 运行前后的 SHA-256，任一变化均判失败；失败也保留退出码、日志和回执。源文件不移动、不删除。

控制台只显示 PASS/FAIL、计数与 `receipt.json` 路径。每次回执和原始输出位于新建的 `private/acceptance/<时间-GUID>/`，包含隔离的 `profile/`。原始输出可能包含聊天正文，整个目录保持 Git 忽略，不上传。脚本不会清理历史验收记录。

可选参数：`-CliPath`、`-EvalPath` 指定待测程序，`-OutputRoot` 指定私有输出根目录，`-SelfUid` / `-SelfUin` 提供自己的身份，`-TimeoutSeconds` 设置单进程超时（默认 120 秒）。个人 @ 识别需要可靠身份；脚本通过不代表 @、回复或转发的语义已由人工逐条核对。

回执保留 `warning_codes`。`W_UNKNOWN_ELEMENT` 等提示表示保守归一化，不能解释成这些元素的全部文字均已读取。真实样本还应人工检查时间、回复、转发、撤回、系统消息和身份字段；聊天时区按配置解释，项目交付期限的 America/Santiago 时区不替代聊天时区。

## 真实云服务的小范围联调

默认密钥名为 `CHAT_TLDR_LLM_API_KEY`（DeepSeek）和 `TYPESAFE_API_KEY`（Jev），只在本机环境中配置，不写入配置文件、回执或聊天。仅检查变量存在或 `doctor` 成功不能证明云服务可用。

首次联调只导入仓库的合成样本，使用新的 `private/cloud-smoke/<运行编号>/` 数据目录，避免旧缓存让请求被跳过。先 `config init`、`import` 和 `doctor`，再运行：

```powershell
.\target\debug\chat-tldr.exe --data-dir .\private\cloud-smoke\run-01 analyze --chat qq:group:synthetic-study --decider llm --max-steps 8 --budget-usd 0.05
```

这条命令确实会发送合成文本并产生费用；预算是本地估算，不是账单硬上限。保留实际退出码、完整 JSONL、`stats --run` 和后续 inbox，按证据核对结论。partial、预算耗尽、认证错误都不能记成成功验收。

验证 Jev 时另建空数据目录，设置隔离配置的 `[agent] direct_max = 0` 后使用 `--decider jev`，使小样本也进入切分/分类路径。检查本轮 `jev-log --run` 的真实 provider/model、cache_hit 和降级告警；仅命令成功或发生 LLM 降级不能证明 Jev 通过。合成调用成功只证明协议与连通性，真实质量仍需要同学独立标注和评估。

真实群聊上传必须使用用户指定用于该次云联调的样本；离线导入验收本身不会上传任何文本。

## 平台验证

Windows 继续全量执行六个模块的 clippy/test。Linux 验证 CLI/eval 并运行合成 QCE 离线验收脚本，macOS 将 CLI、GUI、eval 分模块并行检查与测试；`clippy` / `test` 汇总要求所有应跑平台成功。具体结果以当前提交 CI 为准。

本地已用 3 条普通合成消息、7 条 Docker 模板、合法空导出和损坏 JSON 运行脚本；验证了中文/空格路径、显式身份、原始输入不变与失败回执。超时 helper 在两个输出管道分别写入 300,000 字节后等待，1 秒超时设置下约 2.13 秒返回失败并回收直接子进程；这只是一例故障回归，不是通用性能上界。

GUI 编译、headless egui 交互和 helper 子进程测试，不等于在对应操作系统上实际点过文件选择器、登录或原生窗口。Windows 原生截图见 [GUI_VERIFICATION](GUI_VERIFICATION.md) 和 [REVIEW_FIXES](REVIEW_FIXES.md)；Linux/macOS 原生窗口验收仍需相应桌面环境。
