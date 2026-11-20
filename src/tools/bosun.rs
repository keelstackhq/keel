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
        3 if mentions_existing_branch(out) => Res::Human {
            title: format!(
                "bosun {pin} cannot init on an existing branch; upgrade bosun to a release \
                 where init reuses the current branch"
            ),
            command: "npm install -g @keelstackhq/bosun@latest".into(),
        },
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

/// bosun 0.21.0 exits 3 when `init --branch B` finds `B` already existing.
fn mentions_existing_branch(out: &Out) -> bool {
    let msg = json(out)
        .map(|v| v["error"]["message"].as_str().unwrap_or("").to_string())
        .unwrap_or_default();
    let all = format!("{msg} {} {}", out.stdout, out.stderr).to_lowercase();
    all.contains("branch") && all.contains("exist")
}

/// Epics and tasks `plan import --dry-run` would create (`data.epics[]`, `data.tasks[]`,
/// `action: "would-create"`).
pub fn would_create(data: &Value) -> usize {
    ["epics", "tasks"]
        .iter()
        .filter_map(|k| data[*k].as_array())
        .flatten()
        .filter(|i| i["action"] == "would-create")
        .count()
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

/// `bosun --json setup-snippet`
pub fn setup_snippet_cmd(dir: &Path) -> Cmd {
    Cmd::new("bosun", &["--json", "setup-snippet"]).dir(dir)
}

/// `bosun --json doctor --remote [--all]`
pub fn doctor_cmd(dir: &Path, all: bool) -> Cmd {
    let mut a = vec!["--json", "doctor", "--remote"];
    if all {
        a.push("--all");
    }
    Cmd::new("bosun", &a).dir(dir)
}

/// What `bosun doctor --remote` found.
#[derive(Debug, PartialEq, Eq)]
pub enum Doctor {
    Clean,
    /// Missing items, each `id: fix` when bosun gives a fix.
    Missing(Vec<String>),
    /// Exit 5 with `data.offline`: no `GH_TOKEN`/`GITHUB_TOKEN` in the environment.
    Offline,
}

fn mentions_all(o: &Out) -> bool {
    let msg = json(o)
        .map(|v| v["error"]["message"].as_str().unwrap_or("").to_string())
        .unwrap_or_default();
    format!("{} {} {msg}", o.stderr, o.stdout).contains("--all")
}

/// `doctor --remote --all`, retried without `--all` only on exit 2 that mentions `--all`.
pub fn doctor(r: &dyn Runner, dir: &Path) -> Result<Doctor, String> {
    let mut c = doctor_cmd(dir, true);
    let mut o = r.run(&c);
    if o.code == 2 && mentions_all(&o) {
        c = doctor_cmd(dir, false);
        o = r.run(&c);
    }
    let v = json(&o);
    if o.code == 5 && v.as_ref().is_some_and(|v| v["data"]["offline"] == true) {
        return Ok(Doctor::Offline);
    }
    let items = v.as_ref().and_then(|v| v["data"]["items"].as_array());
    let missing: Vec<String> = items
        .map(|a| {
            a.iter()
                .filter(|i| i["status"] == "missing")
                .map(|i| {
                    let name = i["id"].as_str().or(i["name"].as_str()).unwrap_or("item");
                    match i["fix"].as_str().or(i["detail"].as_str()) {
                        Some(d) if !d.is_empty() => format!("{name}: {d}"),
                        _ => name.to_string(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let count = v
        .as_ref()
        .and_then(|v| v["data"]["missing"].as_u64())
        .unwrap_or(missing.len() as u64);
    if count > 0 {
        return Ok(Doctor::Missing(if missing.is_empty() {
            vec![format!("{count} item(s)")]
        } else {
            missing
        }));
    }
    if o.code != 0 {
        return Err(failure(&c, &o));
    }
    if v.is_none() {
        return Err(not_json(&c));
    }
    Ok(Doctor::Clean)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::FakeRunner;

    fn cmd() -> Cmd {
        Cmd::new("bosun", &["--json", "init"])
    }

    #[test]
    fn init_on_an_existing_branch_is_an_upgrade_human_step() {
        let o = Out::with(
            3,
            r#"{"ok":false,"exit_code":3,"error":{"kind":"conflict","message":"branch keel/bootstrap already exists"}}"#,
            "",
        );
        let Res::Human { title, command } = interpret(&cmd(), &o, "0.21.0") else {
            panic!()
        };
        assert_eq!(
            title,
            "bosun 0.21.0 cannot init on an existing branch; upgrade bosun to a release where init reuses the current branch"
        );
        assert_eq!(command, "npm install -g @keelstackhq/bosun@latest");
        // Another exit 3 stays a failure.
        assert!(matches!(
            interpret(
                &cmd(),
                &Out::with(3, "", "conflict in owners.toml"),
                "0.21.0"
            ),
            Res::Failed(_)
        ));
    }

    #[test]
    fn counts_would_create_epics_and_tasks() {
        let d = serde_json::json!({"epics":[{"action":"would-create"},{"action":"unchanged"}],
            "tasks":[{"action":"would-create"},{"action":"would-create"}]});
        assert_eq!(would_create(&d), 3);
        assert_eq!(would_create(&serde_json::json!({"epics":[],"tasks":[]})), 0);
        assert_eq!(would_create(&Value::Null), 0);
    }

    #[test]
    fn all_is_retried_only_when_exit_2_mentions_it() {
        let clean = Out::ok(r#"{"ok":true,"data":{"items":[],"missing":0}}"#);
        let r = FakeRunner::new()
            .on(
                &["bosun", "--json", "doctor", "--remote", "--all"],
                Out::with(2, "", "error: unexpected argument '--all'"),
            )
            .on(&["bosun", "--json", "doctor", "--remote"], clean.clone());
        assert_eq!(doctor(&r, Path::new(".")).unwrap(), Doctor::Clean);
        assert_eq!(r.calls().len(), 2);
        // An exit 2 from something else is a failure, with no retry.
        let r = FakeRunner::new()
            .on(
                &["bosun", "--json", "doctor", "--remote", "--all"],
                Out::with(2, "", "bad usage"),
            )
            .on(&["bosun", "--json", "doctor", "--remote"], clean);
        assert!(doctor(&r, Path::new(".")).is_err());
        assert_eq!(r.calls().len(), 1);
    }

    #[test]
    fn offline_exit_5_is_detected() {
        let r = FakeRunner::new().on(
            &["bosun"],
            Out::with(
                5,
                r#"{"ok":false,"exit_code":5,"data":{"offline":true,"missing":0}}"#,
                "",
            ),
        );
        assert_eq!(doctor(&r, Path::new(".")).unwrap(), Doctor::Offline);
    }
}
