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
    /// `--area` is not understood: this hoist predates it.
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

/// An old hoist answers `--area` with a usage error (exit 2, no hoist JSON).
fn old_hoist(o: &Out) -> bool {
    o.code == 2
        && json(o).is_none_or(|v| v.get("human_steps").is_none() && v.get("items").is_none())
}

/// hoist's areas except `cloudflare`, which the dns and backup steps own.
const SETTINGS_AREAS: [&str; 4] = ["github", "npm", "crates", "release"];
const CLOUDFLARE: &str = "cloudflare";

/// `["--area", A, "--area", B, ...]`. The one place `--area` is spelled.
fn area_args(areas: &[&str]) -> Vec<String> {
    areas
        .iter()
        .flat_map(|a| ["--area".to_string(), a.to_string()])
        .collect()
}

fn cmd(base: &[&str], extra: Vec<String>, dir: &Path) -> Cmd {
    let mut a: Vec<&str> = base.to_vec();
    a.extend(extra.iter().map(String::as_str));
    Cmd::new("hoist", &a).dir(dir)
}

/// The part of the `cloudflare` area one step owns: hoist has no sub-areas, so a step reads
/// its own JSON items (by `kind`) and human steps (by `id` prefix) out of the whole area.
pub struct Slice {
    pub kinds: &'static [&'static str],
    pub human_prefixes: &'static [&'static str],
}

/// `[cloudflare.dns]`: records `dns_record`; human steps `dns:<name> <TYPE>`, `dns-cname:<host>`.
pub const DNS: Slice = Slice {
    kinds: &["dns_record"],
    human_prefixes: &["dns:", "dns-cname:"],
};

/// `[cloudflare.r2]`: items `r2_bucket`, `r2_lifecycle`, `r2_token`. No human step id is
/// relayed for it, so none is claimed.
pub const R2: Slice = Slice {
    kinds: &["r2_bucket", "r2_lifecycle", "r2_token"],
    human_prefixes: &[],
};

/// `hoist --json check --area github --area npm --area crates --area release`. A hoist that
/// rejects `--area` (exit 2 without hoist JSON) gets the plain `check`.
pub fn check_settings(r: &dyn Runner, dir: &Path) -> Res {
    let c = cmd(&["--json", "check"], area_args(&SETTINGS_AREAS), dir);
    let o = r.run(&c);
    if old_hoist(&o) {
        return check_plain(r, dir);
    }
    check_res(&c, &o)
}

fn check_plain(r: &dyn Runner, dir: &Path) -> Res {
    let c = Cmd::new("hoist", &["--json", "check"]).dir(dir);
    check_res(&c, &r.run(&c))
}

fn check_res(c: &Cmd, o: &Out) -> Res {
    match o.code {
        0..=2 => match envelope(c, o) {
            Ok(_) if o.code == 0 => Res::Ok,
            Ok(_) => Res::Drift,
            Err(e) => Res::Failed(e),
        },
        _ => Res::Failed(failure(c, o)),
    }
}

/// `hoist --json apply --yes` plus the same `--area` flags as [`check_settings`], same fallback.
pub fn apply_settings(r: &dyn Runner, dir: &Path) -> Res {
    let c = cmd(
        &["--json", "apply", "--yes"],
        area_args(&SETTINGS_AREAS),
        dir,
    );
    let o = r.run(&c);
    if old_hoist(&o) {
        let c = Cmd::new("hoist", &["--json", "apply", "--yes"]).dir(dir);
        return apply_res(&c, &r.run(&c));
    }
    apply_res(&c, &o)
}

/// An item is in drift unless its `ok` is true or its `action` or `status` is "ok" or "noop".
/// Assumed from hoist's item shape until the exact field is relayed.
fn in_drift(item: &Value) -> bool {
    let word = |k: &str| matches!(item[k].as_str(), Some("ok" | "noop"));
    !(item["ok"].as_bool() == Some(true) || word("action") || word("status"))
}

