use super::repo;
use crate::ctx::Ctx;
use crate::error::KeelError;
use crate::exec::Cmd;
use crate::step::{Check, Step};
use crate::tools::{self, gh, git, splice};

pub struct Render;

fn template(cx: &Ctx) -> Result<String, KeelError> {
    cx.manifest.template.clone().ok_or_else(|| {
        KeelError::Env(
            "no template: pass --template rust|ts|SOURCE or set `template` in keel.toml".into(),
        )
    })
}

impl Step for Render {
    fn id(&self) -> &'static str {
        "render"
    }

    fn check(&self, cx: &mut Ctx) -> Result<Check, KeelError> {
        let lock = cx.dir.join(".splice/lock.toml").is_file();
        let needs_cargo_lock =
            cx.dir.join("Cargo.toml").is_file() && !cx.dir.join("Cargo.lock").is_file();
        Ok(match (lock, needs_cargo_lock) {
            (true, false) => Check::Done("rendered (.splice/lock.toml)".into()),
            (true, true) => Check::Todo("Cargo.lock is missing".into()),
            (false, _) => Check::Todo(format!("render {}", template(cx)?)),
        })
    }

    fn apply(&self, cx: &mut Ctx) -> Result<Check, KeelError> {
        let repo = repo(cx)?;
        let tpl = template(cx)?;
        let src = splice::source(&tpl).to_string();
        let r = cx.runner;
        let Some(git_ref) = cx.manifest.template_ref.clone() else {
            return Ok(Check::Failed(format!(
                "no template_ref for {src}: pass --template-ref REF (tag, branch or sha) or set `template_ref` in keel.toml"
            )));
        };
        let lock_before = cx.dir.join(".splice/lock.toml").is_file();

        if !lock_before {
            // Validate piece names before touching the checkout.
            match splice::pieces(r, &cx.dir, &src) {
                splice::Pieces::Ids(ids) => {
                    let bad: Vec<&str> = cx
                        .manifest
                        .pieces
                        .iter()
                        .map(|p| p.split(':').next().unwrap_or(p))
                        .filter(|id| !ids.iter().any(|i| i == id))
                        .collect();
                    if !bad.is_empty() {
                        return Ok(Check::Failed(format!(
                            "unknown piece {} in {src}; valid ids: {}",
                            bad.join(", "),
                            ids.join(", ")
                        )));
                    }
                }
                splice::Pieces::Unavailable(w) => eprintln!("keel: warning: {w}"),
            }
        }
        // Every path below commits: be on the work branch first (§5.1).
        let default = match gh::default_branch(r, &repo) {
            Ok(b) => b,
            Err(e) => return Ok(Check::Failed(e)),
        };
        if let Err(e) = git::ensure_branch(r, &cx.dir, &default, cx.manifest.branch_name()) {
            return Ok(Check::Failed(e));
        }
        // The branch may already hold the render (a re-run from a fresh clone).
        let rendered = cx.dir.join(".splice/lock.toml").is_file();
        let mut render_files: Vec<String> = Vec::new();
        if !rendered {
            // `splice new` refuses a dirty tree: commit keel.toml (only it) on the branch first.
            if let Err(e) = git::commit_path(r, &cx.dir, "keel.toml", "keel: add keel.toml") {
                return Ok(Check::Failed(e));
            }
            let rust = src == splice::RUST_SOURCE;
            let c = splice::new_cmd(&cx.dir, &src, &git_ref, &cx.manifest.pieces, rust);
            let o = r.run(&c);
            if o.code != 0 {
                return Ok(Check::Failed(tools::failure(&c, &o)));
            }
            match tools::json(&o) {
                Some(v) => render_files = splice::render_files(&v),
                None => return Ok(Check::Failed(tools::not_json(&c))),
            }
        }
        let lockfile = cx.dir.join("Cargo.toml").is_file() && !cx.dir.join("Cargo.lock").is_file();
        if lockfile {
            let c = Cmd::new("cargo", &["generate-lockfile"]).dir(&cx.dir);
            let o = r.run(&c);
            if o.code != 0 {
                return Ok(Check::Failed(tools::failure(&c, &o)));
            }
        }
        // keel's scratch dir must never reach the target repo's history.
        let ignored = match git::ensure_ignored(&cx.dir, ".keel/") {
            Ok(c) => c,
            Err(e) => return Ok(Check::Failed(format!("cannot update .gitignore: {e}"))),
        };
        let committed = if rendered {
            // Lockfile-only: commit just what keel changed, never the rest of the worktree.
            let mut res = Ok(());
            if lockfile {
                res = git::commit_path(r, &cx.dir, "Cargo.lock", "keel: add Cargo.lock");
            }
            if res.is_ok() && ignored {
                res = git::commit_path(r, &cx.dir, ".gitignore", "keel: ignore .keel");
            }
            res
        } else {
            // Only what the render wrote, plus what keel itself changed: never `git add -A`.
            let mut paths = render_files;
            if lockfile {
                paths.push("Cargo.lock".into());
            }
            if ignored {
                paths.push(".gitignore".into());
            }
            git::commit_paths(r, &cx.dir, &paths, &format!("keel: render {tpl}"))
        };
        if let Err(e) = committed {
            return Ok(Check::Failed(e));
        }
        self.check(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;
    use crate::ctx::Opts;
    use crate::exec::{FakeRunner, Out};

    fn fake() -> FakeRunner {
        FakeRunner::new()
            .on(
                &["gh", "repo", "view"],
                Out::ok(r#"{"defaultBranchRef":{"name":"main"}}"#),
            )
            .on(
                &["git", "rev-parse", "--abbrev-ref", "HEAD"],
                Out::ok("main\n"),
            )
            .on(&["git", "rev-parse", "--verify"], Out::with(1, "", ""))
            .on(&["git", "checkout", "-b", "keel/bootstrap"], Out::ok(""))
            .on(
                &["splice", "--json", "pieces"],
                Out::ok(r#"{"ok":true,"pieces":[{"id":"rust-cli"},{"id":"docs"}]}"#),
            )
            .on(
                &["splice", "--json", "--yes", "new"],
                Out::ok(
                    r#"{"ok":true,"changes":[{"path":"src/main.rs","action":"create"},{"path":"skipped.txt","action":"skip"}]}"#,
                ),
            )
            .on(&["cargo", "generate-lockfile"], Out::ok(""))
            .on(&["git", "status", "--porcelain"], Out::ok("?? keel.toml\n"))
            .on(&["git", "add"], Out::ok(""))
            .on(&["git", "commit"], Out::ok(""))
    }

    fn apply(r: &FakeRunner, dir: &std::path::Path, tpl: &str) -> Check {
        let mut cx = ctx(
            r,
            dir.to_path_buf(),
            manifest(tpl),
            PINS,
            Opts::default(),
            &no_sleep,
            &no_stdin,
        );
        Render.apply(&mut cx).unwrap()
    }

    fn lines(r: &FakeRunner) -> Vec<String> {
        r.calls().iter().map(|c| c.line()).collect()
    }

    #[test]
    fn check_states() {
        let d = tempfile::tempdir().unwrap();
        let r = FakeRunner::new();
        let mut cx = ctx(
            &r,
            d.path().into(),
            manifest("rust"),
            PINS,
            Opts::default(),
            &no_sleep,
            &no_stdin,
        );
        assert!(matches!(Render.check(&mut cx).unwrap(), Check::Todo(_)));
        std::fs::create_dir(d.path().join(".splice")).unwrap();
        std::fs::write(d.path().join(".splice/lock.toml"), "").unwrap();
        assert!(matches!(Render.check(&mut cx).unwrap(), Check::Done(_)));
        std::fs::write(d.path().join("Cargo.toml"), "").unwrap();
        assert!(
            matches!(Render.check(&mut cx).unwrap(), Check::Todo(m) if m.contains("Cargo.lock"))
        );
    }

    #[test]
    fn render_builds_exact_argv_in_order() {
        let d = tempfile::tempdir().unwrap();
        let r = fake();
        // Not done after apply (the fake writes no lock file): only the calls matter here.
        let _ = apply(&r, d.path(), "rust");
        let l = lines(&r);
        let pos = |s: &str| {
            l.iter()
                .position(|x| x == s)
                .unwrap_or_else(|| panic!("{s} in {l:?}"))
        };
        let new = "splice --json --yes new gh:keelstackhq/hull//template-rust . --ref v0.13.0 --force --piece rust-cli:my-cli --data no_placeholder=true";
        let kc = "git commit -m keel: add keel.toml -- keel.toml";
        assert!(pos("git checkout -b keel/bootstrap") < pos("git add -- keel.toml"));
        assert!(pos("git add -- keel.toml") < pos(kc));
        assert!(pos(kc) < pos(new));
        let add = "git add -- .splice/lock.toml src/main.rs .gitignore";
        assert!(pos(new) < pos(add));
        assert!(
            pos(add)
                < pos(
                    "git commit -m keel: render rust -- .splice/lock.toml src/main.rs .gitignore"
                )
        );
        assert!(!l.iter().any(|x| x.contains("skipped.txt")), "{l:?}");
        assert!(
            !l.iter()
                .any(|x| x.contains(" -A") || x.contains("commit -a")),
            "{l:?}"
        );
        assert_eq!(
            r.calls()
                .iter()
                .find(|c| c.program == "splice" && c.args[2] == "new")
                .unwrap()
                .dir
                .as_deref(),
            Some(d.path())
        );
    }

    #[test]
    fn clean_keel_toml_is_not_recommitted() {
        let d = tempfile::tempdir().unwrap();
        let r = fake().on(&["git", "status", "--porcelain"], Out::ok(""));
        let _ = apply(&r, d.path(), "rust");
        let l = lines(&r);
        assert!(
            !l.iter().any(|x| x.contains("keel: add keel.toml")),
            "{l:?}"
        );
        assert!(l.iter().any(|x| x.starts_with("splice --json --yes new")));
    }

    #[test]
    fn ts_template_has_no_rust_data_flag() {
        let d = tempfile::tempdir().unwrap();
        let r = fake();
        let _ = apply(&r, d.path(), "ts");
        assert!(
            lines(&r).contains(
                &"splice --json --yes new gh:keelstackhq/hull//template . --ref v0.13.0 --force --piece rust-cli:my-cli"
                    .to_string()
            )
        );
    }

    #[test]
    fn current_feature_branch_is_kept() {
        let d = tempfile::tempdir().unwrap();
        let r = fake().on(
            &["git", "rev-parse", "--abbrev-ref", "HEAD"],
            Out::ok("claude/work\n"),
        );
        let _ = apply(&r, d.path(), "rust");
        assert!(!lines(&r).iter().any(|l| l.starts_with("git checkout")));
    }

    #[test]
    fn pieces_exit_2_warns_and_renders() {
        let d = tempfile::tempdir().unwrap();
        let r = fake().on(
            &["splice", "--json", "pieces"],
            Out::with(2, "", "unknown command"),
        );
        let _ = apply(&r, d.path(), "rust");
        assert!(
            lines(&r)
                .iter()
                .any(|l| l.starts_with("splice --json --yes new"))
        );
    }

    #[test]
    fn unknown_piece_fails_listing_valid_ids_before_any_write() {
        let d = tempfile::tempdir().unwrap();
        let r = fake().on(
            &["splice", "--json", "pieces"],
            Out::ok(r#"{"ok":true,"pieces":[{"id":"docs"},{"id":"web"}]}"#),
        );
        let Check::Failed(m) = apply(&r, d.path(), "rust") else {
            panic!()
        };
        assert!(m.contains("rust-cli") && m.contains("docs, web"), "{m}");
        assert!(
            !lines(&r)
                .iter()
                .any(|l| l.starts_with("git") || l.contains(" new "))
        );
    }

    #[test]
    fn custom_source_without_a_template_ref_fails_asking_for_one() {
        let d = tempfile::tempdir().unwrap();
        let r = fake();
        let mut m = manifest("gh:x/y");
        m.template_ref = None;
        let mut cx = ctx(
            &r,
            d.path().to_path_buf(),
            m,
            PINS,
            Opts::default(),
            &no_sleep,
            &no_stdin,
        );
        let Check::Failed(e) = Render.apply(&mut cx).unwrap() else {
            panic!()
        };
        assert!(
            e.contains("--template-ref") && e.contains("template_ref"),
            "{e}"
        );
        assert!(lines(&r).is_empty(), "nothing runs before the ref is known");
    }

    #[test]
    fn splice_conflict_fails() {
        let d = tempfile::tempdir().unwrap();
        let r = fake().on(
            &["splice", "--json", "--yes", "new"],
            Out::with(1, "", "conflict"),
        );
        assert!(
            matches!(apply(&r, d.path(), "rust"), Check::Failed(m) if m.contains("splice --json --yes new"))
        );
    }

    #[test]
    fn cargo_lock_generated_when_missing() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join(".splice")).unwrap();
        std::fs::write(d.path().join(".splice/lock.toml"), "").unwrap();
        std::fs::write(d.path().join("Cargo.toml"), "").unwrap();
        let r = fake();
        let _ = apply(&r, d.path(), "rust");
        let l = lines(&r);
        assert!(l.contains(&"cargo generate-lockfile".to_string()));
        assert!(!l.iter().any(|x| x.starts_with("splice")), "{l:?}");
        // On the work branch, committing only Cargo.lock (and the .keel ignore).
        let pos = |p: &str| {
            l.iter()
                .position(|x| x == p)
                .unwrap_or_else(|| panic!("{p} in {l:?}"))
        };
        assert!(pos("git checkout -b keel/bootstrap") < pos("cargo generate-lockfile"));
        assert!(l.contains(&"git add -- Cargo.lock".to_string()));
        assert!(!l.iter().any(|x| x.starts_with("git add -A")), "{l:?}");
    }

    #[test]
    fn keel_dir_is_gitignored_and_committed_with_the_render() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(".gitignore"), "target\n").unwrap();
        let r = fake();
        let _ = apply(&r, d.path(), "rust");
        let g = std::fs::read_to_string(d.path().join(".gitignore")).unwrap();
        assert!(g.lines().any(|l| l == ".keel/"), "{g}");
        let l = lines(&r);
        let p = |s: &str| l.iter().position(|x| x == s).unwrap();
        assert!(
            p(
                "splice --json --yes new gh:keelstackhq/hull//template-rust . --ref v0.13.0 --force --piece rust-cli:my-cli --data no_placeholder=true"
            ) < p("git add -- .splice/lock.toml src/main.rs .gitignore")
        );
    }

    #[test]
    fn full_source_of_the_rust_template_gets_the_rust_flags() {
        let d = tempfile::tempdir().unwrap();
        let r = fake();
        let _ = apply(&r, d.path(), "gh:keelstackhq/hull//template-rust");
        assert!(
            lines(&r)
                .iter()
                .any(|l| l.starts_with("splice --json --yes new")
                    && l.ends_with("--data no_placeholder=true"))
        );
    }
}
