use super::{failure, json, not_found};
use crate::exec::{Cmd, Runner};
use std::path::Path;

pub const RUST_SOURCE: &str = "gh:keelstackhq/hull//template-rust";
pub const TS_SOURCE: &str = "gh:keelstackhq/hull//template";

/// `rust`/`ts` map to hull's templates; anything else is passed through as the source.
pub fn source(template: &str) -> &str {
    match template {
        "rust" => RUST_SOURCE,
        "ts" => TS_SOURCE,
        other => other,
    }
}

pub enum Pieces {
    Ids(Vec<String>),
    /// Old splice (exit 2) or unusable output: render without validating.
    Unavailable(String),
}

/// `splice --json pieces <source>`; assumed `{pieces:[{id,..}]}` (spec §10).
pub fn pieces(r: &dyn Runner, dir: &Path, source: &str) -> Pieces {
    let c = Cmd::new("splice", &["--json", "pieces", source]).dir(dir);
    let o = r.run(&c);
    if o.code == 2 || not_found(&o) {
        return Pieces::Unavailable(
            "this splice has no `pieces` command; piece names were not validated (upgrade splice)"
                .into(),
        );
    }
    let ids = json(&o).and_then(|v| {
        v["pieces"].as_array().map(|a| {
            a.iter()
                .filter_map(|p| p["id"].as_str().map(str::to_string))
                .collect()
        })
    });
    match (o.code, ids) {
        (0, Some(ids)) => Pieces::Ids(ids),
        _ => Pieces::Unavailable(format!(
            "piece names were not validated: {}",
            failure(&c, &o)
        )),
    }
}

/// `splice --json --yes new <source> . --ref <ref> --force --piece ID[:NAME]... [--data no_placeholder=true]`
/// (`--force`: the checkout is never empty; keel commits `keel.toml` first so the tree is clean).
pub fn new_cmd(dir: &Path, source: &str, git_ref: &str, pieces: &[String], rust: bool) -> Cmd {
    let mut args: Vec<&str> = vec![
        "--json", "--yes", "new", source, ".", "--ref", git_ref, "--force",
    ];
    for p in pieces {
        args.push("--piece");
        args.push(p);
    }
    if rust {
        args.extend(["--data", "no_placeholder=true"]);
    }
    Cmd::new("splice", &args).dir(dir)
}

/// The files a render wrote: `changes[].path` whose action is not `skip`, plus
/// `.splice/lock.toml`. Absolute paths, `..` and `.git/` entries are dropped.
pub fn render_files(out: &serde_json::Value) -> Vec<String> {
    let mut files: Vec<String> = out["changes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["action"].as_str() != Some("skip"))
        .filter_map(|c| c["path"].as_str())
        .filter(|p| {
            let path = Path::new(p);
            path.is_relative()
                && !p.is_empty()
                && path.components().all(
                    |c| matches!(c, std::path::Component::Normal(s) if s != ".git" && s != ".keel"),
                )
        })
        .map(str::to_string)
        .collect();
    files.push(".splice/lock.toml".into());
    files.sort();
    files.dedup();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_passes_the_ref() {
        let c = new_cmd(Path::new("."), "gh:a/b", "v1", &["p:x".into()], true);
        assert_eq!(
            c.line(),
            "splice --json --yes new gh:a/b . --ref v1 --force --piece p:x --data no_placeholder=true"
        );
    }

    #[test]
    fn render_files_skip_skipped_and_unsafe_paths() {
        let v = serde_json::json!({"changes":[
            {"path":"a.txt","action":"create"},
            {"path":"dir/b.rs","action":"merge"},
            {"path":"c.txt","action":"skip"},
            {"path":"../x","action":"create"},
            {"path":"/etc/x","action":"create"},
            {"path":".git/hooks/x","action":"create"},
            {"path":"a.txt","action":"merge"}]});
        assert_eq!(render_files(&v), [".splice/lock.toml", "a.txt", "dir/b.rs"]);
        assert_eq!(
            render_files(&serde_json::json!({"ok":true})),
            [".splice/lock.toml"]
        );
    }

    #[test]
    fn sources() {
        assert_eq!(source("rust"), "gh:keelstackhq/hull//template-rust");
        assert_eq!(source("ts"), "gh:keelstackhq/hull//template");
        assert_eq!(source("gh:x/y"), "gh:x/y");
    }
}
