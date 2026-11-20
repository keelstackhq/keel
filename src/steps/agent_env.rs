use crate::ctx::Ctx;
use crate::error::KeelError;
use crate::report::HumanStep;
use crate::step::{Check, Step};
use crate::tools::bosun::{self, Res};

pub struct AgentEnv;

pub const FILE: &str = ".keel/cloud-setup.sh";
const PRELUDE: &str = include_str!("../proxy_ca.sh");

impl Step for AgentEnv {
    fn id(&self) -> &'static str {
        "agent-env"
    }

    fn check(&self, cx: &mut Ctx) -> Result<Check, KeelError> {
        Ok(if cx.manifest.confirmed.agent_env {
            Check::Done("cloud setup script confirmed".into())
        } else {
            Check::Todo(format!("write {FILE}"))
        })
    }

    fn apply(&self, cx: &mut Ctx) -> Result<Check, KeelError> {
        let c = bosun::setup_snippet_cmd(&cx.dir);
        let pin = cx.pins.tool("bosun").map(|p| p.version).unwrap_or_default();
        let data = match bosun::run(cx.runner, &c, &pin) {
            Res::Ok(d) => d,
            Res::Human { title, command } => {
                return Ok(Check::Human(HumanStep {
                    id: self.id().into(),
                    title,
                    url: String::new(),
                    command,
                }));
            }
            Res::Failed(e) => return Ok(Check::Failed(e)),
        };
        let Some(script) = data["script"].as_str().filter(|s| !s.trim().is_empty()) else {
            return Ok(Check::Failed(
                "`bosun --json setup-snippet` returned no `data.script`; upgrade bosun".into(),
            ));
        };
        let path = cx.dir.join(FILE);
        let text = format!("{PRELUDE}\n{script}\n");
        let wrote = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, text));
        if let Err(e) = wrote {
            return Ok(Check::Failed(format!("cannot write {FILE}: {e}")));
        }
        Ok(Check::Human(HumanStep {
            id: self.id().into(),
            title: format!(
                "Paste {FILE} into claude.ai > Code > your environment > Setup script, save it, then confirm"
            ),
            url: "https://claude.ai/code".into(),
            command: "keel up --confirm agent-env".into(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;
    use crate::ctx::Opts;
    use crate::exec::{FakeRunner, Out};

    fn run(r: &FakeRunner, dir: &std::path::Path, confirmed: bool, apply: bool) -> Check {
        let mut m = manifest("rust");
        m.confirmed.agent_env = confirmed;
        let mut cx = ctx(
            r,
            dir.into(),
            m,
            PINS,
            Opts::default(),
            &no_sleep,
            &no_stdin,
        );
        if apply {
            AgentEnv.apply(&mut cx)
        } else {
            AgentEnv.check(&mut cx)
        }
        .unwrap()
    }

    #[test]
    fn confirmed_is_done_else_todo_and_check_writes_nothing() {
        let d = tempfile::tempdir().unwrap();
        let r = FakeRunner::new();
        assert!(matches!(run(&r, d.path(), true, false), Check::Done(_)));
        assert!(matches!(run(&r, d.path(), false, false), Check::Todo(_)));
        assert!(!d.path().join(FILE).exists());
    }

    #[test]
    fn apply_writes_prelude_plus_snippet_and_asks_a_human() {
        let d = tempfile::tempdir().unwrap();
        let r = FakeRunner::new().on(
            &["bosun", "--json", "setup-snippet"],
            Out::ok(
                r#"{"version":1,"command":"setup-snippet","ok":true,"exit_code":0,"data":{"script":"echo BOSUN_SNIPPET"},"error":null,"next":[]}"#,
            ),
        );
        let Check::Human(h) = run(&r, d.path(), false, true) else {
            panic!()
        };
        assert_eq!(h.command, "keel up --confirm agent-env");
        let text = std::fs::read_to_string(d.path().join(FILE)).unwrap();
        assert!(text.contains("proxy-ca.sh") && text.ends_with("echo BOSUN_SNIPPET\n"));
        assert!(text.find("proxy-ca.sh") < text.find("BOSUN_SNIPPET"));
    }

    #[test]
    fn snippet_failure_is_failed_with_its_fix() {
        let d = tempfile::tempdir().unwrap();
        let r = FakeRunner::new().on(
            &["bosun", "--json", "setup-snippet"],
            Out::with(
                1,
                r#"{"ok":false,"error":{"message":"boom","fix":"run bosun init"}}"#,
                "",
            ),
        );
        let Check::Failed(m) = run(&r, d.path(), false, true) else {
            panic!()
        };
        assert!(m.contains("run bosun init"), "{m}");
        assert!(!d.path().join(FILE).exists());
    }

    #[test]
    fn prelude_is_valid_bash_and_never_fails() {
        assert!(PRELUDE.contains("/etc/profile.d/keel-proxy-ca.sh"));
        for v in [
            "NODE_EXTRA_CA_CERTS",
            "SSL_CERT_FILE",
            "REQUESTS_CA_BUNDLE",
            "CARGO_HTTP_CAINFO",
            "GIT_SSL_CAINFO",
            "npm config set cafile",
        ] {
            assert!(PRELUDE.contains(v), "{v}");
        }
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("p.sh");
        std::fs::write(&f, PRELUDE).unwrap();
        match std::process::Command::new("bash")
            .arg("-n")
            .arg(&f)
            .status()
        {
            Ok(s) => assert!(s.success()),
            Err(_) => eprintln!("bash not found; skipped `bash -n`"),
        }
    }
}
