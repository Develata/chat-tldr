# ADR-0011：CLI/GUI 模型设置与本地凭据

状态：已接受。2026-09-27，Develata 在遇到 GUI 的 E_CONFIG 后要求能在面板/CLI 填写 provider 格式、地址、模型与 API key，并明确将初始的加密保存要求调整为“先明文存入 config，课程作业先完成闭环”。

配置写入由 CLI 承担，GUI 仍只依赖 core，通过 JSONL 子进程交互。`config show` 返回不含密钥的有效配置；`config set llm|jev` 修改一个 provider，并原子替换本地配置文件。四个冻结业务类型、SQLite 与分析规则不变。LLM 支持 OpenAI Chat Completions / Anthropic Messages；Jev 保持 SystemOne。

本地 `api_key` 优先于环境变量，供 GUI 重启后直接使用；未保存密钥时继续按 `api_key_env` 查找，兼容 Docker/CI。密钥通过隐藏终端输入、stdin 或已有环境变量迁入，GUI 使用有长度限制的 stdin JSON，不进入 argv、Debug、输出事件或 GUI 偏好文件。`config.toml` 保存明文，输入遮罩不代表加密。示例配置、仓库、测试回执不包含真实密钥。

文件锁防止 CLI 写入竞争，revision 防止 GUI 的旧草稿覆盖较新配置。更改已保存凭据对应的地址或格式时，需要新 key 或显式清除；旧服务的额外请求参数可用空对象清除。保存失败保留原配置，密钥留空保留已有值，删除密钥明确恢复环境变量查找。不同 provider 可分别保存，避免一次失败产生半套配置。

本次不引入系统凭据库或主密码机制。后续若恢复加密需求，用新 ADR 定义迁移及无桌面环境行为；不把当前实现描述成加密存储。
