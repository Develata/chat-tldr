# ADR-0003 CLI 拥有全部逻辑，GUI 通过子进程调用

**背景**：需要一个图形界面用于演示，同时需要一个可脚本化、可评估的入口。

**决策**：`apps/cli` 拥有全部业务逻辑和唯一的数据库访问权。GUI 启动 `chat-tldr` 子进程，逐行解析 stdout 上的 JSON Lines（协议见 CLI_PROTOCOL）。GUI 只依赖 `crates/core`。

**理由**
- 单一事实来源：数据库只有一个写入者，不存在 GUI 与 CLI 并发写入同一份数据的问题。
- 并行开发：同学 B 用 `fixtures/jsonl/` 里的 mock 输出就能开发 GUI，不用等 engine 完成。
- 可评估：eval 与 GUI 走同一个协议，评估的就是用户实际看到的结果。
- 演示保底：GUI 出问题时可以改用 `--html`。
- 依赖隔离：GUI 不会把 rusqlite、reqwest 等依赖拉进来；Cargo 依赖图保证了“GUI 不碰数据库”。

**代价**
- 每次查询都要启动一个进程（约几十毫秒），对演示来说足够。
- 协议需要版本管理：采用 `schema_version` MAJOR.MINOR 规则，GUI 启动时先握手。
