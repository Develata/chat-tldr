# 目录与文件存放设计

> 2026-09-26。本文统一规定源码、协作材料和运行文件放在哪里、由谁读写，以及哪些可以清理。
> 用户已于 2026-09-26 采纳此布局。当前 CLI 基础分析闭环、历史查询与 eval 表格往返已实现，原生 GUI 已接入 CLI 并完成 Windows 合成数据截图与交互验收，见 [GUI_VERIFICATION](GUI_VERIFICATION.md)。下列树仍包含规划位置，文件随功能创建，不预建大量空模块。
> 保留现有六个 workspace 成员和 `--data-dir` 约定。QCE 管理组件单独预留位置，尚未加入 workspace。

## 1. 三个根目录

| 根目录 | 内容 | 生命周期 |
|---|---|---|
| **仓库** `chat-tldr/` | 源码、设计契约、测试、合成数据、配置模板 | Git 管理 |
| **程序安装目录** | CLI / GUI / 可选 QCE 管理程序、许可证 | 随版本更新；程序运行不向这里写业务数据 |
| **数据目录** `<data-dir>/` | 配置、SQLite、GUI 偏好、QCE 导出与组件状态、缓存、日志 | 用户数据；升级程序后保留 |

开发期间用 `--data-dir <仓库>/private/dev`。真实聊天、真实模型响应和运行输出都留在忽略目录内。自动化测试使用操作系统临时目录，不读写用户的默认数据目录。

## 2. 仓库布局

