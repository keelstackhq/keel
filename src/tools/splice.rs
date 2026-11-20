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

/// `splice --json --yes new <source> . --force --piece ID[:NAME]... [--data no_placeholder=true]`
/// (`--force`: the checkout is never empty; keel commits `keel.toml` first so the tree is clean).
pub fn new_cmd(dir: &Path, source: &str, pieces: &[String], rust: bool) -> Cmd {
    let mut args: Vec<&str> = vec!["--json", "--yes", "new", source, ".", "--force"];
    for p in pieces {
        args.push("--piece");
        args.push(p);
    }
    if rust {
        args.extend(["--data", "no_placeholder=true"]);
    }
    Cmd::new("splice", &args).dir(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources() {
        assert_eq!(source("rust"), "gh:keelstackhq/hull//template-rust");
        assert_eq!(source("ts"), "gh:keelstackhq/hull//template");
        assert_eq!(source("gh:x/y"), "gh:x/y");
    }
}
