#![forbid(unsafe_code)]

use clap::Parser;
use clap::error::ErrorKind;
use keel::{Cli, render_json, render_json_error, render_text};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            let code = match err.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => 0,
                _ => 3,
            };
            let _ = err.print();
            if code == 3
                && args
                    .iter()
                    .take_while(|a| *a != "--")
                    .any(|a| a == "--json")
            {
                let command = command_name(&args);
                let message = err.to_string();
                let fix = format!("run `keel {command} --help`").replace("  ", " ");
                println!(
                    "{}",
                    render_json_error(&command, message.trim_end(), fix.trim_end())
                );
            }
            return ExitCode::from(code);
        }
    };
    let json = cli.json;
    let command = cli.command.name();
    match keel::run(cli) {
        Ok(report) => {
            if json {
                println!("{}", render_json(&report));
            } else {
                print!("{}", render_text(&report));
            }
            ExitCode::from(report.exit_code.clamp(0, 255) as u8)
        }
        Err(err) => {
            eprintln!("error: {err}");
            if json {
                println!(
                    "{}",
                    render_json_error(
                        command,
                        &err.to_string(),
                        "the message says what to change; see `keel --help`"
                    )
                );
            }
            ExitCode::from(err.exit_code() as u8)
        }
    }
}

/// The subcommand word of the command line, skipping flags and the value of `-C`.
fn command_name(args: &[String]) -> String {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "-C" {
            it.next();
        } else if a == "--" {
            break;
        } else if !a.starts_with('-') {
            return if ["up", "plan", "doctor"].contains(&a.as_str()) {
                a.clone()
            } else {
                String::new()
            };
        }
    }
    String::new()
}
