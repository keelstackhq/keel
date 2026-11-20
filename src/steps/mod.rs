//! The steps of `keel up` (spec §5), in order.
use crate::ctx::Ctx;
use crate::error::KeelError;
use crate::step::Step;

pub mod apps;
pub mod governance;
pub mod preflight;
pub mod render;
pub mod settings;
pub mod token;

/// Steps 1-6; the agent-env, doctor and merge steps are added after them.
pub fn all_steps() -> Vec<Box<dyn Step>> {
    vec![
        Box::new(preflight::Preflight),
        Box::new(apps::Apps),
        Box::new(render::Render),
        Box::new(governance::Governance),
        Box::new(settings::Settings),
        Box::new(token::Token),
    ]
}

pub(crate) fn repo(cx: &Ctx) -> Result<String, KeelError> {
    cx.manifest.repo.clone().ok_or_else(|| {
        KeelError::Env("no repo: pass --repo OWNER/NAME or set `repo` in keel.toml".into())
    })
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

    pub const PINS: &str = "[pins]\nsplice = \"0.6.0\"\nbosun = \"0.21.0\"\nhoist = \"0.5.1\"\n";

    pub fn manifest(template: &str) -> Manifest {
        Manifest {
            repo: Some("owner/name".into()),
            template: Some(template.into()),
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
