use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoworkerError {
    #[error("config: {0}")]
    Config(String),
    #[error("store: {0}")]
    Store(String),
    #[error("workflow: {0}")]
    Workflow(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Other(#[from] anyhow::Error),
    #[cfg(feature = "sqlite")]
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, CoworkerError>;
