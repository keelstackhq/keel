use super::{repo, work_branch};
use crate::ctx::Ctx;
use crate::error::KeelError;
use crate::report::HumanStep;
use crate::step::{Check, Step};
use crate::tools::gh;

pub struct Merge;

impl Step for Merge {
    fn id(&self) -> &'static str {
        "merge"
    }

    fn check(&self, cx: &mut Ctx) -> Result<Check, KeelError> {
        repo(cx)?;
        let branch = work_branch(cx);
        Ok(match gh::pr_view(cx.runner, &cx.dir, &branch) {
            Some(p) if p.state == "MERGED" => Check::Done(format!("PR #{} is merged", p.number)),
            Some(p) if p.state == "OPEN" => Check::Human(HumanStep {
                id: self.id().into(),
                title: format!("Approve head {} and merge PR #{}", p.head, p.number),
                url: p.url,
                command: "keel up".into(),
            }),
            _ => Check::Todo(format!("wait for the pull request of {branch}")),
        })
    }

    fn apply(&self, cx: &mut Ctx) -> Result<Check, KeelError> {
        let branch = work_branch(cx);
        Ok(Check::Failed(format!(
            "no open pull request for {branch}; run `keel up` to open it"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;
    use crate::ctx::Opts;
    use crate::exec::{FakeRunner, Out};

    fn go(view: Out) -> Check {
        let r = FakeRunner::new()
            .on(
                &["git", "rev-parse", "--abbrev-ref", "HEAD"],
                Out::ok("keel/bootstrap\n"),
            )
            .on(&["gh", "pr", "view", "keel/bootstrap"], view);
        let mut cx = ctx(
            &r,
            ".".into(),
            manifest("rust"),
            PINS,
            Opts::default(),
            &no_sleep,
            &no_stdin,
        );
        Merge.check(&mut cx).unwrap()
    }

    fn pr(state: &str) -> Out {
        Out::ok(&format!(
            r#"{{"number":7,"url":"https://github.com/o/n/pull/7","state":"{state}","headRefOid":"abc123"}}"#
        ))
    }

    #[test]
    fn merged_is_done() {
        assert!(matches!(go(pr("MERGED")), Check::Done(_)));
    }

    #[test]
    fn open_is_a_human_step_with_sha_and_url() {
        let Check::Human(h) = go(pr("OPEN")) else {
            panic!()
        };
        assert_eq!(h.title, "Approve head abc123 and merge PR #7");
        assert_eq!(h.url, "https://github.com/o/n/pull/7");
    }

    #[test]
    fn no_pr_is_todo() {
        assert!(matches!(go(Out::with(1, "", "none")), Check::Todo(_)));
    }
}
