use super::failure;
use crate::exec::{Cmd, Runner};
use std::path::Path;

fn git(r: &dyn Runner, dir: &Path, args: &[&str]) -> crate::exec::Out {
    r.run(&Cmd::new("git", args).dir(dir))
}

pub fn version(r: &dyn Runner) -> Option<String> {
    let o = r.run(&Cmd::new("git", &["--version"]));
    (o.code == 0).then(|| {
        o.stdout
            .trim()
            .trim_start_matches("git version ")
            .to_string()
    })
}

pub fn is_work_tree(r: &dyn Runner, dir: &Path) -> bool {
    git(r, dir, &["rev-parse", "--is-inside-work-tree"]).code == 0
}

pub fn origin(r: &dyn Runner, dir: &Path) -> Option<String> {
    let o = git(r, dir, &["remote", "get-url", "origin"]);
    (o.code == 0).then(|| o.stdout.trim().to_string())
}

/// Does a remote URL (https, ssh or a proxy path) point at `repo` (OWNER/NAME)?
pub fn origin_matches(url: &str, repo: &str) -> bool {
    let u = url.trim().trim_end_matches('/').to_lowercase();
    let u = u.strip_suffix(".git").unwrap_or(&u);
    let repo = repo.to_lowercase();
    u.strip_suffix(&repo)
        .is_some_and(|rest| rest.ends_with('/') || rest.ends_with(':'))
}

pub fn current_branch(r: &dyn Runner, dir: &Path) -> Result<String, String> {
    let c = Cmd::new("git", &["rev-parse", "--abbrev-ref", "HEAD"]).dir(dir);
    let o = r.run(&c);
    if o.code != 0 {
        return Err(failure(&c, &o));
    }
    Ok(o.stdout.trim().to_string())
}

/// §5.1: stay on the current branch unless it is the default branch (or detached); then use
/// `branch`, created from the default branch (or checked out when it already exists).
pub fn ensure_branch(
    r: &dyn Runner,
    dir: &Path,
    default_branch: &str,
    branch: &str,
) -> Result<String, String> {
    let cur = current_branch(r, dir)?;
    if cur != default_branch && cur != "HEAD" && !cur.is_empty() {
        return Ok(cur);
    }
    let exists = git(
        r,
        dir,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .code
        == 0;
    let c = if exists {
        Cmd::new("git", &["checkout", branch])
    } else {
        Cmd::new("git", &["checkout", "-b", branch])
    }
    .dir(dir);
    let o = r.run(&c);
    if o.code != 0 {
        return Err(failure(&c, &o));
    }
    Ok(branch.to_string())
}

/// `git add -A` then commit; "nothing to commit" is fine.
pub fn commit_all(r: &dyn Runner, dir: &Path, message: &str) -> Result<(), String> {
    let add = Cmd::new("git", &["add", "-A"]).dir(dir);
    let o = r.run(&add);
    if o.code != 0 {
        return Err(failure(&add, &o));
    }
    let c = Cmd::new("git", &["commit", "-m", message]).dir(dir);
    let o = r.run(&c);
    if o.code != 0 && !o.stdout.contains("nothing to commit") {
        return Err(failure(&c, &o));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_forms() {
        for u in [
            "https://github.com/Owner/Name.git",
            "git@github.com:owner/name.git",
            "http://local_proxy@127.0.0.1:1234/git/owner/name",
            "https://github.com/owner/name/",
        ] {
            assert!(origin_matches(u, "owner/name"), "{u}");
        }
        assert!(!origin_matches(
            "https://github.com/owner/other-name",
            "owner/name"
        ));
        assert!(!origin_matches(
            "https://github.com/xowner/name",
            "owner/name"
        ));
    }
}
