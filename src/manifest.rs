use crate::cli::UpArgs;
use crate::error::KeelError;
use crate::pins::Pins;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const FILE: &str = "keel.toml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Token {
    #[serde(default = "default_secret")]
    pub secret: String,
    #[serde(default = "default_environment")]
    pub environment: String,
}

fn default_secret() -> String {
    "SPLICE_BOT_TOKEN".into()
}
fn default_environment() -> String {
    "splice-bot".into()
}

impl Default for Token {
    fn default() -> Token {
        Token {
            secret: default_secret(),
            environment: default_environment(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dns {
    Cloudflare,
    #[default]
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fly {
    pub app: String,
    pub region: String,
    /// Public hostname, no scheme.
    pub host: String,
    #[serde(default)]
    pub dns: Dns,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deploy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fly: Option<Fly>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    R2,
    S3,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backup {
    pub provider: Provider,
    #[serde(default = "default_retention")]
    pub retention_days: u32,
}

fn default_retention() -> u32 {
    30
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Confirmed {
    #[serde(default)]
    pub agent_env: bool,
    /// The DNS records were added by hand (`--confirm dns`).
    #[serde(default)]
    pub dns: bool,
    /// The backup bucket was created by hand (`--confirm backup`).
    #[serde(default)]
    pub backup: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pieces: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default)]
    pub token: Token,
    #[serde(default)]
    pub confirmed: Confirmed,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deploy: Option<Deploy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<Backup>,
}

fn manifest_err(message: impl Into<String>) -> KeelError {
    KeelError::Manifest {
        file: FILE.into(),
        message: message.into(),
    }
}

impl Manifest {
    pub const DEFAULT_BRANCH: &'static str = "keel/bootstrap";

    /// `dir/keel.toml`, or the empty manifest when the file does not exist.
    pub fn load(dir: &Path) -> Result<Manifest, KeelError> {
        let path = dir.join(FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => Manifest::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Manifest::default()),
            Err(e) => Err(manifest_err(format!("cannot read it: {e}"))),
        }
    }

    pub fn parse(text: &str) -> Result<Manifest, KeelError> {
        let m: Manifest =
            toml::from_str(text).map_err(|e| manifest_err(e.message().trim().to_string()))?;
        m.validate()?;
        Ok(m)
    }

    fn validate(&self) -> Result<(), KeelError> {
        if let Some(f) = self.fly() {
            for (key, v) in [("app", &f.app), ("region", &f.region), ("host", &f.host)] {
                if v.trim().is_empty() {
                    return Err(manifest_err(format!(
                        "[deploy.fly] `{key}` must not be empty"
                    )));
                }
            }
            if f.host.contains("://") || f.host.contains('/') {
                return Err(manifest_err(
                    "[deploy.fly] `host` is a bare hostname such as app.example.com (no scheme or path)",
                ));
            }
        }
        if let Some(b) = &self.backup {
            if self.fly().is_none() {
                return Err(manifest_err(
                    "[backup] needs [deploy.fly] (the backup secrets go to the fly app): add a [deploy.fly] table or remove [backup]",
                ));
            }
            if b.retention_days < 1 {
                return Err(manifest_err("[backup] `retention_days` must be at least 1"));
            }
        }
        Ok(())
    }

    /// The `[deploy.fly]` table, if any.
    pub fn fly(&self) -> Option<&Fly> {
        self.deploy.as_ref().and_then(|d| d.fly.as_ref())
    }

    pub fn exists(dir: &Path) -> bool {
        dir.join(FILE).is_file()
    }

    pub fn write(&self, dir: &Path) -> Result<(), KeelError> {
        if let Some(r) = &self.repo {
            validate_repo(r)?;
        }
        let text = toml::to_string(self).map_err(|e| manifest_err(e.to_string()))?;
        // Atomic: a temp file in the same directory, then a rename over keel.toml.
        let tmp = dir.join(format!(".{FILE}.{}.tmp", std::process::id()));
        let res = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, dir.join(FILE)));
        if let Err(e) = res {
            let _ = std::fs::remove_file(&tmp);
            return Err(manifest_err(format!("cannot write it: {e}")));
        }
        Ok(())
    }

    /// For the `rust`/`ts` templates without a `template_ref`, take the `hull` pin. Returns
    /// whether the manifest changed. A custom source gets no default: it needs an explicit ref.
    pub fn default_template_ref(&mut self, pins: &Pins) -> bool {
        let hull = matches!(self.template.as_deref(), Some("rust" | "ts"));
        match pins.version("hull") {
            Some(v) if hull && self.template_ref.is_none() => {
                self.template_ref = Some(v.to_string());
                true
            }
            _ => false,
        }
    }

    pub fn branch_name(&self) -> &str {
        self.branch.as_deref().unwrap_or(Self::DEFAULT_BRANCH)
    }

    /// Fold the flags into the manifest. A flag that disagrees with a value already in the
    /// manifest is a usage error naming the key. Returns whether the manifest changed.
    pub fn merge_flags(&mut self, args: &UpArgs) -> Result<bool, KeelError> {
        let mut changed = false;
        if let Some(r) = &args.repo {
            validate_repo(r)?;
        }
        changed |= merge_opt("repo", &mut self.repo, args.repo.as_deref())?;
        changed |= merge_opt("template", &mut self.template, args.template.as_deref())?;
        changed |= merge_opt(
            "template-ref",
            &mut self.template_ref,
            args.template_ref.as_deref(),
        )?;
        changed |= merge_opt("plan", &mut self.plan, args.plan.as_deref())?;
        if let Some(p) = &args.pieces {
            let pieces = parse_pieces(p)?;
            if self.pieces.is_empty() {
                self.pieces = pieces;
                changed = true;
            } else if self.pieces != pieces {
                return Err(conflict(
                    "pieces",
                    &self.pieces.join(","),
                    &pieces.join(","),
                ));
            }
        }
        for step in &args.confirm {
            match step.as_str() {
                "agent-env" => {
                    changed |= !self.confirmed.agent_env;
                    self.confirmed.agent_env = true;
                }
                "dns" => {
                    changed |= !self.confirmed.dns;
                    self.confirmed.dns = true;
                }
                "backup" => {
                    changed |= !self.confirmed.backup;
                    self.confirmed.backup = true;
                }
                other => {
                    return Err(KeelError::Usage(format!(
                        "--confirm {other}: unknown step; the steps that take it are `agent-env`, `dns` and `backup`"
                    )));
                }
            }
        }
        Ok(changed)
    }
}

/// `OWNER/NAME`: `^[A-Za-z0-9-]+/[A-Za-z0-9._-]+$` and no `..`.
pub fn validate_repo(repo: &str) -> Result<(), KeelError> {
    let ok = repo.split_once('/').is_some_and(|(owner, name)| {
        !owner.is_empty()
            && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !name.is_empty()
            && name != "."
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    }) && !repo.contains("..");
    if ok {
        Ok(())
    } else {
        Err(KeelError::Usage(format!(
            "--repo {repo:?}: expected OWNER/NAME (letters, digits, `-`; the name may also have `.` and `_`), e.g. keelstackhq/my-app"
        )))
    }
}

fn conflict(key: &str, have: &str, flag: &str) -> KeelError {
    KeelError::Usage(format!(
        "--{key} {flag} disagrees with `{key} = {have}` in {FILE}: change the flag or edit {FILE}"
    ))
}

fn merge_opt(key: &str, field: &mut Option<String>, flag: Option<&str>) -> Result<bool, KeelError> {
    match (field.as_deref(), flag) {
        (_, None) => Ok(false),
        (None, Some(f)) => {
            *field = Some(f.to_string());
            Ok(true)
        }
        (Some(have), Some(f)) if have == f => Ok(false),
        (Some(have), Some(f)) => Err(conflict(key, have, f)),
    }
}

/// `ID[:NAME],ID[:NAME]...`
pub fn parse_pieces(s: &str) -> Result<Vec<String>, KeelError> {
    let pieces: Vec<String> = s.split(',').map(|p| p.trim().to_string()).collect();
    if pieces.iter().any(|p| p.is_empty() || p.starts_with(':')) {
        return Err(KeelError::Usage(format!(
            "--pieces {s:?}: expected a comma list of ID or ID:NAME, e.g. rust-cli:my-cli"
        )));
    }
    Ok(pieces)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> UpArgs {
        UpArgs::default()
    }

    #[test]
    fn parses_full_manifest_with_defaults() {
        let m = Manifest::parse(
            "repo = \"o/n\"\ntemplate = \"rust\"\npieces = [\"rust-cli:x\"]\n[confirmed]\nagent_env = true\n",
        )
        .unwrap();
        assert_eq!(m.repo.as_deref(), Some("o/n"));
        assert_eq!(m.token.secret, "SPLICE_BOT_TOKEN");
        assert_eq!(m.token.environment, "splice-bot");
        assert!(m.confirmed.agent_env);
        assert_eq!(m.branch_name(), "keel/bootstrap");
    }

    #[test]
    fn unknown_key_is_a_manifest_error_naming_it() {
        let e = Manifest::parse("repo = \"o/n\"\nreop = 1\n").unwrap_err();
        assert!(matches!(e, KeelError::Manifest { .. }));
        assert!(e.to_string().contains("reop"), "{e}");
        let e = Manifest::parse("[token]\nsecrt = \"x\"\n").unwrap_err();
        assert!(e.to_string().contains("secrt"), "{e}");
    }

    const FLY: &str = "[deploy.fly]\napp = \"a\"\nregion = \"fra\"\nhost = \"app.example.com\"\n";

    #[test]
    fn deploy_and_backup_tables_parse_with_defaults_and_round_trip() {
        let m = Manifest::parse(&format!("{FLY}[backup]\nprovider = \"r2\"\n")).unwrap();
        let f = m.fly().unwrap();
        assert_eq!((f.app.as_str(), f.dns), ("a", Dns::None));
        let b = m.backup.as_ref().unwrap();
        assert_eq!((b.provider, b.retention_days), (Provider::R2, 30));
        let dir = tempfile::tempdir().unwrap();
        m.write(dir.path()).unwrap();
        assert_eq!(Manifest::load(dir.path()).unwrap(), m);
        assert!(Manifest::parse("").unwrap().fly().is_none());
    }

    #[test]
    fn deploy_and_backup_errors_name_the_key() {
        for (text, want) in [
            (format!("{FLY}dns = \"route53\"\n"), "route53"),
            (format!("{FLY}zone = \"x\"\n"), "zone"),
            (format!("{FLY}[backup]\nprovider = \"gcs\"\n"), "gcs"),
            (
                format!("{FLY}[backup]\nprovider = \"r2\"\nretention = 3\n"),
                "retention",
            ),
            (
                format!("{FLY}[backup]\nprovider = \"r2\"\nretention_days = 0\n"),
                "retention_days",
            ),
            ("[backup]\nprovider = \"r2\"\n".to_string(), "[deploy.fly]"),
            (
                "[deploy.fly]\napp = \"a\"\nregion = \"fra\"\n".to_string(),
                "host",
            ),
            (FLY.replace("app.example.com", "https://x.io"), "host"),
        ] {
            let e = Manifest::parse(&text).unwrap_err();
            assert!(matches!(e, KeelError::Manifest { .. }), "{text}");
            assert_eq!(e.exit_code(), 3);
            assert!(e.to_string().contains(want), "{text}: {e}");
        }
    }

    #[test]
    fn flags_fill_an_empty_manifest_and_round_trip() {
        let mut m = Manifest::default();
        let a = UpArgs {
            repo: Some("o/n".into()),
            template: Some("rust".into()),
            pieces: Some("rust-cli:my-cli, ts-lib".into()),
            ..args()
        };
        assert!(m.merge_flags(&a).unwrap());
        assert_eq!(m.pieces, ["rust-cli:my-cli", "ts-lib"]);
        let dir = tempfile::tempdir().unwrap();
        m.write(dir.path()).unwrap();
        assert_eq!(Manifest::load(dir.path()).unwrap(), m);
        // Same flags again: no change.
        assert!(!m.merge_flags(&a).unwrap());
    }

    #[test]
    fn disagreeing_flag_is_a_usage_error_naming_the_key() {
        let mut m = Manifest::parse("repo = \"o/n\"\ntemplate = \"rust\"\n").unwrap();
        let e = m
            .merge_flags(&UpArgs {
                template: Some("ts".into()),
                ..args()
            })
            .unwrap_err();
        assert!(matches!(e, KeelError::Usage(_)));
        assert!(e.to_string().contains("template"), "{e}");
        let e = m
            .merge_flags(&UpArgs {
                repo: Some("o/other".into()),
                ..args()
            })
            .unwrap_err();
        assert!(e.to_string().contains("repo"), "{e}");
    }

    #[test]
    fn pieces_conflict_and_bad_list() {
        let mut m = Manifest::parse("pieces = [\"a\"]\n").unwrap();
        let e = m
            .merge_flags(&UpArgs {
                pieces: Some("b".into()),
                ..args()
            })
            .unwrap_err();
        assert!(e.to_string().contains("pieces"), "{e}");
        assert!(parse_pieces("a,,b").is_err());
    }

    #[test]
    fn confirm_agent_env_is_recorded_and_others_rejected() {
        let mut m = Manifest::default();
        assert!(
            m.merge_flags(&UpArgs {
                confirm: vec!["agent-env".into()],
                ..args()
            })
            .unwrap()
        );
        assert!(m.confirmed.agent_env);
        assert!(
            m.merge_flags(&UpArgs {
                confirm: vec!["dns".into(), "backup".into()],
                ..args()
            })
            .unwrap()
        );
        assert!(m.confirmed.dns && m.confirmed.backup);
        assert!(
            m.merge_flags(&UpArgs {
                confirm: vec!["merge".into()],
                ..args()
            })
            .is_err()
        );
    }

    #[test]
    fn missing_file_loads_default() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Manifest::load(dir.path()).unwrap(), Manifest::default());
        assert!(!Manifest::exists(dir.path()));
    }

