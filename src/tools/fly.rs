//! `fly` (flyctl) adapter and the hull `scripts/fly-setup.sh` helper. keel never sees a Fly token:
//! the script pipes the deploy token into `gh secret set` itself.
use crate::exec::{Cmd, Out, Runner};
use serde_json::Value;
use std::path::Path;

pub const SETUP_SCRIPT: &str = "scripts/fly-setup.sh";

fn ok(r: &dyn Runner, dir: &Path, args: &[&str]) -> bool {
    r.run(&Cmd::new("fly", args).dir(dir)).code == 0
}

/// `fly auth whoami --json` exits 0 (logged in, or FLY_API_TOKEN is valid).
pub fn logged_in(r: &dyn Runner, dir: &Path) -> bool {
    ok(r, dir, &["auth", "whoami", "--json"])
}

/// `fly status --app APP --json` exits 0: the app exists.
pub fn app_exists(r: &dyn Runner, dir: &Path, app: &str) -> bool {
    ok(r, dir, &["status", "--app", app, "--json"])
}

/// `fly certs show HOST --app APP --json` exits 0: the certificate exists.
pub fn cert_exists(r: &dyn Runner, dir: &Path, app: &str, host: &str) -> bool {
    ok(r, dir, &["certs", "show", host, "--app", app, "--json"])
}

/// Addresses from `fly ips list --app APP --json`: a JSON array of `{Address, Type}`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ips {
    pub v4: Vec<String>,
    pub v6: Vec<String>,
}

pub fn parse_ips(stdout: &str) -> Option<Ips> {
    let v: Value = serde_json::from_str(stdout.trim()).ok()?;
    let mut ips = Ips::default();
    for i in v.as_array()? {
        let (Some(a), Some(t)) = (i["Address"].as_str(), i["Type"].as_str()) else {
            continue;
        };
        match t {
            "v4" => ips.v4.push(a.to_string()),
            "v6" => ips.v6.push(a.to_string()),
            _ => {} // shared_v4 and private addresses are not DNS targets
        }
    }
    Some(ips)
}

pub fn ips(r: &dyn Runner, dir: &Path, app: &str) -> Result<Ips, String> {
    let c = Cmd::new("fly", &["ips", "list", "--app", app, "--json"]).dir(dir);
    let o = r.run(&c);
    if o.code != 0 {
        return Err(super::failure(&c, &o));
    }
    parse_ips(&o.stdout).ok_or_else(|| super::not_json(&c))
}

pub fn setup_cmd(dir: &Path, args: &[&str]) -> Cmd {
    Cmd::new(SETUP_SCRIPT, args).dir(dir)
}

/// `scripts/fly-setup.sh --app APP --region R --host H`: app, volume, secrets, IPs, certificate.
pub fn setup(r: &dyn Runner, dir: &Path, app: &str, region: &str, host: &str) -> Out {
    r.run(&setup_cmd(
        dir,
        &["--app", app, "--region", region, "--host", host],
    ))
}

/// `scripts/fly-setup.sh --app APP --set-gh-secret ENV`: the script stores the deploy token as
/// the `FLY_API_TOKEN` environment secret of the repo.
pub fn set_gh_secret(r: &dyn Runner, dir: &Path, app: &str, env: &str) -> Out {
    r.run(&setup_cmd(dir, &["--app", app, "--set-gh-secret", env]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_v4_and_v6_only() {
        let ips = parse_ips(
            r#"[{"Address":"1.2.3.4","Type":"v4"},{"Address":"2a09::1","Type":"v6"},{"Address":"9.9.9.9","Type":"shared_v4"},{"Address":"fdaa::1","Type":"private_v6"}]"#,
        )
        .unwrap();
        assert_eq!(ips.v4, ["1.2.3.4"]);
        assert_eq!(ips.v6, ["2a09::1"]);
        assert!(parse_ips("nope").is_none());
        assert!(parse_ips("{}").is_none());
    }
}