```text
chat-tldr/
├─ Cargo.toml                  # workspace 成员与统一依赖
├─ Cargo.lock                  # 提交，团队共用一份依赖锁
├─ config.example.toml         # 只有环境变量名，没有密钥
├─ README.md                   # 产品说明与快速开始
├─ CONTRIBUTING.md             # 开发、验证与 PR 流程
├─ LICENSE
├─ .gitignore
├─ .github/
│  ├─ workflows/ci.yml         # fmt / clippy / test
│  ├─ CODEOWNERS
│  └─ ISSUE_TEMPLATE/
├─ crates/
│  ├─ core/                    # 公共类型与 JSONL 协议
│  │  ├─ Cargo.toml
│  │  ├─ src/
│  │  │  ├─ lib.rs             # 公开导出与协议版本
│  │  │  ├─ ids.rs             # ID、Cursor、RenderProfile
│  │  │  ├─ message.rs         # UnifiedMessage、ImportBatch 等
│  │  │  ├─ insight.rs         # Insight、Evidence、时间约束
│  │  │  ├─ agent.rs           # AgentAction、AgentObservation
│  │  │  └─ protocol.rs        # CliEvent、payload、流校验；需要时再拆目录
│  │  └─ tests/                # 协议往返、兼容性、fixtures 校验
│  ├─ qce/                     # QCE JSON → ImportBatch，主线实现
│  │  ├─ Cargo.toml
│  │  ├─ src/
│  │  │  ├─ lib.rs             # parse_qce_json 入口
│  │  │  ├─ wire.rs            # QCE 文件字段结构
│  │  │  ├─ elements.rs        # text / at / reply / forward 等
│  │  │  └─ convert.rs         # 规范化、稳定 ID、导入警告
│  │  └─ tests/                # 元素覆盖、缺字段、重叠导出
│  └─ engine/                  # 全部业务处理与数据库访问
│     ├─ Cargo.toml
│     ├─ src/                  # 模块表见下
│     ├─ migrations/           # 0001_initial.sql 等，只追加版本
│     ├─ prompts/              # 英文模型提示词模板
│     │  ├─ extract/
│     │  ├─ decide/
│     │  └─ review/
│     └─ tests/                # 用例、事务、恢复、Mock 模型
├─ apps/
│  ├─ cli/
│  │  ├─ Cargo.toml
│  │  ├─ src/
│  │  │  ├─ main.rs            # 进程入口、退出码
│  │  │  ├─ args.rs            # clap 参数与互斥约束
│  │  │  ├─ paths.rs           # 数据/配置/输出路径解析
│  │  │  ├─ commands.rs        # 基础命令与状态操作，调用 engine
│  │  │  ├─ analyze.rs         # 分析参数、取消与退出状态
│  │  │  ├─ history.rs         # stats / decisions / jev-log 的协议包装
│  │  │  ├─ html.rs            # HTML 转义、证据高亮与原子输出
│  │  │  └─ output.rs          # 唯一 JSONL writer
│  │  └─ tests/                # 子进程级 CLI 合作接口测试
│  ├─ gui/
│  │  ├─ Cargo.toml
│  │  ├─ src/
│  │  │  ├─ main.rs
│  │  │  ├─ app.rs             # UI 状态与操作分发
│  │  │  ├─ bridge.rs          # 子进程、管道、事件解析；相关测试在 bridge/
│  │  │  ├─ model.rs           # 协议事件到界面状态，不打开数据库
│  │  │  ├─ prefs.rs           # 仅 GUI 偏好，原子保存
│  │  │  ├─ appearance.rs      # 字体与外观
│  │  │  └─ ui.rs              # 群列表、收件箱、证据、日志
│  │  └─ README.md             # 启动方式、--demo 与配置边界
│  └─ qce-manager/             # 预留外围组件，见该目录 README
│     └─ README.md             # 获取/导出/管理任务与文件交接边界
├─ eval/
│  ├─ Cargo.toml               # 仍是 chat-tldr-eval，不移动到 apps/
│  ├─ src/                     # 已有协议检查、CSV/gold 转换；评分/校准待实现
│  ├─ tests/                   # 协议和标注往返；未来补指标手算用例
│  ├─ synthetic/               # 合成评估聊天与人工校核的 gold
│  └─ private/                 # 真实数据、标注、实验运行；忽略
├─ fixtures/
│  ├─ qce/                     # 跨 crate 共用的合成 QCE 文件
│  ├─ jsonl/                   # GUI / CLI / eval 共用的协议样例
│  └─ models/                  # 手写/合成的模型请求响应
├─ docs/
│  ├─ README.md                # 文档导航与状态
│  ├─ FILE_LAYOUT.md           # 本文
│  ├─ ARCHITECTURE.md          # 依赖方向与模块边界
│  ├─ DATA_MODEL.md            # 类型、数据库表与字段含义
│  ├─ CLI_PROTOCOL.md          # 当前 CLI 规格
│  ├─ CLI_V1_REVIEW.md          # 已采纳的首次 v1 评审记录
│  ├─ PIPELINE.md
│  ├─ EVALUATION.md
│  ├─ ROADMAP.md
│  ├─ TEAM_ASSIGNMENTS.md       # 当前主线与同学外围分工
│  ├─ OPEN_QUESTIONS.md
│  ├─ decisions/              # 长期架构取舍及理由
│  ├─ tasks/                  # 可分派的任务说明
│  └─ ui/                     # 线框图、设计稿、交互说明
├─ reports/<交付日期>/         # 人工审核后的聚合结果、图、报告
├─ scripts/                   # 必要的 PowerShell 开发/打包脚本
├─ private/                   # 本机开发数据与素材；忽略
├─ dist/                      # 生成的发布包；忽略
├─ target/                    # Cargo 构建产物；忽略
└─ .codegraph/                # 本机代码索引；忽略
```

### engine 内部分层

沿用 ARCHITECTURE 的业务模块，按功能放置，而不是按负责人拆目录：

