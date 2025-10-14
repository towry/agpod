use thiserror::Error;

#[derive(Error, Debug)]
pub enum KiloError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Template error: {0}")]
    Template(String),

    #[error("Plugin error: {0}")]
    #[allow(dead_code)]
    Plugin(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Directory already exists: {0}")]
    DirectoryExists(String),

    #[error("Template not found: {0}")]
    TemplateNotFound(String),

    #[error("Git error: {0}")]
    Git(String),
}

pub type KiloResult<T> = Result<T, KiloError>;
