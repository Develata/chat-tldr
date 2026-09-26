//! Engine boundary: storage and business logic live here, never in the GUI.

pub mod agent;
pub mod config;
pub mod decider;
pub mod extract;
pub mod llm;
pub mod rank;
pub mod render;
mod segment;
pub mod store;
pub mod temporal;
pub mod verify;

pub use config::Config;
pub type Result<T> = std::result::Result<T, EngineError>;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("invalid configuration: {0}")]
    Config(String),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("database format error: {0}")]
    DatabaseFormat(String),
    #[error("database file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("stored data is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("chat not found: {0}")]
    ChatNotFound(String),
    #[error("invalid input: {0}")]
    Input(String),
    #[error("insight not found: {0}")]
    InsightNotFound(String),
    #[error("invalid cursor: {0}")]
    CursorInvalid(String),
    #[error("analysis already running for chat {0}")]
    RunInProgress(String),
    #[error("{0}")]
    Provider(#[from] llm::ProviderError),
    #[error("invalid arguments: {0}")]
    Usage(String),
    #[error("estimated request cost exceeds the remaining budget")]
    BudgetExceeded,
    #[error("analysis cancelled")]
    Cancelled,
}

impl EngineError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Config(_) => "E_CONFIG",
            Self::Database(rusqlite::Error::SqliteFailure(e, _))
                if matches!(
                    e.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                ) =>
            {
                "E_DB_BUSY"
            }
            Self::ChatNotFound(_) => "E_CHAT_NOT_FOUND",
            Self::Input(_) => "E_INPUT_PARSE",
            Self::InsightNotFound(_) => "E_INSIGHT_NOT_FOUND",
            Self::CursorInvalid(_) => "E_CURSOR_INVALID",
            Self::RunInProgress(_) => "E_RUN_IN_PROGRESS",
            Self::Provider(e) => e.code(),
            Self::Usage(_) => "E_USAGE",
            Self::BudgetExceeded => "E_BUDGET_EXCEEDED",
            Self::Cancelled => "E_CANCELLED",
            _ => "E_DB",
        }
    }

    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Config(_) => 4,
            Self::Usage(_) => 2,
            Self::BudgetExceeded => 6,
            Self::Cancelled => 130,
            Self::Provider(e) => e.exit_code(),
            Self::InsightNotFound(_) | Self::CursorInvalid(_) => 3,
            Self::ChatNotFound(_) | Self::Input(_) => 3,
            _ => 7,
        }
    }
}
