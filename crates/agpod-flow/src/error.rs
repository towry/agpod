use thiserror::Error;

#[derive(Error, Debug)]
pub enum FlowError {
    #[error(
        "No git remote found. Please configure a remote (e.g., `git remote add origin <url>`)"
    )]
    NoGitRemote,

    #[error("Not in a git repository")]
    NotGitRepo,

    #[error("Git error: {0}")]
    Git(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("Frontmatter missing in document: {path}")]
    FrontmatterMissing { path: String },

    #[error("doc_id missing in document: {path}. Fix with:\n  agpod flow doc init --path {path} --task <task-id> --type <doc-type>")]
    DocIdMissing { path: String },

    #[error("Invalid frontmatter field in {path}: {detail}")]
    InvalidFrontmatter { path: String, detail: String },

    #[error("No active task in session {session_id}. Run:\n  agpod flow -s {session_id} focus --task <task-id>")]
    NoActiveTask { session_id: String },

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Task not found: {0}")]
    TaskNotFound(String),

    #[error("Config error: {0}")]
    Config(String),

    #[error("{0}")]
    Other(String),
}

pub type FlowResult<T> = Result<T, FlowError>;
