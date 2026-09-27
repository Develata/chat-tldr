pub type Result<T> = std::result::Result<T, Failure>;

#[derive(Debug)]
pub struct Failure {
    pub code: &'static str,
    pub exit: u8,
    pub message: String,
}

impl Failure {
    pub fn new(code: &'static str, exit: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            exit,
            message: message.into(),
        }
    }

    pub fn usage(message: &'static str) -> Self {
        Self::new("E_USAGE", 2, message)
    }

    pub fn io() -> Self {
        Self::new(
            "E_OUTPUT_WRITE",
            8,
            "无法安全读写组件目录；请检查权限、空间及路径链接",
        )
    }

    pub fn protocol() -> Self {
        Self::new(
            "E_QCE_RESPONSE",
            3,
            "服务响应不符合预期结构；未回显响应内容",
        )
    }
}