| `crates/engine/src/` 下的模块 | 内容 |
|---|---|
| `lib.rs` | 对 CLI 提供的服务入口与公开类型 |
| `config.rs` | 业务配置类型、默认值与校验；不自行推断文件位置 |
| `store.rs` | 连接、迁移、原子导入与基础消息查询 |
| `store/analysis.rs`、`store/inbox.rs` | 分析会话、提交与证据来源读取；收件箱、反馈、生命周期 |
| `store/history.rs` | stats/decisions/jev-log 的只读事务查询与旧记录兼容，测试在 `store/history/` |
| `render/`、`segment/` | 渲染、burst、话题候选与归属 |
| `decider/`、`llm/`、`embed/` | provider trait、真实客户端与 Mock |
| `extract/`、`temporal/`、`verify/` | 抽取、日期规范化、证据校验 |
| `rank/`、`feedback/` | 分层、排序、用户评价与偏好更新 |
| `agent/`、`cache/`、`baseline/` | 控制循环、模型缓存、评估基线 |

小模块可先用同名 `.rs` 文件或只有 `mod.rs` 的目录，内容长到需要按职责拆分时再增加文件。`core::lib.rs` 重导出公开协议类型，内部拆文件不改变调用方的导入路径。

已实现的控制器位于 `agent/mod.rs`，模型请求/缓存/预算位于 `agent/runtime.rs`；表中的 embed、baseline 等仍是规划位置。历史统计的版本化 JSON 扩展使用既有 DB v1 的 meta/日志列，不额外创建一份数据库或给 GUI/eval 开 SQL 入口。当前批次代码变更后的 `.codegraph/` 同步仍待执行，索引不作为已同步的验收证据。

SQL 迁移、提示词与 HTML 模板在编译时嵌入。可执行文件移动后，不依赖仓库路径或当前工作目录来寻找这些资源。提示词文件变更进入缓存键；不从开发目录悄悄加载另一份模板。

## 3. 运行数据布局

```text
<data-dir>/
├─ config.toml                         # CLI/engine 配置；key 只写环境变量名
├─ chat-tldr.db                        # 唯一业务数据库
├─ chat-tldr.db-wal / chat-tldr.db-shm  # SQLite 管理，不手动清理
├─ gui-state.json                      # 窗口、布局、选中会话等 GUI 偏好
├─ sources/qce/
│  ├─ settings.toml                    # QCE 管理组件自己的配置
│  ├─ state/                           # 组件运行状态/需要持久保留的会话信息
│  └─ exports/<export-id>/
│     ├─ messages.json                 # 导出完成、可导入的原始文件
│     └─ manifest.json                 # 来源、完成时间、文件哈希等来源记录
├─ components/qce/<version>/           # 管理组件下载、安装的 QCE 程序
├─ cache/qce/downloads/                # 可重新下载的安装包缓存
├─ tmp/<component>/<job-id>/           # 未完成下载、解压、临时写入
├─ logs/<component>/                  # cli / gui / qce-manager 的诊断日志
├─ outputs/                           # 用户显式导出的 HTML / JSONL / CSV
└─ backups/<snapshot-id>/              # engine 生成的一致性备份
```

- 目录按需要创建；普通查询不为一个空数据目录建立整棵目录树。
- 模型响应缓存仍存数据库 `model_cache` 表；`cache/` 目录主要用于外围组件下载缓存，不再保存第二份业务结果。
- 用户自行指定的原始 QCE 文件可以在任何位置。`import` 只读它，不强制复制、移动或删除；规范化后的消息与证据存入数据库，后续查看不依赖原文件仍位于原路径。
- QCE 管理组件自行取得的原始导出放 `sources/qce/exports/`，**不放 cache 或 tmp**。导入成功也不自动使原始导出变成可清理文件。
- 常规 CLI 事件只写 stdout，默认不额外落盘。需要留档时由调用方重定向到私有实验目录或用户指定的输出位置。
- 诊断日志默认不包含聊天正文、完整模型输入输出和密钥；携带正文的调试材料归为私有数据，而不是可提交的日志样例。

### 文件归属

