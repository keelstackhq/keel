use crate::error::KeelError;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

pub const EMBEDDED: &str = include_str!("pins.toml");

/// The recorded npm integrity (sha512, base64) of a tool's launcher and platform packages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sha512Set {
    pub launcher: String,
    /// Keyed `linux_x64`, `linux_arm64`, `darwin_arm64`, ...
    pub platform: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPin {
    pub version: String,
    /// None when the version does not match the one the sha512s were recorded for.
    pub sha512: Option<Sha512Set>,
}

#[derive(Debug, Clone, Default)]
pub struct Pins {
    versions: BTreeMap<String, String>,
    sha: BTreeMap<String, (String, Sha512Set)>,
}

fn pins_err(file: &str, message: impl Into<String>) -> KeelError {
    KeelError::Manifest {
        file: file.into(),
        message: message.into(),
    }
}

fn parse_table(file: &str, text: &str) -> Result<toml::Table, KeelError> {
    text.parse::<toml::Table>()
        .map_err(|e| pins_err(file, e.message().trim().to_string()))
}

fn versions_of(file: &str, t: &toml::Table) -> Result<BTreeMap<String, String>, KeelError> {
    let mut out = BTreeMap::new();
    if let Some(p) = t.get("pins") {
        let p = p
            .as_table()
            .ok_or_else(|| pins_err(file, "`pins` must be a table"))?;
        for (k, v) in p {
            if let Some(s) = v.as_str() {
                out.insert(k.clone(), s.to_string());
            }
        }
    }
    Ok(out)
}

impl Pins {
    /// The embedded table, or the file named by `KEEL_PINS` in its place; then `<repo_dir>/pins.toml`
    /// `[pins]` versions override.
    pub fn load(repo_dir: &Path) -> Result<Pins, KeelError> {
        let base = match std::env::var_os("KEEL_PINS") {
            Some(p) => {
                let name = p.to_string_lossy().into_owned();
                let text = std::fs::read_to_string(&p).map_err(|e| {
                    KeelError::Env(format!(
                        "KEEL_PINS={name}: cannot read it ({e}); unset it or point it at a pins.toml"
                    ))
                })?;
                (name, text)
            }
            None => ("pins.toml (embedded)".to_string(), EMBEDDED.to_string()),
        };
        let repo_file = repo_dir.join("pins.toml");
        let repo_text = match std::fs::read_to_string(&repo_file) {
            Ok(t) => Some(t),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(pins_err("pins.toml", format!("cannot read it: {e}"))),
        };
        Pins::from_texts(&base.0, &base.1, repo_text.as_deref())
    }

    pub fn from_texts(
        base_name: &str,
        base: &str,
        repo_override: Option<&str>,
    ) -> Result<Pins, KeelError> {
        let t = parse_table(base_name, base)?;
        let mut pins = Pins {
            versions: versions_of(base_name, &t)?,
            sha: BTreeMap::new(),
        };
        if let Some(s) = t.get("sha512").and_then(|v| v.as_table()) {
            for (tool, v) in s {
                let Some(tbl) = v.as_table() else { continue };
                let get = |k: &str| tbl.get(k).and_then(|x| x.as_str()).map(str::to_string);
                let (Some(version), Some(launcher)) = (get("version"), get("launcher")) else {
                    return Err(pins_err(
                        base_name,
                        format!("[sha512.{tool}] needs `version` and `launcher`"),
                    ));
                };
                let platform = tbl
                    .iter()
                    .filter(|(k, _)| !matches!(k.as_str(), "version" | "launcher"))
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect();
                pins.sha
                    .insert(tool.clone(), (version, Sha512Set { launcher, platform }));
            }
        }
        if let Some(text) = repo_override {
            let t = parse_table("pins.toml", text)?;
            pins.versions.extend(versions_of("pins.toml", &t)?);
        }
        Ok(pins)
    }

    pub fn tool(&self, name: &str) -> Option<ToolPin> {
        let version = self.versions.get(name)?.clone();
        let sha512 = self
            .sha
            .get(name)
            .filter(|(v, _)| *v == version)
            .map(|(_, s)| s.clone());
        Some(ToolPin { version, sha512 })
    }
}

/// `linux_x64`, `darwin_arm64`, `win32_x64`: the key of this platform in `[sha512.<tool>]`.
pub fn platform_key() -> String {
    platform_key_for(std::env::consts::OS, std::env::consts::ARCH)
}

pub fn platform_key_for(os: &str, arch: &str) -> String {
    let os = match os {
        "macos" => "darwin",
        "windows" => "win32",
        o => o,
    };
    let arch = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        a => a,
    };
    format!("{os}_{arch}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_has_all_four_tools_with_sha512() {
        let p = Pins::from_texts("e", EMBEDDED, None).unwrap();
        for (tool, v) in [
            ("splice", "0.6.0"),
            ("hoist", "0.5.1"),
            ("berth", "0.2.1"),
            ("bosun", "0.21.0"),
        ] {
            let t = p.tool(tool).unwrap();
            assert_eq!(t.version, v);
            let s = t.sha512.unwrap();
            assert!(s.launcher.len() > 80);
            assert!(s.platform.contains_key("linux_x64"));
            assert!(s.platform.contains_key("linux_arm64"));
        }
        assert!(p.tool("nope").is_none());
    }

    #[test]
    fn repo_pins_override_version_and_drop_sha_on_mismatch() {
        let repo = "[pins]\nsplice = \"0.7.0\"\nbosun = \"0.21.0\"\nrust = \"1.97\"\nnextest = \"0.9\"\n\n[other]\nx = 1\n";
        let p = Pins::from_texts("e", EMBEDDED, Some(repo)).unwrap();
        let s = p.tool("splice").unwrap();
        assert_eq!(s.version, "0.7.0");
        assert!(s.sha512.is_none());
        assert!(p.tool("bosun").unwrap().sha512.is_some());
        assert!(p.tool("rust").unwrap().sha512.is_none());
        assert_eq!(p.tool("hoist").unwrap().version, "0.5.1");
    }

    #[test]
    fn bad_toml_is_a_manifest_error() {
        assert!(Pins::from_texts("e", "[pins\n", None).is_err());
        assert!(Pins::from_texts("e", EMBEDDED, Some("[pins\n")).is_err());
        assert!(Pins::from_texts("e", "[sha512.x]\nversion=\"1\"\n", None).is_err());
    }

    #[test]
    fn platform_keys() {
        assert_eq!(platform_key_for("linux", "x86_64"), "linux_x64");
        assert_eq!(platform_key_for("linux", "aarch64"), "linux_arm64");
        assert_eq!(platform_key_for("macos", "aarch64"), "darwin_arm64");
        assert_eq!(platform_key_for("windows", "x86_64"), "win32_x64");
    }

    #[test]
    fn load_reads_repo_pins_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pins.toml"), "[pins]\nhoist = \"9.9.9\"\n").unwrap();
        // KEEL_PINS is not set by any test, so this reads the embedded default.
        let p = Pins::load(dir.path()).unwrap();
        assert_eq!(p.tool("hoist").unwrap().version, "9.9.9");
    }
}
