/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use crate::{
    error::{Error, Result},
    jj::RevSet,
    message::{MessageSection, MessageSectionsMap, build_commit_message},
};

#[derive(Debug, clap::Parser)]
pub struct PatchOptions {
    /// Pull Request number
    pull_request: u64,

    /// Name of the branch to be created. Defaults to `PR-<number>`
    #[clap(long)]
    branch_name: Option<String>,

    /// If given, create new branch but do not check out
    #[clap(long)]
    no_checkout: bool,
}

fn do_patch(
    jj: &mut crate::jj::Jujutsu,
    config: &crate::config::Config,
    message: &MessageSectionsMap,
    branch_name: &str,
) -> Result<()> {
    jj.run_git_fetch()?;

    let resolved = jj.resolve_reference(
        format!("refs/remotes/{}/{}", config.remote_name, branch_name).as_str(),
    )?;

    let mut message = message.clone();
    message.insert(MessageSection::LastCommit, resolved.to_string());
    let message = build_commit_message(&message);
    let base_revset = {
        let base_branch = jj.git_repo.find_branch(
            format!("{}/{}", config.remote_name, config.master_ref.branch_name()).as_str(),
            git2::BranchType::Remote,
        )?;
        RevSet::from_remote_branch(&base_branch, config.remote_name.clone())?.unique()
    };

    let head_revset = {
        let head_branch = jj.git_repo.find_branch(
            format!("{}/{}", config.remote_name, branch_name).as_ref(),
            git2::BranchType::Remote,
        )?;
        RevSet::from_remote_branch(&head_branch, config.remote_name.clone())?.unique()
    };

    jj.new_revision(
        Some(base_revset.fork_point(&head_revset).unique()),
        Some(message),
        false,
    )?;

    jj.restore(
        None as Option<&str>,
        Some(format!(
            "exactly(remote_bookmarks({}, {}), 1)",
            branch_name, config.remote_name
        )),
        Some("@"),
    )?;

    Ok(())
}

pub async fn patch<GH, PR>(
    opts: PatchOptions,
    jj: &mut crate::jj::Jujutsu,
    mut gh: GH,
    config: &crate::config::Config,
) -> Result<()>
where
    PR: crate::github::GHPullRequest,
    GH: crate::github::GitHubAdapter<PRAdapter = PR>,
{
    let pr = gh.pull_request(opts.pull_request).await?;

    if pr.base_branch_name() != config.master_ref.branch_name() {
        return Err(Error::new(format!(
            "Specified PR {} is not based on the target branch. Adopting stacked PRs is not yet supported",
            pr.pr_number()
        )));
    }

    do_patch(jj, config, pr.sections(), pr.head_branch_name())
}

#[cfg(test)]
mod tests {
    use crate::{commands::patch::PatchOptions, jj::RevSet, message::MessageSection, testing};

    #[tokio::test]
    async fn test_single_on_head() {
        let pr_nr = 1;
        let (_temp_dir, mut jj, _) = testing::setup::repo_with_origin();
        let commit_oid =
            testing::git::add_commit_and_push_to_remote(&jj.git_repo, "spr/test/test-branch");
        let tree_oid = jj
            .get_tree_oid_for_commit(commit_oid)
            .expect("Expected to get tree for commit");
        let new_main = testing::git::add_commit_and_push_to_remote(&jj.git_repo, "main");

        super::patch(
            PatchOptions {
                pull_request: 1,
                branch_name: None,
                no_checkout: true,
            },
            &mut jj,
            crate::github::fakes::GitHub {
                pull_requests: std::collections::BTreeMap::from([(
                    1,
                    crate::github::fakes::PullRequest {
                        number: 1,
                        base: String::from("main"),
                        head: String::from("spr/test/test-branch"),
                        sections: std::collections::BTreeMap::from([(
                            MessageSection::PullRequest,
                            testing::config::basic().pull_request_url(pr_nr),
                        )]),
                    },
                )]),
            },
            &testing::config::basic(),
        )
        .await
        .expect("patch() should not fail");

        let change = jj
            .revset_to_change_id(&RevSet::current())
            .expect("Failed to resolve change of current");
        let rev = jj
            .read_revision(&testing::config::basic(), change)
            .expect("Failed to read revision after patch");

        let new_tree = jj
            .get_tree_oid_for_commit(
                jj.resolve_revision_to_commit_id(rev.id.as_ref())
                    .expect("Failed to get commit OID for rev"),
            )
            .expect("Failed to get tree for current revision");

        assert_eq!(
            new_tree, tree_oid,
            "Commit created by stack doesn't have same tree as remote branch HEAD"
        );
        assert_eq!(
            rev.pull_request_number,
            Some(pr_nr),
            "Parsed PR# from revision didn't match"
        );
        assert_eq!(
            rev.message.get(&MessageSection::LastCommit),
            Some(&commit_oid.to_string()),
            "Parsed last commit didn't match expected upstream commit"
        );
        assert_ne!(
            jj.git_repo
                .merge_base(
                    new_main,
                    jj.resolve_revision_to_commit_id(rev.id.as_ref())
                        .expect("Failed to find commit for revision")
                )
                .expect("Couldn't get merge base"),
            new_main,
            "new change was based on HEAD instead of base of commit",
        )
    }
}
