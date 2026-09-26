# 文档导航

> 2026-09-26：主线已实现 QCE 单文件导入、当前 Ours 分析闭环、话题合并/关闭/回填、六个总览、CLI/GUI/HTML 和部分离线评估。401 项 Rust 回归通过；真实云模型质量、独立人工 gold、同学外围交付仍待完成。功能入口见 [项目 README](../README.md)，责任与状态见 [TEAM_ASSIGNMENTS](TEAM_ASSIGNMENTS.md)。

| 想了解什么 | 文档 | 当前定位 |
|---|---|---|
| 当前分工、接入方法与验收 | [TEAM_ASSIGNMENTS.md](TEAM_ASSIGNMENTS.md) | 当前唯一分工依据；A/B/C 是待绑定的任务席位 |
| 执行输入、云服务及平台验收 | [ACCEPTANCE.md](ACCEPTANCE.md) | 独立数据目录、离线验收脚本、真实联调及结果边界 |
| 本地模型密钥与启动 | [ENVIRONMENT.md](ENVIRONMENT.md) | `.env` 模板、子进程环境及离线 doctor |
| 文件放哪里、谁读写、哪些可清理 | [FILE_LAYOUT.md](FILE_LAYOUT.md) | 目录与存储设计；规划目录按实现逐步建立 |
| 模块边界与依赖 | [ARCHITECTURE.md](ARCHITECTURE.md) | 架构依据 |
| CLI 当前字段与事件 | [CLI_PROTOCOL.md](CLI_PROTOCOL.md) | 现有协议规格 |
| CLI 改进的决策记录 | [CLI_V1_REVIEW.md](CLI_V1_REVIEW.md) | 用户已采纳；正式定义同步 CLI_PROTOCOL |
| 共享类型与数据库 | [DATA_MODEL.md](DATA_MODEL.md) | 类型和表结构规格 |
| 算法与状态流转 | [PIPELINE.md](PIPELINE.md)、[ANALYSIS_EXECUTION.md](ANALYSIS_EXECUTION.md) | 已实现基础控制循环与模型阶段；完整策略缺口见执行说明 |
| 热门、优先、相关、截止、未读、资料 | [ANALYSIS_VIEWS.md](ANALYSIS_VIEWS.md) | 六个总览的统计口径、窗口、已读边界和剩余语义功能 |
| GUI 使用与已验证范围 | [GUI 使用说明](../apps/gui/README.md)、[GUI_VERIFICATION.md](GUI_VERIFICATION.md)、[REVIEW_FIXES.md](REVIEW_FIXES.md) | 原生收件箱截图与后续总览回归分开记录 |
| 多场景输入与人工验收起点 | [合成样本说明](../fixtures/qce/scenario-analysis.README.md)、[eval 使用说明](../eval/README.md) | 100 条主样本 + 4 条回填；工具回归不等于独立 gold 或真实评分 |
| CI 与依赖更新 | [CONTRIBUTING](../CONTRIBUTING.md#ci-依赖维护) | Node.js 24 Actions、Ubuntu 26.04、模块并行、完整 SHA 与更新 PR |
| 如何评估 | [EVALUATION.md](EVALUATION.md) | 指标、标注和实验流程 |
| 实际期限与分工 | [ROADMAP.md](ROADMAP.md) | 顶部“当前安排”优先于旧的相对日程 |
| 为什么这样设计 | [decisions/README.md](decisions/README.md) | 架构决策记录 |
| 尚未确定的事项 | [OPEN_QUESTIONS.md](OPEN_QUESTIONS.md) | 待确认或待实测的事项 |

`tasks/` 保存早期模块规格和验收目标，其历史清单不是当前同学待办或完成台账；分工以 TEAM_ASSIGNMENTS 为准。QCE 适配器、GUI、eval 已由 Codex 实现相应主线功能。同学的入口分别为 [QCE 管理组件](../apps/qce-manager/README.md)、[GUI 设计复核](ui/README.md) 和 [人工验收材料](../eval/synthetic/README.md)。

正式决定落到对应主题文档。协议描述目标能力；当前二进制可调用的能力以 `chat-tldr version` 返回值为准。共享样例可用于界面设计，但不能证明对应业务命令已实现。
