/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use crate::{
    error::{Error, Result},
    github::PullRequest,
    jj::RevSet,
    message::{MessageSection, validate_commit_message},
    output::{output, write_commit_title},
};

#[derive(Debug, clap::Parser)]
pub struct AmendOptions {
    /// Amend commits in range from base to revision
    #[clap(long, short = 'a')]
    all: bool,

    /// Base revision for --all mode (if not specified, uses trunk)
    #[clap(long)]
    base: Option<String>,

    /// Jujutsu revision(s) to operate on. Can be a single revision like '@' or a range like 'main..@' or 'a::c'.
    /// If a range is provided, behaves like --all mode. If not specified, uses '@-'.
    #[clap(short = 'r', long)]
    revision: Option<String>,

    /// Whether to also merge in any code changes
    #[clap(long)]
    pull_code_changes: bool,
}

fn do_amend<I: IntoIterator<Item = (crate::jj::PreparedCommit, Option<PullRequest>)>>(
    opts: AmendOptions,
    jj: &crate::jj::Jujutsu,
    config: &crate::config::Config,
    commits: I,
) -> Result<()> {
    let mut failure = false;
    let mut items: Vec<_> = commits.into_iter().collect();

    for (commit, pull_request) in items.iter_mut() {
        write_commit_title(commit)?;
        if let Some(pull_request) = pull_request {
            // Ok, we want to update our local change with any code changes that were done upstream
            if opts.pull_code_changes
                && let Some(old_rev) = commit.message.get(&MessageSection::LastCommit)
            {
                let base_commit = jj.git_repo.find_commit(git2::Oid::from_str(old_rev)?)?;
                let head_branch = jj.git_repo.find_branch(
                    format!("{}/{}", config.remote_name, pull_request.head.branch_name()).as_str(),
                    git2::BranchType::Remote,
                )?;
                jj.squash_copy(
                    &RevSet::from(&base_commit).to(&RevSet::from_remote_branch(
                        head_branch,
                        &config.remote_name,
                    )?),
                    crate::jj::ChangeId::from(commit.short_id.clone()),
                )?;
            }

            for (k, v) in pull_request.sections.iter() {
                commit.message.insert(*k, v.clone());
            }
            commit.message_changed = true;
        }

        failure = validate_commit_message(&commit.message).is_err() || failure;
    }
    let mut pc: Vec<_> = items.into_iter().map(|t| t.0).collect();
    jj.rewrite_commit_messages(&mut pc)?;

    if failure { Err(Error::empty()) } else { Ok(()) }
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

pub async fn amend(
    opts: AmendOptions,
    jj: &crate::jj::Jujutsu,
    gh: &mut crate::github::GitHub,
    config: &crate::config::Config,
) -> Result<()> {
    // Determine revision and whether to use range mode
    let (use_range_mode, base_rev, target_rev, is_inclusive) =
        crate::revision_utils::parse_revision_and_range(
            opts.revision.as_deref(),
            opts.all,
            opts.base.as_deref(),
        )?;

    let pc = if use_range_mode {
        jj.get_prepared_commits_from_to(config, &base_rev, &target_rev, is_inclusive)?
    } else {
        vec![jj.get_prepared_commit_for_revision(config, &target_rev)?]
    };

    if pc.is_empty() {
        output("👋", "No commits found - nothing to do. Good bye!")?;
        return Ok(());
    }

    #[allow(clippy::needless_collect)]
    let pull_requests: Result<Vec<_>> =
        collect_futures(pc.iter().map(|c: &crate::jj::PreparedCommit| {
            let gh = gh.clone();
            let pr_num = c.pull_request_number;
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

    do_amend(opts, jj, config, std::iter::zip(pc, pull_requests?))
}

#[cfg(test)]
mod tests {
    use super::do_amend;
    use crate::{
        commands::amend::AmendOptions,
        jj::{ChangeId, RevSet},
        message::MessageSection,
        testing,
    };
    use std::fs;

    fn create_jujutsu_commit(
        jj: &crate::jj::Jujutsu,
        message: &str,
        file_content: &str,
    ) -> ChangeId {
        // Create a file
        let file_path = jj
            .git_repo
            .workdir()
            .expect("Failed to extract workdir from JJ handle")
            .join("test.txt");
        fs::write(&file_path, file_content).expect("Failed to write test file");

        jj.commit(message).expect("Failed to commit revision");
        jj.revset_to_change_id(&RevSet::current().parent())
            .expect("Failed to get changeid of '@-'")
    }

    #[tokio::test]
    async fn test_single_on_head() {
        let (_temp_dir, jj, _) = testing::setup::repo_with_origin();

        let _ = create_jujutsu_commit(&jj, "Test commit\n\nLast Commit: My Last Commit", "file 1");
        let change = jj
            .get_prepared_commit_for_revision(&testing::config::basic(), "@-")
            .expect("Failed to prepare commit");

        let _ = do_amend(
            AmendOptions {
                all: false,
                base: None,
                revision: None,
                pull_code_changes: false,
            },
            &jj,
            &testing::config::basic(),
            [(
                change.clone(),
                Some(crate::github::PullRequest {
                    number: 1,
                    state: crate::github::PullRequestState::Open,
                    title: String::from("New Title"),
                    body: None,
                    base_oid: git2::Oid::zero(),
                    sections: std::collections::BTreeMap::from([
                        (MessageSection::Summary, "New Summary".into()),
                        (MessageSection::Title, "New Title".into()),
                    ]),
                    base: crate::github::GitHubBranch::new_from_branch_name(
                        "main", "origin", "main",
                    ),
                    head_oid: git2::Oid::zero(),
                    head: crate::github::GitHubBranch::new_from_branch_name(
                        "spr/test/test-commit",
                        "origin",
                        "main",
                    ),
                    merge_commit: None,
                    reviewers: std::collections::HashMap::new(),
                    review_status: None,
                }),
            )],
        )
        .expect("do_amend was not expected to error");

        // Reread the changed commit so we can check whether it was updated
        let change = jj
            .get_prepared_commit_for_revision(&testing::config::basic(), "@-")
            .expect("Failed to prepare commit");
        assert_eq!(
            change.message.get(&MessageSection::Title),
            Some(&"New Title".into()),
            "Title was not updated"
        );
        assert_eq!(
            change.message.get(&MessageSection::Summary),
            Some(&"New Summary".into()),
            "Summary was not updated"
        );
        assert_eq!(
            change.message.get(&MessageSection::LastCommit),
            Some(&"My Last Commit".into()),
            "Last Commit was changed"
        );
    }

    #[tokio::test]
    async fn test_pull_changes_on_head() {
        let (_temp_dir, jj, _) = testing::setup::repo_with_origin();
        let trunk_oid = jj
            .git_repo
            .refname_to_id("HEAD")
            .expect("Failed to revparse HEAD");

        let rev = create_jujutsu_commit(
            &jj,
            format!("Test commit\n\nLast Commit: {trunk_oid}").as_str(),
            "file 1",
        );
        let change = jj
            .get_prepared_commit_for_revision(
                &testing::config::basic(),
                format!("change_id({})", rev.as_ref()).as_str(),
            )
            .expect("Failed to prepare commit");
        let pre_amend_tree = jj
            .get_tree_oid_for_commit(
                jj.resolve_revision_to_commit_id(rev.as_ref())
                    .expect("Failed to get commit for revision"),
            )
            .expect("Failed to get tree for commit");

        jj.git_repo
            .set_head_detached(trunk_oid)
            .expect("Expected to be able to checkout trunk");
        testing::git::add_commit_and_push_to_remote(&jj.git_repo, "spr/test/test-commit");

        let _ = do_amend(
            AmendOptions {
                all: false,
                base: None,
                revision: None,
                pull_code_changes: true,
            },
            &jj,
            &testing::config::basic(),
            [(
                change.clone(),
                Some(crate::github::PullRequest {
                    number: 1,
                    state: crate::github::PullRequestState::Open,
                    title: String::from("New Title"),
                    body: None,
                    base_oid: git2::Oid::zero(),
                    sections: std::collections::BTreeMap::from([
                        (MessageSection::Summary, "New Summary".into()),
                        (MessageSection::Title, "New Title".into()),
                    ]),
                    base: crate::github::GitHubBranch::new_from_branch_name(
                        "main", "origin", "main",
                    ),
                    head_oid: git2::Oid::zero(),
                    head: crate::github::GitHubBranch::new_from_branch_name(
                        "spr/test/test-commit",
                        "origin",
                        "main",
                    ),
                    merge_commit: None,
                    reviewers: std::collections::HashMap::new(),
                    review_status: None,
                }),
            )],
        )
        .expect("do_amend was not expected to error");

        // Reread the changed commit so we can check whether it was updated
        let change = jj
            .get_prepared_commit_for_revision(&testing::config::basic(), rev.as_ref())
            .expect("Failed to prepare commit");
        assert_eq!(
            change.message.get(&MessageSection::Title),
            Some(&"New Title".into()),
            "Title was not updated"
        );
        assert_eq!(
            change.message.get(&MessageSection::Summary),
            Some(&"New Summary".into()),
            "Summary was not updated"
        );
        assert_eq!(
            change.message.get(&MessageSection::LastCommit),
            Some(&trunk_oid.to_string()),
            "Last Commit was changed"
        );

        let post_amend_tree = jj
            .get_tree_oid_for_commit(
                jj.resolve_revision_to_commit_id(rev.as_ref())
                    .expect("Failed to get commit for revision"),
            )
            .expect("Failed to get tree for commit");
        assert_ne!(pre_amend_tree, post_amend_tree, "Tree didn't change");
    }
}