| 文件/目录 | 主要写入方 | 其他组件如何使用 |
|---|---|---|
| `config.toml` | CLI 配置命令或用户显式编辑 | CLI 读取、校验后传配置给 engine |
| 数据库及 WAL/SHM | `engine::store` | GUI/eval/QCE 管理组件通过 CLI 获得业务数据 |
| `gui-state.json` | GUI | 只存界面偏好，不存一套可替代数据库的事项状态 |
| `sources/qce/`、`components/qce/`、`cache/qce/` | QCE 管理组件 | 导出完成后把 JSON 绝对路径交给 `import` |
| `tmp/<component>/` | 对应组件 | 每个任务有独立 job 目录 |
| `logs/<component>/` | 对应组件 | 用于诊断，不参与业务状态判断 |
| `outputs/` | 用户显式触发的导出操作 | GUI/用户打开结果；文件名由调用方确定 |
| `backups/` | engine 的一致性快照机制 | 恢复前检查版本；不会在启动时自动覆盖现有库 |

这里的“写入方”指代码归属，不保证只存在一个 CLI 进程。并发由数据库事务、命令锁和临时文件隔离处理，不能把目录划分当成并发锁。

### 路径解析

1. 显式 `--data-dir DIR` 优先。开发、演示、评估各自传独立路径。
2. 默认路径沿用现有 CLI 约定：Windows `%APPDATA%\chat-tldr`。本项目对 Linux 约定 `$XDG_DATA_HOME/chat-tldr`（缺省为 `~/.local/share/chat-tldr`），macOS 约定 `~/Library/Application Support/chat-tldr`。
3. `--config FILE` 只改变本次读哪份配置，不改变数据库和其他运行文件的数据根；默认读 `<data-dir>/config.toml`。
4. CLI 参数中的相对路径相对进程启动时的工作目录；配置文件内将来出现的相对文件路径相对该配置文件的父目录。入口统一转成明确的绝对路径，再传给 engine。
5. GUI 启动 CLI/QCE 管理组件时传绝对路径，并让同一用户会话使用同一个 data-dir。路径解析的权威实现在 CLI `paths.rs`；`doctor` 负责输出解析结果，GUI 不自己猜另一套默认目录。
6. 群名、昵称和 `qq:group:...` 这类逻辑 ID 不直接作为文件名。目录名使用受控的 export-id/job-id/snapshot-id，只允许安全的文件名字符；避免 Windows 的冒号、路径分隔符和跨目录问题。
7. 显式 `--html FILE` 或 stdout 重定向的路径由调用方决定；推荐放在 `outputs/`，程序不会暗中改写成另一个目标位置。

## 4. 评估文件布局

```text
eval/
├─ src/、tests/                       # Rust 工具源码
├─ synthetic/<dataset-id>/
│  ├─ group.json                      # 合成聊天
│  ├─ gold/messages.jsonl
│  ├─ gold/items.jsonl
│  └─ README.md                       # 场景、生成来源、人工校核记录
└─ private/                           # 整个目录被 Git 忽略
   ├─ data/<dataset-id>/group.json
   ├─ consent/<dataset-id>/           # 测试群同意记录
   ├─ gold/<dataset-id>/               # 真实消息/事项标注
   ├─ runs/<experiment-id>/<system>/
   │  ├─ profile/                     # 此系统专用的 --data-dir
   │  ├─ analyze.jsonl
   │  ├─ inbox.jsonl
   │  ├─ messages.jsonl
   │  ├─ jev.jsonl
   │  └─ run-manifest.json            # 版本、数据哈希、参数、运行 ID、冷热缓存说明
   └─ results/<experiment-id>/         # 指标 CSV、校准图、待审核汇总
```

- `<system>` 例如 `ours`、`ours-llm`、`b0`；不同系统不共用 profile，也不复用另一实验留下的业务状态。
- `chat-tldr --data-dir .../profile` 指向数据库目录；`chat-tldr-eval score --run .../<system>` 指向包含四份 JSONL 的目录，两者不混用。
- 合成评估的运行产物也放 private/runs，只有经过人工审核、确认只包含聚合结果的材料才复制到 `reports/<交付日期>/`。
- `run-manifest.json` 是实验来源记录，不替代 CLI 事件或数据库。记录配置的非密钥字段/哈希、provider/model、Git 提交与工作区是否有改动、数据/标注哈希；不记录环境变量值。

