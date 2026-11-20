use thiserror::Error;

/// Usage, manifest or environment errors. Every message says how to fix the problem.
#[derive(Debug, Error)]
pub enum KeelError {
    #[error("{0}")]
    Usage(String),
    #[error("{file}: {message}")]
    Manifest { file: String, message: String },
    #[error("{0}")]
    Env(String),
}

impl KeelError {
    pub fn exit_code(&self) -> i32 {
        3
    }
}
