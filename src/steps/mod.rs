//! The steps of `keel up` (spec §5), in order.
use crate::ctx::Ctx;
use crate::error::KeelError;
use crate::report::HumanStep;
use crate::step::{Check, Step};

pub mod agent_env;
pub mod apps;
pub mod deploy;
pub mod dns;
pub mod doctor;
pub mod governance;
pub mod merge;
pub mod preflight;
pub mod render;
pub mod settings;
pub mod token;

/// All steps, in order.
pub fn all_steps() -> Vec<Box<dyn Step>> {
    vec![
        Box::new(preflight::Preflight),
        Box::new(apps::Apps),
        Box::new(render::Render),
        Box::new(governance::Governance),
        Box::new(settings::Settings),
        Box::new(token::Token),
        Box::new(agent_env::AgentEnv),
        Box::new(deploy::Deploy),
        Box::new(dns::Dns),
        Box::new(doctor::Doctor),
        Box::new(merge::Merge),
    ]
}

/// A hoist `--only SECTION` outcome as a step result. `Done` carries `done_msg`.
pub(crate) fn hoist_check(
    res: crate::tools::hoist::Res,
    id: &str,
    only: &str,
    done_msg: &str,
) -> Check {
    use crate::tools::hoist::Res;
    match res {
        Res::Ok => Check::Done(done_msg.into()),
        Res::Drift => Check::Todo(format!("hoist reports drift in [{only}]")),
        Res::Human {
            title,
            url,
            command,
        } => Check::Human(HumanStep {
            id: id.into(),
            title,
            url,
            command,
        }),
        Res::Failed(e) => Check::Failed(e),
        Res::Upgrade => Check::Human(HumanStep {
            id: id.into(),
            title: format!("Upgrade hoist to a release with [{only}]"),
            url: String::new(),
            command: "npm install -g @keelstackhq/hoist@latest".into(),
        }),
    }
}

/// The human step for a Cloudflare step without `CLOUDFLARE_API_TOKEN`.
pub(crate) fn cloudflare_token_step(id: &str, what: &str) -> Check {
    Check::Human(HumanStep {
        id: id.into(),
        title: format!("Set CLOUDFLARE_API_TOKEN or {what}"),
        url: "https://dash.cloudflare.com/profile/api-tokens".into(),
        command: "export CLOUDFLARE_API_TOKEN=... then keel up".into(),
    })
}

pub(crate) fn repo(cx: &Ctx) -> Result<String, KeelError> {
    cx.manifest.repo.clone().ok_or_else(|| {
        KeelError::Env("no repo: pass --repo OWNER/NAME or set `repo` in keel.toml".into())
    })
}

/// The branch steps 3-9 work on (§5.1), without switching: the current branch unless it is the
/// default branch or HEAD is detached; then the manifest's `branch`.
pub(crate) fn work_branch(cx: &Ctx) -> Result<String, String> {
    let repo = repo(cx).map_err(|e| e.to_string())?;
    let default = crate::tools::gh::default_branch(cx.runner, &repo)?;
    let cur = crate::tools::git::current_branch(cx.runner, &cx.dir)?;
    Ok(crate::tools::git::resolve_branch(
        &cur,
        &default,
        cx.manifest.branch_name(),
    ))
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;
    use crate::ctx::Opts;
    use crate::exec::{FakeRunner, Out};

    #[test]
    fn work_branch_follows_5_1() {
        for (head, want) in [
            ("main\n", "keel/bootstrap"),
            ("HEAD\n", "keel/bootstrap"),
            ("claude/work\n", "claude/work"),
        ] {
            let r = FakeRunner::new()
                .on(
                    &["gh", "repo", "view"],
                    Out::ok(r#"{"defaultBranchRef":{"name":"main"}}"#),
                )
                .on(&["git", "rev-parse", "--abbrev-ref", "HEAD"], Out::ok(head));
            let cx = ctx(
                &r,
                ".".into(),
                manifest("rust"),
                PINS,
                Opts::default(),
                &no_sleep,
                &no_stdin,
            );
            assert_eq!(work_branch(&cx).unwrap(), want, "{head}");
        }
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use crate::ctx::{Ctx, Opts};
    use crate::exec::FakeRunner;
    use crate::manifest::Manifest;
    use crate::pins::Pins;
    use std::path::PathBuf;
    use std::time::Duration;
    use zeroize::Zeroizing;

    pub const PINS: &str = "[pins]\nsplice = \"0.6.0\"\nbosun = \"0.21.0\"\nhoist = \"0.5.1\"\nberth = \"0.2.1\"\n\n[refs.hull]\ntag = \"v0.13.0\"\nsha = \"3f4a65fda568cdd20c76168ed156d25a28ba5630\"\n";

    pub fn manifest(template: &str) -> Manifest {
        Manifest {
            repo: Some("owner/name".into()),
            template: Some(template.into()),
            template_ref: Some("v0.13.0".into()),
            pieces: vec!["rust-cli:my-cli".into()],
            ..Manifest::default()
        }
    }

    pub fn ctx<'a>(
        r: &'a FakeRunner,
        dir: PathBuf,
        manifest: Manifest,
        pins_text: &str,
        opts: Opts,
        sleep: &'a dyn Fn(Duration),
        stdin: &'a dyn Fn() -> std::io::Result<Zeroizing<String>>,
    ) -> Ctx<'a> {
        Ctx {
            runner: r,
            dir,
            manifest,
            pins: Pins::from_texts("t", pins_text, None).unwrap(),
            opts,
            sleep,
            stdin,
        }
    }

    pub fn no_sleep(_: Duration) {}
    pub fn no_stdin() -> std::io::Result<Zeroizing<String>> {
        Err(std::io::Error::other("no stdin"))
    }
}
