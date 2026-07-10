use thiserror::Error;

#[derive(Debug, Error)]
pub enum UeError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid operation: {0}")]
    Invalid(String),
    #[error("clips overlap: {0}")]
    Overlap(String),
    #[error("track locked: {0}")]
    Locked(String),
    #[error("serialization error: {0}")]
    Serde(String),
    #[error("unsupported project version: {0} (max {1})")]
    SchemaVersion(u32, u32),
}

impl From<serde_json::Error> for UeError {
    fn from(e: serde_json::Error) -> Self {
        UeError::Serde(e.to_string())
    }
}

pub type UeResult<T> = Result<T, UeError>;
