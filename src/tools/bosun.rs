use super::{failure, json, not_json};
use crate::exec::{Cmd, Out, Runner};
use serde_json::Value;
use std::path::Path;

/// The labels bosun needs, for the fallback when `bosun init` reports no `data.labels`.
/// Source: bosun `src/cli/adopt.rs` `labels_wanted()`: `bosun:halt`, `registry::LABEL`
/// (`src/registry.rs`: `bosun:registry`) and every `State::ALL` label
/// (`src/core/state.rs`: `bosun:state/<name>`).
pub const LABELS: [&str; 12] = [
    "bosun:halt",
    "bosun:registry",
    "bosun:state/planned",
    "bosun:state/ready",
    "bosun:state/claimed",
    "bosun:state/building",
    "bosun:state/review",
    "bosun:state/fixing",
    "bosun:state/queued",
    "bosun:state/merged",
    "bosun:state/blocked",
    "bosun:state/needs-human",
];

pub enum Res {
    /// Exit 0 with a JSON envelope: its `data`.
    Ok(Value),
    /// Exit 5 (needs a human), or exit 2 (old bosun) with the upgrade command.
    Human {
        title: String,
        command: String,
    },
    Failed(String),
}

/// Map a bosun result (§5.2). `pin` is the pinned version, for the old-bosun message.
pub fn interpret(cmd: &Cmd, out: &Out, pin: &str) -> Res {
    match out.code {
        0 => match json(out) {
            Some(v) => Res::Ok(v.get("data").cloned().unwrap_or(Value::Null)),
            None => Res::Failed(not_json(cmd)),
        },
        2 => {
            let npm = format!("npm install -g @keelstackhq/bosun@{pin}");
            Res::Human {
                title: format!(
                    "Upgrade bosun to the pinned version {pin} (this one lacks a flag keel uses): {npm}"
                ),
                command: npm,
            }
        }
        5 => {
            let e = json(out).map(|v| v["error"].clone()).unwrap_or(Value::Null);
            let title = ["fix", "message"]
                .iter()
                .find_map(|k| e[*k].as_str().filter(|s| !s.is_empty()))
                .unwrap_or("bosun needs a human decision; run it by hand to see which")
                .to_string();
            Res::Human {
                title,
                command: "keel up".into(),
            }
        }
        _ => Res::Failed(failure(cmd, out)),
    }
}

pub fn run(r: &dyn Runner, cmd: &Cmd, pin: &str) -> Res {
    interpret(cmd, &r.run(cmd), pin)
}

/// `bosun --json init --owner O --branch B --no-pr [--stack rust]`
pub fn init_cmd(dir: &Path, owner: &str, branch: &str, rust: bool) -> Cmd {
    let mut a = vec![
        "--json", "init", "--owner", owner, "--branch", branch, "--no-pr",
    ];
    if rust {
        a.extend(["--stack", "rust"]);
    }
    Cmd::new("bosun", &a).dir(dir)
}

pub fn plan_import_cmd(dir: &Path, file: &str, dry_run: bool) -> Cmd {
    let mut a = vec!["--json", "plan", "import", file];
    if dry_run {
        a.push("--dry-run");
    }
    Cmd::new("bosun", &a).dir(dir)
}
