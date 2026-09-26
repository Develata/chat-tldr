# 合成 JSONL 合作样例

所有姓名、群、消息和模型结果均为人工构造，不含真实聊天，不是实际模型运行记录。

这些文件描述首次 v1 的合作协议，供 GUI 设计、JSONL 客户端和离线演示使用。`version.jsonl` 的 capabilities 只列当前基础 CLI 的能力；分析和收件箱文件是后续功能的协议样例，不能据此认定相应命令已经实现。

| 文件 | 用途 | 预期进程退出码 |
|---|---|---|
| `version.jsonl` | 版本握手、能力发现 | 0 |
| `chats.jsonl` | 已导入但未分析的合成群 | 0 |
| `inbox-empty.jsonl` | 空收件箱；`view_cursor=null` 禁用标为已读 | 0 |
| `inbox.jsonl` | 已校验的 P0 截止事项，负责人是别人 | 0 |
| `analyze-complete.jsonl` | 进度与统计，正常结束 | 0 |
| `analyze-partial.jsonl` | 可恢复的模型失败，部分结束 | 6 |

每个文件独立表示一次调用：`run_id` 恒定，`seq` 从 0 连续递增，以唯一 `done` 结束。通过 `chat_tldr_core::CliEvent` 解析每行，再用 `EventStreamValidator` 检查流及进程退出码。未知事件仍然计入序号；未知事件和未知枚举值应显示为“其他”或略过，已知事件缺少必需字段必须报协议错误。

`inbox.jsonl` 特意保留 `assignee=other` 的 P0 截止事项，避免 GUI 或规则实现把用户已确认的优先级语义改成“只关心分给我的任务”。证据高亮使用 Unicode scalar 下标，区间为左闭右开。
