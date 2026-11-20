use thiserror::Error;

/// Usage, manifest or environment errors. Every message says how to fix the problem.
#[derive(Debug, Error)]
pub enum KeelError {
    #[error("{0}")]
    Usage(String),
    /// A usage error whose fix is a command line (shown in the JSON `error.fix`).
    #[error("{message}")]
    Fix { message: String, fix: String },
    #[error("{file}: {message}")]
    Manifest { file: String, message: String },
    #[error("{0}")]
    Env(String),
}

impl KeelError {
    /// How to fix it, when the error knows a specific command.
    pub fn fix(&self) -> Option<&str> {
        match self {
            KeelError::Fix { fix, .. } => Some(fix),
            _ => None,
        }
    }

    pub fn exit_code(&self) -> i32 {
        3
    }
}
