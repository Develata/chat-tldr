//! Engine boundary: storage and business logic live here, never in the GUI.

pub mod config;
pub mod render;
pub mod store;

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
            _ => "E_DB",
        }
    }

    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Config(_) => 4,
            Self::ChatNotFound(_) | Self::Input(_) => 3,
            _ => 7,
        }
    }
}
