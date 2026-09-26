# 仓库协作约定

- 用中文汇报；说明实现范围、实际验证结果和剩余缺口。当前分工以 `docs/TEAM_ASSIGNMENTS.md` 为准。
- 修改接口前读 `docs/CLI_PROTOCOL.md`、`docs/DATA_MODEL.md`；文件归属读 `docs/FILE_LAYOUT.md`。首次 v1 草案已经批准，后续公共契约变更由 Develata 决策。
- 主线由 Codex 编码，Develata 参与设计与审核。同学负责 QCE 管理、GUI 设计和合成验收材料；保留其他人的改动。
- `crates/core` 只放共享类型/协议；QCE 适配器是纯函数。SQLite 只由 `engine::store` 打开；GUI、eval 只依赖 core，通过 CLI 子进程交互。
- Rust stable / edition 2024；模型请求采用同步 blocking 客户端。测试使用合成数据和 mock，不调用真实云服务。
- CLI stdout 是 JSONL，stderr 是诊断；帮助/版本选项允许文本。只有已经实现的命令进入 `version.capabilities`。
- 不提交真实聊天、密钥、数据库、原始导出、`HANDOFF_CODEX.md` 或 `.codegraph/`；不删除用户导出。API key 只从环境变量读取。
- 合并前运行 `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked`。CI job 名保持 `fmt`、`clippy`、`test`。
- 使用现有任务分支与 PR，不 force push、不 rebase。用户对本次初始化、合并和推送的授权已经给出；后续操作按具体任务授权执行。
- Windows 使用 PowerShell；文本检索先 rg，结构检索先用已有 CodeGraph 索引。索引是本地生成物，可在代码变化后 `codegraph sync`。
