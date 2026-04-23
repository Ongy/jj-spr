use std::process::Command;

#[derive(Debug, clap::Parser, Default)]
pub struct ListOptions {}

pub async fn list<GH, PR>(
    jj: &mut crate::jj::Jujutsu,
    mut gh: GH,
    config: &crate::config::Config,
    _: ListOptions,
) -> crate::error::Result<()>
where
    PR: crate::github::GHPullRequest,
    GH: crate::github::GitHubAdapter<PRAdapter = PR>,
{
    let revset = crate::jj::RevSet::mutable();
    let revisions = jj.read_revision_range(
        &revset.and(&crate::jj::RevSet::description("glob:\"*Pull Request:*\"")),
    )?;
    if revisions.is_empty() {
        crate::output::output(
            &config.icons.wave,
            "No commits found - nothing to do. Good bye!",
        )?;
        return Ok(());
    }

    let pull_requests = gh
        .pull_requests(
            revisions
                .iter()
                .map(|revision| revision.pull_request_number),
        )
        .await?;

    let mut template = String::from("\"\"");
    for (rev, pr) in std::iter::zip(revisions, pull_requests).into_iter() {
        let pr = match pr {
            Some(pr) => pr,
            None => continue,
        };

        let mut message = config.pull_request_url(pr.pr_number()) + " ";
        if pr.closed() {
            message += config.icons.land.as_ref();
        } else {
            // If it's already landed, all other info we might have is useless.
            if !pr.reviewers().is_empty() {
                message += config.icons.eyes.as_ref();
            }
            match pr.review_decision() {
                Some(crate::github::ReviewDecision::ChangesRequested) => {
                    message += config.icons.sparkle.as_ref();
                }
                Some(crate::github::ReviewDecision::Approved) if !pr.auto_merge_enabled() => {
                    message += config.icons.sparkle.as_ref();
                }
                Some(crate::github::ReviewDecision::ReviewRequired)
                    if pr.reviewers().is_empty() =>
                {
                    message += config.icons.sparkle.as_ref();
                }
                _ => {}
            }
        }

        template = format!(
            "if(stringify(change_id) == \"{}\", \"{}\", {template})",
            rev.id, message,
        )
    }

    // The crate::jj lib is intended for scripting against jj.
    // Here jj is used to generate the user facing output.
    Command::new("jj")
        .current_dir(
            jj.git_repo
                .workdir()
                .ok_or_else(|| crate::error::Error::new("No workdir on git repo?"))?,
        )
        .args([
            "log",
            "--no-pager",
            "-r",
            revset.or(&revset.ancestors_limited(2)).as_ref(),
            "-T",
            format!("builtin_log_compact ++ {template}",).as_ref(),
        ])
        .status()?;

    Ok(())
}
