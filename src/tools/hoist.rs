use super::{failure, json, not_json};
use crate::exec::{Cmd, Out, Runner};
use serde_json::Value;
use std::path::Path;

pub enum Res {
    Ok,
    /// Drift (exit 1) or pending human steps (exit 2) as reported by `check`.
    Drift,
    /// The first of `human_steps` (exit 2).
    Human {
        title: String,
        url: String,
        /// `gh variable set …` from the matching item's reason, else `keel up`.
        command: String,
    },
    Failed(String),
    /// `--only` is not understood: this hoist predates the section.
    Upgrade,
}

fn envelope(cmd: &Cmd, out: &Out) -> Result<Value, String> {
    json(out).ok_or_else(|| not_json(cmd))
}

/// For a human step `variable:<NAME>`: the `gh variable set …` text of the item
/// `{kind:"variable", key:<NAME>}`'s `reason`.
fn variable_command(v: &Value, human: &Value) -> Option<String> {
    let name = human["id"].as_str()?.strip_prefix("variable:")?;
    let item = v["items"]
        .as_array()?
        .iter()
        .find(|i| i["kind"] == "variable" && i["key"].as_str() == Some(name))?;
    let reason = item["reason"].as_str()?;
    let at = reason.find("gh variable set")?;
    Some(reason[at..].trim().to_string())
}

/// An old hoist answers `--only` with a usage error (exit 2, no hoist JSON).
fn old_hoist(o: &Out) -> bool {
    o.code == 2
        && json(o).is_none_or(|v| v.get("human_steps").is_none() && v.get("items").is_none())
}

fn only_args<'a>(base: &[&'a str], only: Option<&'a str>) -> Vec<&'a str> {
    let mut a = base.to_vec();
    if let Some(s) = only {
        a.extend(["--only", s]);
    }
    a
}

pub fn check(r: &dyn Runner, dir: &Path) -> Res {
    check_only(r, dir, None)
}

/// `hoist --json check [--only SECTION]`.
pub fn check_only(r: &dyn Runner, dir: &Path, only: Option<&str>) -> Res {
    let c = Cmd::new("hoist", &only_args(&["--json", "check"], only)).dir(dir);
    let o = r.run(&c);
    if only.is_some() && old_hoist(&o) {
        return Res::Upgrade;
    }
    match o.code {
        0..=2 => match envelope(&c, &o) {
            Ok(_) if o.code == 0 => Res::Ok,
            Ok(_) => Res::Drift,
            Err(e) => Res::Failed(e),
        },
        _ => Res::Failed(failure(&c, &o)),
    }
}

/// `hoist --json apply --yes` (hoist needs `--yes` without a TTY).
pub fn apply(r: &dyn Runner, dir: &Path) -> Res {
    apply_only(r, dir, None)
}

/// `hoist --json apply --yes [--only SECTION]`.
pub fn apply_only(r: &dyn Runner, dir: &Path, only: Option<&str>) -> Res {
    let c = Cmd::new("hoist", &only_args(&["--json", "apply", "--yes"], only)).dir(dir);
    let o = r.run(&c);
    if only.is_some() && old_hoist(&o) {
        return Res::Upgrade;
    }
    match o.code {
        0 => match envelope(&c, &o) {
            Ok(_) => Res::Ok,
            Err(e) => Res::Failed(e),
        },
        2 => match envelope(&c, &o) {
            Ok(v) => {
                let h = &v["human_steps"][0];
                match h["title"].as_str() {
                    Some(t) => Res::Human {
                        title: t.to_string(),
                        url: h["url"].as_str().unwrap_or("").to_string(),
                        command: variable_command(&v, h).unwrap_or_else(|| "keel up".into()),
                    },
                    None => Res::Failed(format!(
                        "`{}` exited 2 without a human step; run it by hand",
                        c.line()
                    )),
                }
            }
            Err(e) => Res::Failed(e),
        },
        _ => Res::Failed(failure(&c, &o)),
    }
}

pub const FILE: &str = "hoist.toml";

/// Whether `hoist.toml` already has the `[header]` table.
pub fn has_section(dir: &Path, header: &str) -> bool {
    std::fs::read_to_string(dir.join(FILE))
        .is_ok_and(|t| t.lines().any(|l| l.trim() == format!("[{header}]")))
}

/// Append `text` (a whole table) to `hoist.toml`, creating the file when it is missing.
pub fn append_section(dir: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    let path = dir.join(FILE);
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    let sep = if have.is_empty() || have.ends_with("\n\n") {
        ""
    } else if have.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| write!(f, "{sep}{text}"))
        .map_err(|e| format!("cannot write {FILE}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::FakeRunner;

    #[test]
    fn only_adds_the_flag_and_old_hoist_means_upgrade() {
        let r = FakeRunner::new().on(
            &["hoist"],
            Out::with(2, "", "error: unexpected argument '--only'"),
        );
        assert!(matches!(
            check_only(&r, Path::new("."), Some("cloudflare.dns")),
            Res::Upgrade
        ));
        assert!(matches!(
            apply_only(&r, Path::new("."), Some("cloudflare.dns")),
            Res::Upgrade
        ));
        assert_eq!(
            r.calls()[0].line(),
            "hoist --json check --only cloudflare.dns"
        );
        assert_eq!(
            r.calls()[1].line(),
            "hoist --json apply --yes --only cloudflare.dns"
        );
        // Exit 2 with hoist JSON is a human step, not an old hoist.
        let r = FakeRunner::new().on(
            &["hoist"],
            Out::with(2, r#"{"items":[],"human_steps":[]}"#, ""),
        );
        assert!(matches!(
            check_only(&r, Path::new("."), Some("x")),
            Res::Drift
        ));
    }

    #[test]
    fn appends_a_section_once_and_keeps_the_file() {
        let d = tempfile::tempdir().unwrap();
        assert!(!has_section(d.path(), "cloudflare.dns"));
        std::fs::write(d.path().join(FILE), "[github]\nx = 1").unwrap();
        append_section(d.path(), "[cloudflare.dns]\nzone = \"a.io\"\n").unwrap();
        let t = std::fs::read_to_string(d.path().join(FILE)).unwrap();
        assert_eq!(t, "[github]\nx = 1\n\n[cloudflare.dns]\nzone = \"a.io\"\n");
        assert!(has_section(d.path(), "cloudflare.dns"));
        assert!(!has_section(d.path(), "cloudflare.r2"));
    }
}
