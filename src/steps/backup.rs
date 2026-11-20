use super::{cloudflare_token_step, hoist_check};
use crate::ctx::Ctx;
use crate::error::KeelError;
use crate::manifest::Provider;
use crate::step::{Check, Step};
use crate::tools::hoist;

pub struct Backup;

const SECTION: &str = "cloudflare.r2";

/// `[cloudflare.r2]`: hoist makes the bucket and its lifecycle rule, and writes a scoped key
/// straight to the fly app's secrets. The key never passes through keel.
fn section_text(app: &str, days: u32) -> String {
    format!(
        "[{SECTION}]\nbucket = \"{app}-backup\"\nlifecycle = {{ expire_days = {days} }}\nkey = {{ scope = \"object-rw\", sink = \"fly:{app}\" }}\n"
    )
}

impl Step for Backup {
    fn id(&self) -> &'static str {
        "backup"
    }

    fn check(&self, cx: &mut Ctx) -> Result<Check, KeelError> {
        let (Some(b), Some(f)) = (cx.manifest.backup.clone(), cx.manifest.fly().cloned()) else {
            return Ok(Check::Done("no [backup]".into()));
        };
        if cx.manifest.confirmed.backup {
            return Ok(Check::Done(
                "backup bucket confirmed as created by hand".into(),
            ));
        }
        if b.provider == Provider::S3 {
            return Ok(Check::Done(format!(
                "s3 backup is not automated: create the bucket and set the secrets on fly app {} by hand",
                f.app
            )));
        }
        if !cx.opts.cloudflare_token {
            let what = format!("create the R2 bucket {}-backup by hand", f.app);
            return Ok(cloudflare_token_step(self.id(), &what));
        }
        if !hoist::has_section(&cx.dir, SECTION) {
            return Ok(Check::Todo(format!(
                "add [{SECTION}] to hoist.toml and apply it (bucket {}-backup)",
                f.app
            )));
        }
        let res = hoist::check_only(cx.runner, &cx.dir, Some(SECTION));
        Ok(hoist_check(
            res,
            self.id(),
            SECTION,
            &format!("R2 bucket {}-backup, key in fly:{}", f.app, f.app),
        ))
    }

    fn apply(&self, cx: &mut Ctx) -> Result<Check, KeelError> {
        let (Some(b), Some(f)) = (cx.manifest.backup.clone(), cx.manifest.fly().cloned()) else {
            return self.check(cx);
        };
        if !hoist::has_section(&cx.dir, SECTION)
            && let Err(e) = hoist::append_section(&cx.dir, &section_text(&f.app, b.retention_days))
        {
            return Ok(Check::Failed(e));
        }
        let res = hoist::apply_only(cx.runner, &cx.dir, Some(SECTION));
        match hoist_check(res, self.id(), SECTION, "applied") {
            Check::Done(_) => self.check(cx),
            other => Ok(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::deploy::tests::fly_manifest;
    use super::super::testutil::*;
    use super::*;
    use crate::ctx::Opts;
    use crate::exec::{FakeRunner, Out};
    use crate::manifest::{Backup as B, Manifest};

    fn with_backup(p: Provider) -> Manifest {
        let mut m = fly_manifest();
        m.backup = Some(B {
            provider: p,
            retention_days: 14,
        });
        m
    }

    fn run(r: &FakeRunner, d: &std::path::Path, m: Manifest, token: bool, apply: bool) -> Check {
        let opts = Opts {
            cloudflare_token: token,
            ..Opts::default()
        };
        let mut cx = ctx(r, d.into(), m, PINS, opts, &no_sleep, &no_stdin);
        if apply {
            Backup.apply(&mut cx)
        } else {
            Backup.check(&mut cx)
        }
        .unwrap()
    }

    #[test]
    fn no_backup_is_done() {
        let d = tempfile::tempdir().unwrap();
        let c = run(&FakeRunner::new(), d.path(), fly_manifest(), true, false);
        assert_eq!(c, Check::Done("no [backup]".into()));
    }

    #[test]
    fn s3_is_done_with_a_by_hand_warning() {
        let d = tempfile::tempdir().unwrap();
        let Check::Done(m) = run(
            &FakeRunner::new(),
            d.path(),
            with_backup(Provider::S3),
            false,
            false,
        ) else {
            panic!()
        };
        assert!(
            m.contains("not automated") && m.contains("fly app app"),
            "{m}"
        );
    }

    #[test]
    fn r2_without_token_is_human() {
        let d = tempfile::tempdir().unwrap();
        let Check::Human(h) = run(
            &FakeRunner::new(),
            d.path(),
            with_backup(Provider::R2),
            false,
            false,
        ) else {
            panic!()
        };
        assert!(
            h.title.starts_with("Set CLOUDFLARE_API_TOKEN or"),
            "{}",
            h.title
        );
        assert_eq!(h.id, "backup");
        assert!(
            h.command.contains("keel up --confirm backup"),
            "{}",
            h.command
        );
    }

    #[test]
    fn confirmed_backup_is_done() {
        let d = tempfile::tempdir().unwrap();
        let mut m = with_backup(Provider::R2);
        m.confirmed.backup = true;
        let c = run(&FakeRunner::new(), d.path(), m, false, false);
        assert!(matches!(c, Check::Done(m) if m.contains("confirmed")));
    }

    #[test]
    fn apply_writes_the_r2_section_and_applies_only_it() {
        let d = tempfile::tempdir().unwrap();
        let r = FakeRunner::new()
            .on(
                &["hoist", "--json", "apply"],
                Out::ok(r#"{"ok":true,"items":[]}"#),
            )
            .on(&["hoist", "--json", "check"], Out::ok(r#"{"ok":true}"#));
        assert!(matches!(
            run(&r, d.path(), with_backup(Provider::R2), true, true),
            Check::Done(_)
        ));
        let t = std::fs::read_to_string(d.path().join("hoist.toml")).unwrap();
        assert_eq!(
            t,
            "[cloudflare.r2]\nbucket = \"app-backup\"\nlifecycle = { expire_days = 14 }\nkey = { scope = \"object-rw\", sink = \"fly:app\" }\n"
        );
        assert_eq!(
            r.calls()[0].line(),
            "hoist --json apply --yes --only cloudflare.r2"
        );
        // The check is read-only: no section, no write.
        let d2 = tempfile::tempdir().unwrap();
        let c = run(
            &FakeRunner::new(),
            d2.path(),
            with_backup(Provider::R2),
            true,
            false,
        );
        assert!(matches!(c, Check::Todo(_)));
        assert!(!d2.path().join("hoist.toml").exists());
    }

    #[test]
    fn old_hoist_is_an_upgrade_human_step() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("hoist.toml"), "[cloudflare.r2]\n").unwrap();
        let r = FakeRunner::new().on(&["hoist"], Out::with(2, "", "unexpected argument '--only'"));
        let Check::Human(h) = run(&r, d.path(), with_backup(Provider::R2), true, false) else {
            panic!()
        };
        assert!(h.title.contains("[cloudflare.r2]"));
    }
}
