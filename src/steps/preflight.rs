use super::repo;
use crate::ctx::Ctx;
use crate::error::KeelError;
use crate::exec::{Cmd, Out};
use crate::report::HumanStep;
use crate::step::{Check, Step};
use crate::tools::{gh, git, not_found, npm};

pub struct Preflight;

/// The tools keel calls, with the binary to ask for `--version`.
const TOOLS: [&str; 3] = ["splice", "bosun", "hoist"];

fn at_pin(out: &Out, version: &str) -> bool {
    out.code == 0 && out.stdout.contains(version)
}

impl Preflight {
    /// Tools not at their pin, as `(name, found)`.
    fn behind(cx: &Ctx) -> Vec<(&'static str, String)> {
        TOOLS
            .iter()
            .filter_map(|t| {
                let pin = cx.pins.tool(t)?;
                let o = cx.runner.run(&Cmd::new(t, &["--version"]));
                (!at_pin(&o, &pin.version)).then(|| {
                    let found = if o.code == 0 {
                        o.stdout.trim().to_string()
                    } else {
                        "none".into()
                    };
                    (*t, found)
                })
            })
            .collect()
    }
}

impl Step for Preflight {
    fn id(&self) -> &'static str {
        "preflight"
    }

    fn check(&self, cx: &mut Ctx) -> Result<Check, KeelError> {
        let repo = repo(cx)?;
        let r = cx.runner;

        let node = r.run(&Cmd::new("node", &["--version"]));
        if not_found(&node) {
            return Ok(Check::Failed(
                "node not found; install Node 24 or newer".into(),
            ));
        }
        let node_v = node.stdout.trim().trim_start_matches('v').to_string();
        let major: u32 = node_v
            .split('.')
            .next()
            .and_then(|m| m.parse().ok())
            .unwrap_or(0);
        if node.code != 0 || major < 24 {
            return Ok(Check::Failed(format!(
                "node {node_v} is too old; install Node 24 or newer (https://nodejs.org)"
            )));
        }

        match gh::auth_ok(r) {
            None => {
                return Ok(Check::Failed(
                    "gh not found; install the GitHub CLI (https://cli.github.com)".into(),
                ));
            }
            Some(false) => {
                return Ok(Check::Human(HumanStep {
                    id: self.id().into(),
                    title: "Log in to GitHub with the gh CLI".into(),
                    url: String::new(),
                    command: "gh auth login".into(),
                }));
            }
            Some(true) => {}
        }
        let Some(login) = gh::login(r) else {
            return Ok(Check::Failed(
                "`gh api user` failed; run `gh auth status` and `gh auth login`".into(),
            ));
        };
        let Some(git_v) = git::version(r) else {
            return Ok(Check::Failed("git not found; install git".into()));
        };
        if let Err(e) = gh::default_branch(r, &repo) {
            return Ok(Check::Failed(format!(
                "repo {repo} is not reachable ({e}); create it or check the name and your access"
            )));
        }
        if !git::is_work_tree(r, &cx.dir) {
            return Ok(Check::Failed(format!(
                "{} is not a git checkout; run `git clone` of {repo} and use -C",
                cx.dir.display()
            )));
        }
        match git::origin(r, &cx.dir) {
            Some(u) if git::origin_matches(&u, &repo) => {}
            Some(u) => {
                return Ok(Check::Failed(format!(
                    "origin is {u}, not {repo}; fix it with `git remote set-url origin` or pass -C of the right checkout"
                )));
            }
            None => {
                return Ok(Check::Failed(format!(
                    "the checkout has no `origin`; run `git remote add origin https://github.com/{repo}.git`"
                )));
            }
        }
        let behind = Preflight::behind(cx);
        if !behind.is_empty() {
            let list: Vec<String> = behind
                .iter()
                .map(|(t, f)| {
                    let v = cx.pins.tool(t).map(|p| p.version).unwrap_or_default();
                    format!("{t} {v} (found {f})")
                })
                .collect();
            return Ok(Check::Todo(format!("install {}", list.join(", "))));
        }
        Ok(Check::Done(format!(
            "node {node_v}, gh as {login}, git {git_v}"
        )))
    }

    fn apply(&self, cx: &mut Ctx) -> Result<Check, KeelError> {
        for (tool, _) in Preflight::behind(cx) {
            let Some(pin) = cx.pins.tool(tool) else {
                continue;
            };
            if let Err(e) = npm::install_pinned(cx.runner, tool, &pin) {
                return Ok(Check::Failed(e));
            }
        }
        self.check(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;
    use crate::ctx::Opts;
    use crate::exec::FakeRunner;
    use crate::pins::platform_key;
    use std::path::PathBuf;

    fn healthy() -> FakeRunner {
        FakeRunner::new()
            .on(&["node", "--version"], Out::ok("v24.1.0\n"))
            .on(&["gh", "auth", "status"], Out::ok(""))
            .on(&["gh", "api", "user"], Out::ok("octo\n"))
            .on(&["git", "--version"], Out::ok("git version 2.47.0\n"))
            .on(
                &["gh", "repo", "view"],
                Out::ok(r#"{"defaultBranchRef":{"name":"main"}}"#),
            )
            .on(
                &["git", "rev-parse", "--is-inside-work-tree"],
                Out::ok("true\n"),
            )
            .on(
                &["git", "remote", "get-url", "origin"],
                Out::ok("https://github.com/owner/name.git\n"),
            )
            .on(&["splice", "--version"], Out::ok("splice 0.6.0\n"))
            .on(&["bosun", "--version"], Out::ok("bosun 0.21.0\n"))
            .on(&["hoist", "--version"], Out::ok("hoist 0.5.1\n"))
    }

    fn run_check(r: &FakeRunner, pins: &str) -> Check {
        let mut cx = ctx(
            r,
            PathBuf::from("."),
            manifest("rust"),
            pins,
            Opts::default(),
            &no_sleep,
            &no_stdin,
        );
        Preflight.check(&mut cx).unwrap()
    }

    #[test]
    fn all_good_is_done() {
        assert_eq!(
            run_check(&healthy(), PINS),
            Check::Done("node 24.1.0, gh as octo, git 2.47.0".into())
        );
    }

    #[test]
    fn old_node_fails_with_fix() {
        let r = healthy().on(&["node", "--version"], Out::ok("v22.3.0\n"));
        let Check::Failed(m) = run_check(&r, PINS) else {
            panic!()
        };
        assert!(m.contains("22.3.0") && m.contains("Node 24"), "{m}");
    }

    #[test]
    fn gh_not_logged_in_is_human() {
        let r = healthy().on(&["gh", "auth", "status"], Out::with(1, "", "not logged in"));
        let Check::Human(h) = run_check(&r, PINS) else {
            panic!()
        };
        assert_eq!(h.command, "gh auth login");
    }

    #[test]
    fn wrong_origin_fails() {
        let r = healthy().on(
            &["git", "remote", "get-url", "origin"],
            Out::ok("https://github.com/other/thing.git\n"),
        );
        let Check::Failed(m) = run_check(&r, PINS) else {
            panic!()
        };
        assert!(m.contains("other/thing"), "{m}");
    }

    #[test]
    fn tool_behind_is_todo() {
        let r = healthy().on(&["bosun", "--version"], Out::ok("bosun 0.20.0\n"));
        let Check::Todo(m) = run_check(&r, PINS) else {
            panic!()
        };
        assert!(m.contains("bosun 0.21.0 (found bosun 0.20.0)"), "{m}");
    }

    /// Pins text for splice with real sha512s of the two fake tarballs.
    fn pins_for(launcher: &[u8], platform: &[u8]) -> String {
        format!(
            "{PINS}\n[sha512.splice]\nversion = \"0.6.0\"\nlauncher = \"{}\"\n{} = \"{}\"\n",
            npm::sha512_b64(launcher),
            platform_key(),
            npm::sha512_b64(platform)
        )
    }

    fn fixture(good: bool) -> (tempfile::TempDir, FakeRunner, String) {
        let tars = tempfile::tempdir().unwrap();
        let (l, p) = (tars.path().join("l.tgz"), tars.path().join("p.tgz"));
        std::fs::write(&l, b"launcher").unwrap();
        std::fs::write(
            &p,
            if good {
                &b"platform"[..]
            } else {
                &b"tampered"[..]
            },
        )
        .unwrap();
        let plat_pkg = format!(
            "@keelstackhq/splice-{}@0.6.0",
            npm::npm_platform(&platform_key())
        );
        let r = healthy()
            .on(&["splice", "--version"], Out::ok("splice 0.5.0\n"))
            // `npm pack` prints an absolute path here, which `dir.join` keeps as is.
            .on(
                &["npm", "pack", "--silent", "@keelstackhq/splice@0.6.0"],
                Out::ok(&format!("{}\n", l.display())),
            )
            .on(
                &["npm", "pack", "--silent", &plat_pkg],
                Out::ok(&format!("{}\n", p.display())),
            )
            .on(&["npm", "install"], Out::ok(""));
        (tars, r, pins_for(b"launcher", b"platform"))
    }

    #[test]
    fn install_with_good_sha_runs_npm_install_g() {
        let (_t, r, pins) = fixture(true);
        let mut cx = ctx(
            &r,
            PathBuf::from("."),
            manifest("rust"),
            &pins,
            Opts::default(),
            &no_sleep,
            &no_stdin,
        );
        let _ = Preflight.apply(&mut cx).unwrap();
        let inst = r
            .calls()
            .into_iter()
            .find(|c| c.program == "npm" && c.args[0] == "install")
            .unwrap();
        assert_eq!(
            &inst.args[..4],
            ["install", "-g", "--ignore-scripts", "--omit=optional"]
        );
        assert_eq!(inst.args.len(), 6);
        assert!(inst.args[4].ends_with("l.tgz") && inst.args[5].ends_with("p.tgz"));
    }

    #[test]
    fn bad_sha_fails_and_installs_nothing() {
        let (_t, r, pins) = fixture(false);
        let mut cx = ctx(
            &r,
            PathBuf::from("."),
            manifest("rust"),
            &pins,
            Opts::default(),
            &no_sleep,
            &no_stdin,
        );
        let Check::Failed(m) = Preflight.apply(&mut cx).unwrap() else {
            panic!()
        };
        assert!(m.contains("mismatch"), "{m}");
        assert!(
            !r.calls()
                .iter()
                .any(|c| c.program == "npm" && c.args[0] == "install")
        );
    }

    #[test]
    fn no_sha512_never_installs() {
        let r = healthy().on(&["splice", "--version"], Out::ok("splice 0.5.0\n"));
        let mut cx = ctx(
            &r,
            PathBuf::from("."),
            manifest("rust"),
            PINS,
            Opts::default(),
            &no_sleep,
            &no_stdin,
        );
        let Check::Failed(m) = Preflight.apply(&mut cx).unwrap() else {
            panic!()
        };
        assert!(m.contains("KEEL_PINS") && m.contains("update keel"), "{m}");
        assert!(!r.calls().iter().any(|c| c.program == "npm"));
    }
}
