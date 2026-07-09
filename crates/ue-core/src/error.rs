use thiserror::Error;

#[derive(Debug, Error)]
pub enum UeError {
    #[error("no encontrado: {0}")]
    NotFound(String),
    #[error("operación inválida: {0}")]
    Invalid(String),
    #[error("los clips se solapan: {0}")]
    Overlap(String),
    #[error("pista bloqueada: {0}")]
    Locked(String),
    #[error("error de serialización: {0}")]
    Serde(String),
    #[error("versión de proyecto no soportada: {0} (máx {1})")]
    SchemaVersion(u32, u32),
}

impl From<serde_json::Error> for UeError {
    fn from(e: serde_json::Error) -> Self {
        UeError::Serde(e.to_string())
    }
}

pub type UeResult<T> = Result<T, UeError>;
