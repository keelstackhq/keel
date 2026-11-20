use crate::cli::UpArgs;
use crate::error::KeelError;
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Confirmed {
    #[serde(default)]
    pub agent_env: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
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
        toml::from_str(text).map_err(|e| manifest_err(e.message().trim().to_string()))
    }

    pub fn exists(dir: &Path) -> bool {
        dir.join(FILE).is_file()
    }

    pub fn write(&self, dir: &Path) -> Result<(), KeelError> {
        let text = toml::to_string(self).map_err(|e| manifest_err(e.to_string()))?;
        std::fs::write(dir.join(FILE), text)
            .map_err(|e| manifest_err(format!("cannot write it: {e}")))
    }

    pub fn branch_name(&self) -> &str {
        self.branch.as_deref().unwrap_or(Self::DEFAULT_BRANCH)
    }

    /// Fold the flags into the manifest. A flag that disagrees with a value already in the
    /// manifest is a usage error naming the key. Returns whether the manifest changed.
    pub fn merge_flags(&mut self, args: &UpArgs) -> Result<bool, KeelError> {
        let mut changed = false;
        changed |= merge_opt("repo", &mut self.repo, args.repo.as_deref())?;
        changed |= merge_opt("template", &mut self.template, args.template.as_deref())?;
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
                other => {
                    return Err(KeelError::Usage(format!(
                        "--confirm {other}: unknown step; the only step that takes it is `agent-env`"
                    )));
                }
            }
        }
        Ok(changed)
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
}