/// The first human step of `v` whose id starts with one of the slice's prefixes.
fn slice_human(v: &Value, c: &Cmd, s: &Slice) -> Option<Res> {
    let h = v["human_steps"].as_array()?.iter().find(|h| {
        h["id"]
            .as_str()
            .is_some_and(|id| s.human_prefixes.iter().any(|p| id.starts_with(p)))
    })?;
    Some(human_res(v, h, c))
}

/// `hoist --json check --area cloudflare`, read for one [`Slice`]: exit 0 is done; exit 1 is
/// drift only when an item of the slice's kinds is in drift (see [`in_drift`]), else done; exit 2
/// is the first human step of the slice, else done. A hoist that rejects `--area` is `Upgrade`.
pub fn check_cloudflare(r: &dyn Runner, dir: &Path, s: &Slice) -> Res {
    let c = cmd(&["--json", "check"], area_args(&[CLOUDFLARE]), dir);
    let o = r.run(&c);
    if old_hoist(&o) {
        return Res::Upgrade;
    }
    match o.code {
        0 => match envelope(&c, &o) {
            Ok(_) => Res::Ok,
            Err(e) => Res::Failed(e),
        },
        1 | 2 => match envelope(&c, &o) {
            Err(e) => Res::Failed(e),
            Ok(v) if o.code == 1 => {
                let drift = v["items"].as_array().is_some_and(|a| {
                    a.iter()
                        .any(|i| s.kinds.contains(&i["kind"].as_str().unwrap_or("")) && in_drift(i))
                });
                if drift { Res::Drift } else { Res::Ok }
            }
            Ok(v) => slice_human(&v, &c, s).unwrap_or(Res::Ok),
        },
        _ => Res::Failed(failure(&c, &o)),
    }
}

/// `hoist --json apply --yes --area cloudflare --secrets-to fly:APP`. The sink is always passed:
/// the area also mints the R2 key when `[cloudflare.r2]` exists, and minting without a sink
/// exits 3. Exit 2 is the slice's first human step, else done.
pub fn apply_cloudflare(r: &dyn Runner, dir: &Path, s: &Slice, fly_app: &str) -> Res {
    let mut extra = area_args(&[CLOUDFLARE]);
    extra.extend(["--secrets-to".to_string(), format!("fly:{fly_app}")]);
    let c = cmd(&["--json", "apply", "--yes"], extra, dir);
    let o = r.run(&c);
    if old_hoist(&o) {
        return Res::Upgrade;
    }
    match o.code {
        2 => match envelope(&c, &o) {
            Ok(v) => slice_human(&v, &c, s).unwrap_or(Res::Ok),
            Err(e) => Res::Failed(e),
        },
        _ => apply_res(&c, &o),
    }
}

fn human_res(v: &Value, h: &Value, c: &Cmd) -> Res {
    match h["title"].as_str() {
        Some(t) => Res::Human {
            title: t.to_string(),
            url: h["url"].as_str().unwrap_or("").to_string(),
            command: variable_command(v, h).unwrap_or_else(|| "keel up".into()),
        },
        None => Res::Failed(format!(
            "`{}` exited 2 without a human step; run it by hand",
            c.line()
        )),
    }
}

