# ADR-0005 Jev + LLM + embedding 分工，统一 Decider 抽象

**背景**：需要做三类事：在给定选项中判断（归属、分类、下一步动作），生成文本（标题、摘要、结论），以及可选的相似度粗筛。

**决策**
- **Jev**（TypeSafe，`POST /v1/systemone`）：所有“在给定选项中选择 / 是否 / 打分”的判断。每个 burst 一次请求，把多个问题合并在一起。固定使用版本 `jev-1.13.0`。
- **普通 LLM**：标题、摘要、结论抽取、截止日期原文、中等置信度时的归属复核。
- **Embedding（可选）**：只在活跃话题过多时粗筛候选。
- **Rust 规则**：@我、日期计算、计数、证据校验、分层。
- Jev 与 LLM 的判断能力统一抽象为 `Decider` trait，实现为 `JevDecider` / `LlmDecider` / `MockDecider`。

**理由**
- Jev 不生成文本，直接返回校准过的概率分布，适合做阈值判断和校准评估；只按输入 token 计费（0.042 美元 / 百万 token），成本很低。
- 官方文档明确说明 Jev 不擅长数字、日期比较和生成，因此这些工作放在代码或 LLM 中。
- `Decider` 抽象带来三个好处：Jev 不可用时降级；做 Ours vs Ours-LLM 的消融实验；测试时注入 Mock。

**代价**
- 同时依赖两个云服务，聊天文本会离开本机。决定不做脱敏（Q-DEC-2）；缓解：图片不上传，在 README 与 GUI 中明确告知，演示只用合成数据或征得同意的测试群。
- 所有提示词（Jev 的 instructions/criteria、LLM 的模板）一律用英文，聊天内容保持原文（Q-JEV-2）。LLM 统一使用 DeepSeek `deepseek-flash`，关闭 thinking 模式（Q-LLM-1）。
- Jev 的中文能力需要验证。见 ADR-0004 的缓解措施。
