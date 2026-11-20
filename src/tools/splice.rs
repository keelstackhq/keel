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

pub struct PieceInfo {
    pub id: String,
    pub multiple: bool,
    pub default_name: Option<String>,
}

pub enum Pieces {
    List(Vec<PieceInfo>),
    /// Old splice (no `pieces` command): render without validating.
    Unavailable(String),
    /// splice answered with an error (bad source or ref, template error).
    Failed(String),
}

/// `splice --json pieces <source> --ref <ref>` (splice#54): one object with
/// `pieces:[{id, description, base, multiple, default_name, requires, vars}]`.
/// Exit 2 is a bad source or usage on a splice that has the command; an older splice exits 2
/// without a `pieces` JSON object, which keel treats as the command being absent.
pub fn pieces(r: &dyn Runner, dir: &Path, source: &str, git_ref: &str) -> Pieces {
    let c = Cmd::new("splice", &["--json", "pieces", source, "--ref", git_ref]).dir(dir);
    let o = r.run(&c);
    if not_found(&o) {
        return Pieces::Unavailable(
            "splice is not installed; piece names were not validated".into(),
        );
    }
    let v = json(&o);
    let answered = v
        .as_ref()
        .is_some_and(|v| v["command"].as_str() == Some("pieces"));
    if o.code == 2 && !answered {
        return Pieces::Unavailable(
            "this splice has no `pieces` command; piece names were not validated (upgrade splice)"
                .into(),
        );
    }
    let list = v.as_ref().and_then(|v| {
        v["pieces"].as_array().map(|a| {
            a.iter()
                .filter_map(|p| {
                    Some(PieceInfo {
                        id: p["id"].as_str()?.to_string(),
                        multiple: p["multiple"].as_bool().unwrap_or(false),
                        default_name: p["default_name"].as_str().map(str::to_string),
                    })
                })
                .collect()
        })
    });
    match (o.code, list) {
        (0, Some(list)) => Pieces::List(list),
        _ if answered || o.code == 3 => Pieces::Failed(failure(&c, &o)),
        _ => Pieces::Unavailable(format!(
            "piece names were not validated: {}",
            failure(&c, &o)
        )),
    }
}

/// Problems with the manifest's `ID[:NAME]` entries against splice's piece list: unknown ids,
/// and a `multiple` piece with no `default_name` given without a `:NAME`.
pub fn check_pieces(wanted: &[String], list: &[PieceInfo]) -> Vec<String> {
    let mut problems = Vec::new();
    for w in wanted {
        let (id, name) = match w.split_once(':') {
            Some((i, n)) => (i, Some(n)),
            None => (w.as_str(), None),
        };
        match list.iter().find(|p| p.id == id) {
            None => problems.push(format!(
                "unknown piece {id}; valid ids: {}",
                list.iter()
                    .map(|p| p.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            Some(p) if p.multiple && p.default_name.is_none() && name.is_none_or(str::is_empty) => {
                problems.push(format!(
                    "piece {id} needs an instance name: pass --pieces {id}:NAME"
                ))
            }
            Some(_) => {}
        }
    }
    problems
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

    fn info(id: &str, multiple: bool, default_name: Option<&str>) -> PieceInfo {
        PieceInfo {
            id: id.into(),
            multiple,
            default_name: default_name.map(str::to_string),
        }
    }

    #[test]
    fn check_pieces_flags_unknown_ids_and_missing_instance_names() {
        let list = [
            info("rust-cli", true, None),
            info("docs", false, None),
            info("web", true, Some("web")),
        ];
        let ok: Vec<String> = ["rust-cli:my-cli", "docs", "web"].map(String::from).into();
        assert!(check_pieces(&ok, &list).is_empty());
        let bad: Vec<String> = ["rust-cli", "nope"].map(String::from).into();
        let p = check_pieces(&bad, &list);
        assert_eq!(p.len(), 2, "{p:?}");
        assert!(p[0].contains("rust-cli:NAME"), "{p:?}");
        assert!(
            p[1].contains("unknown piece nope") && p[1].contains("docs"),
            "{p:?}"
        );
    }

    #[test]
    fn pieces_reads_the_splice_54_shape_and_passes_the_ref() {
        use crate::exec::{FakeRunner, Out};
        let r = FakeRunner::new().on(
            &["splice", "--json", "pieces"],
            Out::ok(
                r#"{"ok":true,"command":"pieces","changes":[],"messages":[],"pieces":[{"id":"rust-cli","description":"d","base":false,"multiple":true,"default_name":null,"requires":["base"],"vars":[]}],"template":{"name":"hull-rust","version":null,"vars":[]}}"#,
            ),
        );
        let Pieces::List(l) = pieces(&r, Path::new("."), "gh:a/b", "v1") else {
            panic!()
        };
        assert_eq!(l[0].id, "rust-cli");
        assert!(l[0].multiple && l[0].default_name.is_none());
        assert_eq!(r.calls()[0].line(), "splice --json pieces gh:a/b --ref v1");
    }

    #[test]
    fn pieces_exit_2_with_a_pieces_object_is_a_failure_not_an_old_splice() {
        use crate::exec::{FakeRunner, Out};
        let r = FakeRunner::new().on(
            &["splice", "--json", "pieces"],
            Out::with(
                2,
                r#"{"ok":false,"command":"pieces","messages":["bad source"]}"#,
                "",
            ),
        );
        assert!(matches!(
            pieces(&r, Path::new("."), "gh:a/b", "v1"),
            Pieces::Failed(_)
        ));
        let old = FakeRunner::new().on(
            &["splice", "--json", "pieces"],
            Out::with(2, "", "error: unrecognized subcommand 'pieces'"),
        );
        assert!(matches!(
            pieces(&old, Path::new("."), "gh:a/b", "v1"),
            Pieces::Unavailable(_)
        ));
    }

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