fn apply_res(c: &Cmd, o: &Out) -> Res {
    match o.code {
        0 => match envelope(c, o) {
            Ok(_) => Res::Ok,
            Err(e) => Res::Failed(e),
        },
        2 => match envelope(c, o) {
            Ok(v) => human_res(&v, &v["human_steps"][0], c),
            Err(e) => Res::Failed(e),
        },
        _ => Res::Failed(failure(c, o)),
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

    fn lines(r: &FakeRunner) -> Vec<String> {
        r.calls().iter().map(|c| c.line()).collect()
    }

    const AREAS: &str = "--area github --area npm --area crates --area release";

    #[test]
    fn settings_names_every_area_but_cloudflare_and_falls_back_on_old_hoist() {
        let r = FakeRunner::new().on(
            &["hoist", "--json", "check", "--area"],
            Out::with(2, "", "error: unexpected argument '--area'"),
        );
        let r = r.on(&["hoist", "--json", "check"], Out::ok(r#"{"ok":true}"#));
        assert!(matches!(check_settings(&r, Path::new(".")), Res::Ok));
        assert_eq!(
            lines(&r),
            [
                format!("hoist --json check {AREAS}"),
                "hoist --json check".into()
            ]
        );
        let r = FakeRunner::new().on(&["hoist"], Out::ok(r#"{"ok":true,"items":[]}"#));
        apply_settings(&r, Path::new("."));
        assert_eq!(lines(&r), [format!("hoist --json apply --yes {AREAS}")]);
        // Exit 2 with hoist JSON is a human step, not an old hoist.
        let r = FakeRunner::new().on(
            &["hoist"],
            Out::with(2, r#"{"items":[],"human_steps":[]}"#, ""),
        );
        assert!(matches!(check_settings(&r, Path::new(".")), Res::Drift));
        assert_eq!(r.calls().len(), 1);
    }

    #[test]
    fn cloudflare_check_reads_the_slice_out_of_the_whole_area() {
        let chk = |out: Out, s: &Slice| {
            let r = FakeRunner::new().on(&["hoist"], out);
            let res = check_cloudflare(&r, Path::new("."), s);
            assert_eq!(lines(&r), ["hoist --json check --area cloudflare"]);
            res
        };
        assert!(matches!(chk(Out::ok(r#"{"items":[]}"#), &DNS), Res::Ok));
        let drift = r#"{"items":[{"kind":"r2_bucket","key":"b","action":"create"},{"kind":"dns_record","key":"h A 1.2.3.4","action":"ok"}]}"#;
        assert!(matches!(chk(Out::with(1, drift, ""), &DNS), Res::Ok));
        assert!(matches!(chk(Out::with(1, drift, ""), &R2), Res::Drift));
        let d2 = r#"{"items":[{"kind":"dns_record","key":"h A 1.2.3.4","ok":false}]}"#;
        assert!(matches!(chk(Out::with(1, d2, ""), &DNS), Res::Drift));
        let n = r#"{"items":[{"kind":"dns_record","key":"h A 1.2.3.4","status":"noop"}]}"#;
        assert!(matches!(chk(Out::with(1, n, ""), &DNS), Res::Ok));
        assert!(matches!(
            chk(Out::with(1, "boom", ""), &DNS),
            Res::Failed(_)
        ));
        assert!(matches!(chk(Out::with(3, "", "x"), &DNS), Res::Failed(_)));
        assert!(matches!(
            chk(Out::with(2, "", "unexpected argument '--area'"), &DNS),
            Res::Upgrade
        ));
        let h = r#"{"items":[],"human_steps":[{"id":"pages:x","title":"other","url":""},{"id":"dns-cname:app.example.com","title":"Add a CNAME","url":"u"}]}"#;
        match chk(Out::with(2, h, ""), &DNS) {
            Res::Human { title, url, .. } => {
                assert_eq!((title.as_str(), url.as_str()), ("Add a CNAME", "u"))
            }
            _ => panic!(),
        }
        assert!(matches!(chk(Out::with(2, h, ""), &R2), Res::Ok));
    }

    #[test]
    fn cloudflare_apply_passes_the_sink_and_reads_exit_codes() {
        let run = |out: Out| {
            let r = FakeRunner::new().on(&["hoist"], out);
            let res = apply_cloudflare(&r, Path::new("."), &DNS, "app");
            assert_eq!(
                lines(&r),
                ["hoist --json apply --yes --area cloudflare --secrets-to fly:app"]
            );
            res
        };
        assert!(matches!(run(Out::ok(r#"{"items":[]}"#)), Res::Ok));
        assert!(matches!(run(Out::with(3, "", "no sink")), Res::Failed(_)));
        assert!(matches!(
            run(Out::with(2, "", "unexpected argument '--area'")),
            Res::Upgrade
        ));
        let h = r#"{"items":[],"human_steps":[{"id":"dns:app.example.com A","title":"Add A","url":""}]}"#;
        assert!(matches!(run(Out::with(2, h, "")), Res::Human { .. }));
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
