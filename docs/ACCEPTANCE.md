# 交付验收

验收分为输入兼容性、真实云调用和原生平台三部分。每次使用独立数据目录，并记录实际运行的二进制；合成样本通过不代表真实群聊质量通过。

## 2026-09-26 首版 CLI/Docker 验收

- `v0.1.0` 对应源码 **427 项 Rust 测试通过**；新增关系事务/迁移回滚、用户反馈/游标保留、完整/部分回答、撤回与时间窗口、B0、Jev 控制器、话题指标和校准回归。严格 clippy、fmt 及远程 CI 通过，后续提交仍需按对应版本核验。
- 真实 Ours/B0 在全新独立 profile 各完成 200 条，分析退出码均为 0；估算费用分别 $0.040238394 / $0.0103278。Ours 有 15 次 Jev 控制器选择，也记录了一次 Jev 降级。完整运行与限制见 [报告](../reports/2026-09-26/README.md)。
- 真实质量与 Jev 校准曲线为**待标注**。已导出 200 条未标注 CSV 给同学 C；禁止 LLM 生成真实 gold。合成对比表与校准 SVG 只证明工具流程/手算一致。
- 发布范围为 Windows CLI、Linux 静态 CLI 和 scratch Docker；GUI 打包后续补齐。分支模拟和 tag 正式发布规则见 [RELEASING](RELEASING.md)。模拟成功不能代替真实平台/云质量验收。
- 本机 Windows release exe 与 Docker 镜像各通过 **22 次 CLI 调用**的本地假服务验收；正式发布 CI 的 Windows/Linux CLI 与 Docker 冒烟也通过。发布镜像 inspect 大小 **4,645,400 bytes**，UID/GID 10001，scratch、只读根文件系统；该尺寸不等于压缩下载大小。
- Compose 配置校验及隔离命名卷验收通过：第一次容器导入合成数据，第二次容器查询到同一会话。仅清理本次验收创建的卷，未操作用户数据卷。

### 已发布版本与复核凭据

首版 [v0.1.0](https://github.com/Develata/chat-tldr/releases/tag/v0.1.0) 已于 2026-09-26 20:38:37 UTC 发布，来源为 [PR #4](https://github.com/Develata/chat-tldr/pull/4) 的合并提交 `f9960d082369c79706eeb2f4dee71acaaf20a968`。后续文档修订不移动该 tag。

| 验收项 | 结果与证据 |
|---|---|
| 源码质量 | [main CI](https://github.com/Develata/chat-tldr/actions/runs/36269891059) 成功；线上 Protect main 严格要求 `fmt`、`clippy`、`test` |
| 发布链 | [main 模拟发布](https://github.com/Develata/chat-tldr/actions/runs/36269891359) 与 [tag 正式发布](https://github.com/Develata/chat-tldr/actions/runs/36269921242) 均成功 |
| 可下载产物 | Windows ZIP、Linux musl tar.gz、Docker 镜像归档及 SHA256SUMS 已发布；下载后逐件校验 SHA-256，实际运行 Windows exe 的 `version` |
| 容器分发 | `ghcr.io/develata/chat-tldr:0.1.0` 匿名拉取成功，digest 为 `sha256:1f80db8d4a15d522af57a9617ed78d3387afc0fbb8f264f26e4ccefd932069f9`；拉取的镜像以只读根文件系统运行 `version` 成功 |
| 真实运行复核 | Ours/B0 共 16 份原始 JSONL 与退出码再次通过 `check-stream`；200 行标注 CSV 仍未填写，真实质量/校准继续标为待标注 |

真实云实验运行的是报告中记录 SHA-256 的本地构建，不是下载后的发布 exe；发布产物验证与云实验分别记录，不能互相替代。程序化的关系/引用校验也不代表人工语义质量通过。

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

## 首次真实单文件验收（2026-09-26）

用户完成 QQ 登录并授权任选最近有记录的群。通过本机 QCE 服务创建普通 JSON 导出，等待任务完成后下载，实际 200 条消息、113,320 字节；随后以独立 profile 运行本页脚本，51 项检查全部通过：

- 首次导入 seen/inserted=200；重复导入 inserted=0、duplicate=200、changed=false。
- 查询得到 1 个群、200 条消息；逐命令 JSONL 与真实退出码校验通过，分析/已读游标未推进。
- 输入 JSON 以及 CLI/eval 二进制的前后 SHA-256 一致；隔离的 doctor 因移除模型密钥返回 E_CONFIG/4 是预期结果。
- 真实样本包含 text、at、face、image、reply、json；两个 json 卡片触发 `W_UNKNOWN_ELEMENT`，仅保留 `[json]` 占位，**卡片正文未读取/未归一化**。该告警保留在导入回执里。
- 独立只读结构核对：200 条消息的 UTC 毫秒、cursor 时间、发送者映射和 recalled/system 标志均无差异；4 条回复均正确解析到样本内目标。selfUid/selfUin 存在，19 个 @ 元素均不指向自己或全体，规范化 mentions_me 全 false，与输入一致。

原始导出位于本机 `private/exports/qce-actual-20260926T144303Z-d6adbf43/export.json`，导出回执在同目录；离线验收回执为 `private/acceptance/20260926T144429318Z-caeacdec82cb48ca9d803ec442a74aab/receipt.json`。这些文件和数据库全部忽略，不提交真实内容、身份或密钥。通用脚本自身不证明导出来源（仍保留 `export_provenance_not_verified`）；本次来源由另存的 QCE 创建/完成任务记录与下载回执支持。

这次离线验收只覆盖该输入样本；未出现的 @me/@all、跨导出边界回复、撤回、系统或合并转发不能据此视为真实形态已验证，媒体附件没有下载。后续云调用见本页首版记录；GUI 真实数据交互和人工质量评分仍未验收。

## 标注后的离线评分

独立人工标注通过 `import-sheet` 后，保存同一实验 profile 的完整 `messages`、`analyze` 与 `inbox --all --include-resolved --include-rejected` 输出，逐份校验真实退出码，再运行：

```powershell
chat-tldr-eval score --gold eval/private/gold/course-demo --run eval/private/runs/exp-01/ours --out eval/private/results/exp-01/ours.csv
```

当前支持 Ours/B0 的抽取/截止日期/排序、话题匹配/ARI/NMI、保存快照的 rejected 比例与单次运行用量；`calibrate` 单独生成 ECE、binary Brier 与 SVG。零分母、缺失统计和未实现指标留空并标记状态；不把合成测试分数当作模型质量，不把快照 rejected 比例当作所有原始提案或人工语义的幻觉率。输入范围、公式与不可机械验证的快照边界见 [eval/README](../eval/README.md#已实现离线评分)。真实质量报告需等待人工 gold。

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
