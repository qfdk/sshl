use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("ssh error: {0}")]
    Ssh(String),
    #[error("sftp error: {0}")]
    Sftp(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("config error: {0}")]
    Config(String),
    /// 用户在主机身份确认框里拒绝了。
    #[error("host key not accepted")]
    HostKeyDeclined,
    /// 用户点了停止（`sftp::cancellable`）。
    #[error("已停止")]
    Cancelled,
    #[error("{0}")]
    Other(String),
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        AppError::Other(e.to_string())
    }
}

impl From<russh::Error> for AppError {
    fn from(e: russh::Error) -> Self {
        AppError::Ssh(e.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