    #[test]
    fn repo_must_be_owner_slash_name() {
        for ok in ["o/n", "Key-stone/my_app.rs", "a1/b-2"] {
            validate_repo(ok).unwrap();
        }
        for bad in [
            "", "o", "o/", "/n", "o/n/x", "o/..", "o/a..b", "o/.", "o_x/n", "o/n x", "o/n;rm",
            "../n", "o/n\n",
        ] {
            let e = Manifest::default()
                .merge_flags(&UpArgs {
                    repo: Some(bad.into()),
                    ..args()
                })
                .unwrap_err();
            assert!(matches!(e, KeelError::Usage(_)), "{bad:?}");
            assert_eq!(e.exit_code(), 2);
        }
    }

    #[test]
    fn write_is_atomic_and_leaves_no_temp_file() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(FILE), "repo = \"old/old\"\n").unwrap();
        let m = Manifest {
            repo: Some("o/n".into()),
            ..Manifest::default()
        };
        m.write(d.path()).unwrap();
        assert_eq!(Manifest::load(d.path()).unwrap(), m);
        let names: Vec<_> = std::fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, [FILE]);
        // A bad repo never reaches the disk.
        let bad = Manifest {
            repo: Some("../x".into()),
            ..Manifest::default()
        };
        assert!(matches!(bad.write(d.path()), Err(KeelError::Usage(_))));
        assert_eq!(Manifest::load(d.path()).unwrap(), m);
    }

    #[test]
    fn template_ref_comes_from_the_hull_pin_for_hull_templates_only() {
        let pins = Pins::from_texts("t", "[pins]\nhull = \"v1\"\n", None).unwrap();
        let mut m = Manifest::parse("template = \"rust\"\n").unwrap();
        assert!(m.default_template_ref(&pins));
        assert_eq!(m.template_ref.as_deref(), Some("v1"));
        assert!(!m.default_template_ref(&pins));
        let mut m = Manifest::parse("template = \"gh:x/y\"\n").unwrap();
        assert!(!m.default_template_ref(&pins));
        assert_eq!(m.template_ref, None);
        m.merge_flags(&UpArgs {
            template_ref: Some("v9".into()),
            ..args()
        })
        .unwrap();
        assert_eq!(m.template_ref.as_deref(), Some("v9"));
        // A repo pins.toml `hull` overrides the embedded pin.
        let pins = Pins::from_texts(
            "t",
            "[pins]\nhull = \"v1\"\n",
            Some("[pins]\nhull = \"v2\"\n"),
        )
        .unwrap();
        let mut m = Manifest::parse("template = \"ts\"\n").unwrap();
        m.default_template_ref(&pins);
        assert_eq!(m.template_ref.as_deref(), Some("v2"));
    }

    #[test]
    fn embedded_pins_pin_hull_to_v0_13_0() {
        let pins = Pins::from_texts("e", crate::pins::EMBEDDED, None).unwrap();
        assert_eq!(pins.version("hull"), Some("v0.13.0"));
    }
}
