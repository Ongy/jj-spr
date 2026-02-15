/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use crate::{
    error::Result,
    jj::RevSet,
    message::{MessageSection, MessageSectionsMap, build_commit_message},
};

#[derive(Debug, clap::Parser)]
pub struct AdoptOptions {
    /// Pull Request number
    pull_request: u64,

    /// Name of the branch to be created. Defaults to `PR-<number>`
    #[clap(long)]
    branch_name: Option<String>,

    /// If given, create new branch but do not check out
    #[clap(long)]
    no_checkout: bool,
}

fn find_commit_for_pr(
    jj: &crate::jj::Jujutsu,
    config: &crate::config::Config,
    nr: u64,
) -> Result<crate::jj::Revision> {
    let url = config.pull_request_url(nr);

    let id =
        jj.revset_to_change_id(&RevSet::description(format!("substring:\"{}\"", url)).unique())?;
    jj.read_revision(config, id)
}

fn do_adopt(
    jj: &mut crate::jj::Jujutsu,
    config: &crate::config::Config,
    message: &MessageSectionsMap,
    branch_name: &str,
    parent: Option<u64>,
) -> Result<()> {
    jj.run_git_fetch()?;

    let resolved = jj.resolve_reference(
        format!("refs/remotes/{}/{}", config.remote_name, branch_name).as_str(),
    )?;

    let mut message = message.clone();
    message.insert(MessageSection::LastCommit, resolved.to_string());
    let message = build_commit_message(&message);

    let head_revset = {
        let head_branch = jj.git_repo.find_branch(
            format!("{}/{}", config.remote_name, branch_name).as_ref(),
            git2::BranchType::Remote,
        )?;
        RevSet::from_remote_branch(&head_branch, config.remote_name.clone())?.unique()
    };

    let base_revset = if let Some(parent) = parent {
        let url = config.pull_request_url(parent);
        RevSet::description(format!("substring:\"{}\"", url)).unique()
    } else {
        let base_branch = jj.git_repo.find_branch(
            format!("{}/{}", config.remote_name, config.master_ref.branch_name()).as_str(),
            git2::BranchType::Remote,
        )?;
        RevSet::from_remote_branch(&base_branch, config.remote_name.clone())?
            .unique()
            .fork_point(&head_revset)
            .unique()
    };

    jj.new_revision(Some(base_revset), Some(message), false)?;

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

pub async fn adopt<GH, PR>(
    opts: AdoptOptions,
    jj: &mut crate::jj::Jujutsu,
    mut gh: GH,
    config: &crate::config::Config,
) -> Result<()>
where
    PR: crate::github::GHPullRequest,
    GH: crate::github::GitHubAdapter<PRAdapter = PR>,
{
    let mut pr_chain = Vec::new();
    let pr = gh.pull_request(opts.pull_request).await?;

    pr_chain.push((pr, None));
    while let Some(last) = pr_chain.last_mut()
        && last.0.base_branch_name() != config.master_ref.branch_name()
    {
        let next = gh.pull_request_by_head(last.0.base_branch_name()).await?;
        last.1 = Some(next.pr_number());

        // Early exit when we already have a change for the parent PR
        if find_commit_for_pr(jj, config, next.pr_number()).is_ok() {
            break;
        }

        // Otherwise, put it onto the chain to be pulled.
        // The loop will continue to make sure we pull everything we need.
        pr_chain.push((next, None));
    }
    pr_chain.reverse();

    for (pr, parent) in pr_chain {
        do_adopt(jj, config, pr.sections(), pr.head_branch_name(), parent)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::AdoptOptions;
    use crate::{jj::RevSet, message::MessageSection, testing};

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

        super::adopt(
            AdoptOptions {
                pull_request: pr_nr,
                branch_name: None,
                no_checkout: true,
            },
            &mut jj,
            crate::github::fakes::GitHub {
                pull_requests: std::collections::BTreeMap::from([(
                    pr_nr,
                    crate::github::fakes::PullRequest {
                        number: pr_nr,
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

    #[tokio::test]
    async fn stacked() {
        let (pr_nr, other_nr) = (1, 3);
        let (_temp_dir, mut jj, _) = testing::setup::repo_with_origin();
        let commit_oid =
            testing::git::add_commit_and_push_to_remote(&jj.git_repo, "spr/test/test-branch");
        let _ = testing::git::add_commit_on_and_push_to_remote(
            &jj.git_repo,
            "spr/test/other-branch",
            [commit_oid],
        );

        super::adopt(
            AdoptOptions {
                pull_request: other_nr,
                branch_name: None,
                no_checkout: true,
            },
            &mut jj,
            crate::github::fakes::GitHub {
                pull_requests: std::collections::BTreeMap::from([
                    (
                        pr_nr,
                        crate::github::fakes::PullRequest {
                            number: 1,
                            base: String::from("main"),
                            head: String::from("spr/test/test-branch"),
                            sections: std::collections::BTreeMap::from([(
                                MessageSection::PullRequest,
                                testing::config::basic().pull_request_url(pr_nr),
                            )]),
                        },
                    ),
                    (
                        other_nr,
                        crate::github::fakes::PullRequest {
                            number: other_nr,
                            base: String::from("spr/test/test-branch"),
                            head: String::from("spr/test/other-branch"),
                            sections: std::collections::BTreeMap::from([(
                                MessageSection::PullRequest,
                                testing::config::basic().pull_request_url(other_nr),
                            )]),
                        },
                    ),
                ]),
            },
            &testing::config::basic(),
        )
        .await
        .expect("patch() should not fail");

        let base_rev = super::find_commit_for_pr(&jj, &testing::config::basic(), pr_nr)
            .expect("Failed to find revision for base PR");
        assert_eq!(
            base_rev.pull_request_number,
            Some(pr_nr),
            "PR # didn't match for base pr",
        );
        let stacked_rev = super::find_commit_for_pr(&jj, &testing::config::basic(), other_nr)
            .expect("Failed to find revision for stacked PR");
        assert_eq!(
            stacked_rev.pull_request_number,
            Some(other_nr),
            "PR # didn't match for other pr",
        );

        let fork = jj
            .revset_to_change_id(
                &RevSet::from(&base_rev.id).fork_point(&RevSet::from(&stacked_rev.id)),
            )
            .expect("Couldn't find fork point of PR revisions");
        assert_eq!(
            fork, base_rev.id,
            "stacked PR's revision was not forked from base PR"
        );
    }

    #[tokio::test]
    async fn partial() {
        let (pr_nr, other_nr) = (1, 3);
        let (_temp_dir, mut jj, _) = testing::setup::repo_with_origin();
        let commit_oid =
            testing::git::add_commit_and_push_to_remote(&jj.git_repo, "spr/test/test-branch");
        let _ = testing::git::add_commit_on_and_push_to_remote(
            &jj.git_repo,
            "spr/test/other-branch",
            [commit_oid],
        );

        super::adopt(
            AdoptOptions {
                pull_request: pr_nr,
                branch_name: None,
                no_checkout: true,
            },
            &mut jj,
            crate::github::fakes::GitHub {
                pull_requests: std::collections::BTreeMap::from([
                    (
                        pr_nr,
                        crate::github::fakes::PullRequest {
                            number: 1,
                            base: String::from("main"),
                            head: String::from("spr/test/test-branch"),
                            sections: std::collections::BTreeMap::from([(
                                MessageSection::PullRequest,
                                testing::config::basic().pull_request_url(pr_nr),
                            )]),
                        },
                    ),
                    (
                        other_nr,
                        crate::github::fakes::PullRequest {
                            number: other_nr,
                            base: String::from("spr/test/test-branch"),
                            head: String::from("spr/test/other-branch"),
                            sections: std::collections::BTreeMap::from([(
                                MessageSection::PullRequest,
                                testing::config::basic().pull_request_url(other_nr),
                            )]),
                        },
                    ),
                ]),
            },
            &testing::config::basic(),
        )
        .await
        .expect("patch() should not fail");

        super::adopt(
            AdoptOptions {
                pull_request: other_nr,
                branch_name: None,
                no_checkout: true,
            },
            &mut jj,
            crate::github::fakes::GitHub {
                pull_requests: std::collections::BTreeMap::from([
                    (
                        pr_nr,
                        crate::github::fakes::PullRequest {
                            number: 1,
                            base: String::from("main"),
                            head: String::from("spr/test/test-branch"),
                            sections: std::collections::BTreeMap::from([(
                                MessageSection::PullRequest,
                                testing::config::basic().pull_request_url(pr_nr),
                            )]),
                        },
                    ),
                    (
                        other_nr,
                        crate::github::fakes::PullRequest {
                            number: other_nr,
                            base: String::from("spr/test/test-branch"),
                            head: String::from("spr/test/other-branch"),
                            sections: std::collections::BTreeMap::from([(
                                MessageSection::PullRequest,
                                testing::config::basic().pull_request_url(other_nr),
                            )]),
                        },
                    ),
                ]),
            },
            &testing::config::basic(),
        )
        .await
        .expect("patch() should not fail");

        let base_rev = super::find_commit_for_pr(&jj, &testing::config::basic(), pr_nr)
            .expect("Failed to find revision for base PR");
        assert_eq!(
            base_rev.pull_request_number,
            Some(pr_nr),
            "PR # didn't match for base pr",
        );
        let stacked_rev = super::find_commit_for_pr(&jj, &testing::config::basic(), other_nr)
            .expect("Failed to find revision for stacked PR");
        assert_eq!(
            stacked_rev.pull_request_number,
            Some(other_nr),
            "PR # didn't match for other pr",
        );

        let fork = jj
            .revset_to_change_id(
                &RevSet::from(&base_rev.id).fork_point(&RevSet::from(&stacked_rev.id)),
            )
            .expect("Couldn't find fork point of PR revisions");
        assert_eq!(
            fork, base_rev.id,
            "stacked PR's revision was not forked from base PR"
        );
    }
}
