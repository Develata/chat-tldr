# 文档导航

| 想了解什么 | 文档 | 当前定位 |
|---|---|---|
| 当前分工、接入方法与验收 | [TEAM_ASSIGNMENTS.md](TEAM_ASSIGNMENTS.md) | 当前唯一分工依据；A/B/C 是待绑定的任务席位 |
| 文件放哪里、谁读写、哪些可清理 | [FILE_LAYOUT.md](FILE_LAYOUT.md) | 目录与存储设计；规划目录按实现逐步建立 |
| 模块边界与依赖 | [ARCHITECTURE.md](ARCHITECTURE.md) | 架构依据 |
| CLI 当前字段与事件 | [CLI_PROTOCOL.md](CLI_PROTOCOL.md) | 现有协议规格 |
| CLI 改进的决策记录 | [CLI_V1_REVIEW.md](CLI_V1_REVIEW.md) | 用户已采纳；正式定义同步 CLI_PROTOCOL |
| 共享类型与数据库 | [DATA_MODEL.md](DATA_MODEL.md) | 类型和表结构规格 |
| 算法与状态流转 | [PIPELINE.md](PIPELINE.md) | 处理规则；控制器与模型阶段仍待实现 |
| 如何评估 | [EVALUATION.md](EVALUATION.md) | 指标、标注和实验流程 |
| 实际期限与分工 | [ROADMAP.md](ROADMAP.md) | 顶部“当前安排”优先于旧的相对日程 |
| 为什么这样设计 | [decisions/README.md](decisions/README.md) | 架构决策记录 |
| 尚未确定的事项 | [OPEN_QUESTIONS.md](OPEN_QUESTIONS.md) | 待确认或待实测的事项 |

`tasks/` 保存模块说明和验收要求；早期 A/B/C 分工已由 TEAM_ASSIGNMENTS 替代。QCE JSON 适配器已归入 Codex 主线，QCE 管理组件的交接见 [apps/qce-manager/README.md](../apps/qce-manager/README.md)。

正式决定落到对应主题文档。协议描述目标能力；当前二进制可调用的能力以 `chat-tldr version` 返回值为准。共享样例可用于界面设计，但不能证明对应业务命令已实现。
