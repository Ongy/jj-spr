use std::iter::zip;

use crate::{
    error::Result,
    github::PullRequestState,
    jj::{ChangeId, RevSet},
};

#[derive(Debug, clap::Parser)]
pub struct SyncOpts {
    #[clap(long, short = 'r')]
    revset: Option<String>,
}

async fn collect_futures<J, I: IntoIterator<Item = tokio::task::JoinHandle<J>>>(
    it: I,
) -> Result<Vec<J>> {
    let iterator = it.into_iter();
    let mut results = Vec::with_capacity(iterator.size_hint().0);
    for handle in iterator {
        results.push(handle.await?);
    }
    Ok(results)
}

pub async fn sync(
    jj: &crate::jj::Jujutsu,
    gh: &mut crate::github::GitHub,
    config: &crate::config::Config,
    opts: SyncOpts,
) -> Result<()> {
    jj.run_git_fetch()?;
    let revset = opts
        .revset
        .as_ref()
        .map(|s| RevSet::from_arg(s))
        .unwrap_or(RevSet::current());

    // We are interested in all revisions that have PRs
    let revisions = jj.read_revision_range(
        config,
        &revset
            .ancestors()
            .and(&RevSet::description("glob:\"*Pull Request:*\"").without(&RevSet::immutable())),
    )?;

    let pull_requests: Result<Vec<_>> =
        collect_futures(revisions.iter().map(|r: &crate::jj::Revision| {
            let gh = gh.clone();
            let pr_num = r.pull_request_number;
            tokio::spawn(async move {
                match pr_num {
                    Some(number) => gh.get_pull_request(number).await.map(|v| Some(v)),
                    None => Ok(None),
                }
            })
        }))
        .await?
        .into_iter()
        .collect();

    for (rev, pr) in zip(revisions, pull_requests?).into_iter() {
        let pr = if let Some(pr) = pr {
            pr
        } else {
            continue;
        };

        // TODO: Should this only abandon changes of PRs that have been merged?
        if pr.state == PullRequestState::Closed {
            jj.abandon(&RevSet::from(&rev.id).unique())?;
        }
    }
    jj.rebase_branch(
        &revset,
        ChangeId::from(format!(
            "{}@{}",
            config.master_ref.branch_name(),
            config.remote_name
        )),
    )?;

    Ok(())
}