## 5. 测试、样例与设计素材

| 内容 | 存放位置 | 约定 |
|---|---|---|
| 小型纯函数测试 | 所属 `.rs` 的 `#[cfg(test)]` 模块 | 紧邻实现 |
| crate 集成测试 | 对应 crate 的 `tests/` | 以公开接口验证行为 |
| CLI 子进程测试 | `apps/cli/tests/` | 校验 stdout、stderr、退出码、临时数据目录 |
| 共用 QCE/协议/模型样例 | `fixtures/qce`、`fixtures/jsonl`、`fixtures/models` | 只放合成内容；相关测试直接引用同一份 |
| 只服务一个测试的少量数据 | 测试内或该 crate 的 `tests/data/` | 不复制另一处已有的黄金样例 |
| 合成评估数据与 gold | `eval/synthetic/` | 用于指标与演示，区别于解析器的小样例 |
| GUI 线框图、设计稿 | `docs/ui/` | 设计素材可提交，但不得含真实聊天截图 |
| 程序实际使用的图标、字体 | `apps/gui/assets/` | 随附许可证，编译/打包时引用 |
| HTML 源模板 | `apps/cli/templates/` | 与生成的 HTML 结果分开 |

测试用 `CARGO_MANIFEST_DIR` 定位仓库内的样例，不依赖调用测试时的工作目录。字体、模板、提示词等正式资源应能进入可执行文件或发布包，不能只在开发机绝对路径下可用。

## 6. 清理、备份与发布

| 内容 | 可否重新生成 | 清理规则 |
|---|---|---|
| `target/`、`.codegraph/`、`dist/` | 可以 | 不属于用户业务数据；需要时重建 |
| `cache/qce/downloads/` | 通常可以重新下载 | 由 QCE 管理组件按管理规则清理 |
| `tmp/<component>/<job-id>/` | 任务中间产物 | 任务完成或确认已失效后清理；活跃任务目录保留 |
| `components/qce/<version>/` | 可重新安装 | 确认版本未被运行中的组件使用后处理 |
| 原始导出、数据库、标注、会话状态、用户输出、备份 | 不保证可以恢复 | 不纳入通用缓存清理，不因导入成功或升级而删除 |
| 诊断日志 | 可丢失历史诊断信息 | 与聊天数据分开轮转，使用明确的大小/保留策略 |

所有清理都限制在该组件拥有的明确目录内；不能接受任意递归删除路径，不能跟随链接越过管理根目录。这是后续实现约束，本轮不执行任何清理或数据迁移。

发布产物规划：

```text
dist/<target>/<version>/
├─ chat-tldr[.exe]
├─ chat-tldr-gui[.exe]
├─ chat-tldr-qce-manager[.exe]   # 组件交付后才加入
├─ config.example.toml
├─ README.txt
└─ licenses/
```

发布包不包含开发数据库、配置密钥、原始聊天、实验私有目录和 CodeGraph 索引。数据库迁移只由 engine 按版本执行；迁移与恢复另设验收，不借目录整理重建数据库。

## 7. 目录的 Git 边界与落实顺序

- 提交：源码、Cargo.lock、配置模板、SQL/提示词/HTML 源模板、设计文档、合成 fixtures、合成评估 gold、审核后的报告。
- 忽略：target、dist、.codegraph、private、eval/private、真实 exports、本地 config.toml、数据库及旁文件、密钥、本地日志/临时目录。
- 临时交接 `HANDOFF_CODEX.md` 继续留在本机并被忽略。持续有效的决定写入相应正式文档，不依赖临时交接文件作为唯一来源。

落实顺序：先完成 CLI 契约与 core 协议类型，再做 QCE 导入和 engine 主线；GUI 与 QCE 管理组件按文件/CLI 边界接入。当前没有需要移动的旧业务数据，现有六个 crate 也不搬迁。增加 QCE 管理程序时，再把其 Cargo.toml 纳入同一个 workspace 和 Cargo.lock。
