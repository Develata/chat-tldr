# ADR-0002 全栈 Rust

**背景**：课程要求团队统一技术栈；团队成员的 Rust 水平不一。

**决策**：core、engine、qce、CLI、GUI、eval 全部用 Rust，放在同一个 Cargo workspace 中。

**理由**
- 一个 workspace、一套 CI（fmt、clippy、test）、一份 `Cargo.lock`，协作流程最简单。
- 共享类型（`crates/core`）由编译器检查：GUI、eval 与 CLI 的协议不一致时编译就会失败，不用等到运行时。
- 单个二进制分发，演示机器不需要装 Python 或 Node 运行时。
- 所需生态（rusqlite、reqwest、egui、jieba-rs、minijinja）都足够成熟。

**代价**
- 学习成本：通过严格的模块边界缓解。同学 A 的模块都是纯函数，同学 B 只依赖 `core`，同学 C 的 eval 主要是数值计算。
- 编译较慢：CI 使用 rust-cache。
