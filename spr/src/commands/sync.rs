use std::iter::zip;

use crate::{
    error::Result,
    jj::{ChangeId, RevSet},
    output::output,
};

#[derive(Debug, clap::Parser)]
pub struct SyncOpts {
    #[clap(long, short = 'r', group = "revs")]
    revset: Option<String>,

    #[clap(long, short = 'a', group = "revs")]
    all: bool,
}

pub async fn sync<GH, PR>(
    jj: &mut crate::jj::Jujutsu,
    mut gh: GH,
    config: &crate::config::Config,
    opts: SyncOpts,
) -> Result<()>
where
    PR: crate::github::GHPullRequest,
    GH: crate::github::GitHubAdapter<PRAdapter = PR>,
{
    jj.run_git_fetch()?;
    let revset = opts
        .revset
        .as_ref()
        .map(|s| RevSet::from_arg(s))
        .unwrap_or(if opts.all {
            RevSet::mutable().heads()
        } else {
            RevSet::current()
        });

    // We are interested in all revisions that have PRs
    let revisions = jj.read_revision_range(
        config,
        &revset
            .ancestors()
            .and(&RevSet::description("glob:\"*Pull Request:*\"").without(&RevSet::immutable())),
    )?;

    let pull_requests = gh
        .pull_requests(revisions.iter().map(|n| n.pull_request_number))
        .await?;

    for (rev, pr) in zip(revisions, pull_requests).into_iter() {
        let pr = if let Some(pr) = pr {
            pr
        } else {
            continue;
        };

        // TODO: Should this only abandon changes of PRs that have been merged?
        if pr.closed() {
            output(
                "🛬",
                format!(
                    "{} landed. Abandoning {:?}",
                    config.pull_request_url(pr.pr_number()),
                    rev.id,
                ),
            )?;
            jj.abandon(&RevSet::from(&rev.id).unique())?;
        }
    }
    if jj.revset_to_change_ids(&revset)?.is_empty() {
        output("👋", "Nothing left to rebase")?;
        return Ok(());
    }
    output("🔁", format!("Going to rebase {:?}", revset))?;
    jj.rebase_branch(
        &revset,
        ChangeId::from(format!("{}@{}", config.master_ref, config.remote_name)),
    )?;

    Ok(())
}
