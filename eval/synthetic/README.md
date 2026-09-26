# 合成验收场景

同学 C 将每个独立场景放在 `<场景名>/` 下，至少包含 QCE 单文件 `group.json` 和写明预期、证据来源与人工校核记录的 `README.md`。

当前本目录尚未收到独立人工标注/gold。Codex 已在共用 fixtures 中补齐 [100 条主样本与 4 条回填样本](../../fixtures/qce/scenario-analysis.README.md)，覆盖 @、身份歧义、截止事项、冷热话题、撤回、媒体、交错与回填等。先从这些原文独立写期望，再对照系统输出；Mock 响应和回归断言不能直接当作独立 gold。

格式先参考 [`fixtures/qce/synthetic-group.json`](../../fixtures/qce/synthetic-group.json)，这是解析接口样例。此目录用于较完整的评估/演示场景，不复制同一个小 fixture 来形成多份权威数据。正式 gold 格式见 [EVALUATION](../../docs/EVALUATION.md)，任务验收见 [TEAM_ASSIGNMENTS](../../docs/TEAM_ASSIGNMENTS.md)。

复用现有输入时，在场景说明中引用路径和消息 ID；只有新增输入才需要新的 `group.json`。已实现的标注导入导出和 Ours 部分指标用法见 [eval/README](../README.md)。更正/取消、待回应、话题与校准等尚未覆盖的能力，先记录人工期望和实际缺口，不填写虚构分数。演示与验收报告按分工放在 `reports/<日期>/`，目录随实际交付创建。

只提交完全合成的聊天；真实样本、同意记录、私有标注与运行产物放 `eval/private/`。期望结果不能写成实测指标。
