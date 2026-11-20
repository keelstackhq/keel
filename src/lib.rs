#![forbid(unsafe_code)]

pub mod cli;
pub mod error;
pub mod report;

pub use cli::{Cli, Command};
pub use error::KeelError;
pub use report::{render_json, render_json_error, render_text};

/// Run a parsed command line. Stubbed until the steps are wired.
pub fn run(cli: Cli) -> Result<report::Report, KeelError> {
    Err(KeelError::Usage(format!(
        "`keel {}` is not implemented yet",
        cli.command.name()
    )))
}
