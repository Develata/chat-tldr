# 合成验收场景

同学 C 将每个独立场景放在 `<场景名>/` 下，至少包含 QCE 单文件 `group.json` 和写明预期、证据来源与人工校核记录的 `README.md`。

格式先参考 [`fixtures/qce/synthetic-group.json`](../../fixtures/qce/synthetic-group.json)，这是解析接口样例。此目录用于较完整的评估/演示场景，不复制同一个小 fixture 来形成多份权威数据。正式 gold 格式见 [EVALUATION](../../docs/EVALUATION.md)，任务验收见 [TEAM_ASSIGNMENTS](../../docs/TEAM_ASSIGNMENTS.md)。

只提交完全合成的聊天；真实样本、同意记录、私有标注与运行产物放 `eval/private/`。期望结果不能写成实测指标。
