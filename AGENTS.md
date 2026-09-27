# 仓库协作约定

- 用中文汇报；说明实现范围、实际验证结果和剩余缺口。当前分工以 `docs/TEAM_ASSIGNMENTS.md` 为准。
- 修改接口前读 `docs/CLI_PROTOCOL.md`、`docs/DATA_MODEL.md`；文件归属读 `docs/FILE_LAYOUT.md`。首次 v1 草案已经批准，后续公共契约变更由 Develata 决策。
- 主线由 Codex 编码，Develata 参与设计与审核。同学负责 QCE 管理、GUI 设计和合成验收材料；保留其他人的改动。
- `crates/core` 只放共享类型/协议；QCE 适配器是纯函数。SQLite 只由 `engine::store` 打开；GUI、eval 只依赖 core，通过 CLI 子进程交互。
- 高内聚、低耦合：按业务职责和数据所有权组织模块，控制器、模型传输、确定性算法和持久化各守边界；通过小而明确的接口协作，避免跨层直连、循环依赖和持续膨胀的单文件。抽取可复用逻辑时保持职责完整，不为拆文件制造无意义的间接层。
- 实现时同时检查算法复杂度、数据库访问和内存分配；避免循环内全表扫描、重复反序列化、重复查找及不必要的正文复制。优先使用合适的索引、批量/增量处理与缓存。性能优化必须保持证据校验、事务、游标与协议语义，用有代表性的测试、查询计划或测量说明依据；未经测量不声称具体提速比例。
- 正确性和用户体验优先于性能。不得为提速绕过验证、削弱错误传播或引入过度耦合；高成本优化先定位瓶颈，再评估收益和维护成本。
- Rust stable / edition 2024；模型请求采用同步 blocking 客户端。测试使用合成数据和 mock，不调用真实云服务。
- CLI stdout 是 JSONL，stderr 是诊断；帮助/版本选项允许文本。只有已经实现的命令进入 `version.capabilities`。
- 不提交真实聊天、密钥、数据库、原始导出、`HANDOFF_CODEX.md` 或 `.codegraph/`；不删除用户导出。按 Develata 2026-09-27 决定，课程阶段允许 API key 明文保存在本地 config；优先于环境变量。GUI 密钥只通过 stdin 交给 CLI，配置查询/日志/Debug 不回显，示例配置不含真实密钥（ADR-0011）。
- 合并前运行 `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked`。CI job 名保持 `fmt`、`clippy`、`test`。
- GitHub CI 按 Cargo 模块并发执行检查与测试，模块间使用 `fail-fast: false`；`clippy`、`test` 保留为稳定汇总检查。任何应跑模块失败、取消或意外跳过都不能被汇总为成功；公共类型、依赖、锁文件和构建配置变更必须覆盖受影响下游，不能用不完整路径筛选漏测。缓存键按模块区分，结果以实际 job 状态为准。
- 使用现有任务分支与 PR，不 force push、不 rebase。用户对本次初始化、合并和推送的授权已经给出；后续操作按具体任务授权执行。
- Windows 使用 PowerShell；文本检索先 rg，结构检索先用已有 CodeGraph 索引。索引是本地生成物，可在代码变化后 `codegraph sync`。
