/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use crate::{
    error::{Error, Result},
    jj::RevSet,
    message::{MessageSection, validate_commit_message},
};

#[derive(Debug, clap::Parser, Default)]
pub struct FetchOptions {
    /// Jujutsu revision(s) to operate on. Can be a single revision like '@' or a range like 'main..@' or 'a::c'.
    /// If a range is provided, behaves like --all mode. If not specified, uses '@-'.
    #[clap(short = 'r', long, group = "revs")]
    revset: Option<String>,

    #[clap(long, short = 'a', group = "revs")]
    all: bool,

    /// Whether to also merge in any code changes
    #[clap(long)]
    pull_code_changes: bool,
}

#[cfg(test)]
impl FetchOptions {
    fn with_revset<S>(mut self, revset: Option<S>) -> Self
    where
        S: Into<String>,
    {
        self.revset = revset.map(|s| s.into());
        self
    }

    pub fn with_pull_code(mut self) -> Self {
        self.pull_code_changes = true;
        self
    }
}

fn do_fetch<
    I: IntoIterator<
        Item = (
            crate::jj::Revision,
            Option<impl crate::github::GHPullRequest>,
        ),
    >,
>(
    opts: FetchOptions,
    jj: &mut crate::jj::Jujutsu,
    config: &crate::config::Config,
    commits: I,
) -> Result<()> {
    let mut failure = false;
    let mut items: Vec<_> = commits.into_iter().collect();

    for (revision, pull_request) in items.iter_mut() {
        if let Some(pull_request) = pull_request {
            // Ok, we want to update our local change with any code changes that were done upstream
            if opts.pull_code_changes
                && let Some(old_rev) = revision.message.get(&MessageSection::LastCommit)
            {
                let base_revset = {
                    let base_commit = jj.git_repo.find_commit(git2::Oid::from_str(old_rev)?)?;
                    RevSet::from(&base_commit)
                };
                let head_revset = {
                    let head_branch = jj.git_repo.find_branch(
                        format!("{}/{}", config.remote_name, pull_request.head_branch_name())
                            .as_str(),
                        git2::BranchType::Remote,
                    )?;
                    RevSet::from_remote_branch(&head_branch, &config.remote_name)?
                };
                // When we are based on the main branch, we'll potentially rebase.
                // This only makes sense for changes on main.
                if pull_request.base_branch_name() == config.master_ref {
                    let main_revset = {
                        let main_branch = jj.git_repo.find_branch(
                            format!("{}/{}", config.remote_name, config.master_ref).as_str(),
                            git2::BranchType::Remote,
                        )?;
                        RevSet::from_remote_branch(&main_branch, &config.remote_name)?
                    };

                    let main_head_fork =
                        jj.revset_to_change_id(&head_revset.fork_point(&main_revset))?;
                    let main_change_fork = jj.revset_to_change_id(
                        &RevSet::from(&revision.id).fork_point(&main_revset),
                    )?;

                    let forks_fork = jj.revset_to_change_id(
                        &RevSet::from(&main_head_fork).fork_point(&RevSet::from(&main_change_fork)),
                    )?;

                    // I.e. HEAD's base is *ahead* of our base.
                    // I.e. a user pressed the "merge base into PR" button
                    // So we should update to also be based on that.
                    if forks_fork == main_change_fork && main_change_fork != main_head_fork {
                        jj.rebase_branch(&RevSet::from(&revision.id), main_head_fork)?;
                    }
                }

                jj.squash_copy(&base_revset.to(&head_revset), revision.id.clone())?;
                let new_latest_commit = jj.resolve_revision_to_commit_id(head_revset.as_ref())?;
                revision
                    .message
                    .insert(MessageSection::LastCommit, new_latest_commit.to_string());
            }

            revision
                .message
                .insert(MessageSection::Title, pull_request.title().into());
            revision
                .message
                .insert(MessageSection::Summary, pull_request.body().into());
        }

        failure = validate_commit_message(&revision.message).is_err() || failure;
    }
    for (rev, _) in items.into_iter() {
        jj.update_revision_message(&rev)?;
    }

    if failure { Err(Error::empty()) } else { Ok(()) }
}

pub async fn fetch<GH, PR>(
    opts: FetchOptions,
    jj: &mut crate::jj::Jujutsu,
    mut gh: GH,
    config: &crate::config::Config,
) -> Result<()>
where
    PR: crate::github::GHPullRequest,
    GH: crate::github::GitHubAdapter<PRAdapter = PR>,
{
    let revset = opts
        .revset
        .as_ref()
        .map(|s| RevSet::from_arg(s))
        .unwrap_or(if opts.all {
            RevSet::mutable().heads()
        } else {
            RevSet::current()
        });
    let revisions = jj.read_revision_range(
        config,
        &&revset
            .ancestors()
            .without(&RevSet::immutable().or(&RevSet::description("exact:\"\""))),
    )?;

    if revisions.is_empty() {
        crate::output::output(
            crate::output::Icons::Wave,
            "No commits found - nothing to do. Good bye!",
        )?;
        return Ok(());
    }

    let pull_requests = gh
        .pull_requests(revisions.iter().map(|r| r.pull_request_number))
        .await?;

    do_fetch(
        opts,
        jj,
        config,
        std::iter::zip(revisions, pull_requests.into_iter()),
    )
}

#[cfg(test)]
mod tests {
    use super::FetchOptions;
    use crate::{
        jj::{ChangeId, RevSet},
        message::MessageSection,
        testing,
    };
    use std::fs;

    fn create_jujutsu_commit(
        jj: &mut crate::jj::Jujutsu,
        message: &str,
        file_content: &str,
    ) -> ChangeId {
        // Create a file
        let file_path = jj
            .git_repo
            .workdir()
            .expect("Failed to extract workdir from JJ handle")
            .join("my_file");
        fs::write(&file_path, file_content).expect("Failed to write test file");

        jj.commit(message).expect("Failed to commit revision");
        jj.revset_to_change_id(&RevSet::current().parent())
            .expect("Failed to get changeid of '@-'")
    }

    #[tokio::test]
    async fn test_single_on_head() {
        let (_temp_dir, mut jj, _) = testing::setup::repo_with_origin();
        let config = testing::config::basic();
        let pr_url = config.pull_request_url(1);

        let _ = create_jujutsu_commit(
            &mut jj,
            format!(
                "Test commit\n\n\nPull Request: {}\nLast Commit: My Last Commit",
                pr_url,
            )
            .as_ref(),
            "file 1",
        );

        super::fetch(
            FetchOptions::default(),
            &mut jj,
            crate::github::fakes::GitHub {
                pull_requests: std::collections::BTreeMap::from([(
                    1,
                    crate::github::fakes::PullRequest::new(
                        "main",
                        "spr/test/test-commit",
                        1,
                        "New Title",
                        "New Summary",
                    ),
                )]),
            },
            &config,
        )
        .await
        .expect("amend should not error");

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
        let (_temp_dir, mut jj, _) = testing::setup::repo_with_origin();
        let config = testing::config::basic();
        let pr_url = config.pull_request_url(1);

        let trunk_oid = jj
            .git_repo
            .refname_to_id("HEAD")
            .expect("Failed to revparse HEAD");

        let rev = create_jujutsu_commit(
            &mut jj,
            format!("Test commit\n\n\nPull Request: {pr_url}\nLast Commit: {trunk_oid}",).as_str(),
            "file 1",
        );
        let pre_amend_tree = jj
            .get_tree_oid_for_commit(
                jj.resolve_revision_to_commit_id(rev.as_ref())
                    .expect("Failed to get commit for revision"),
            )
            .expect("Failed to get tree for commit");

        jj.git_repo
            .set_head_detached(trunk_oid)
            .expect("Expected to be able to checkout trunk");
        let new_oid =
            testing::git::add_commit_and_push_to_remote(&jj.git_repo, "spr/test/test-commit");

        super::fetch(
            FetchOptions::default()
                .with_revset(Some(rev.as_ref()))
                .with_pull_code(),
            &mut jj,
            crate::github::fakes::GitHub {
                pull_requests: std::collections::BTreeMap::from([(
                    1,
                    crate::github::fakes::PullRequest::new(
                        "main",
                        "spr/test/test-commit",
                        1,
                        "New Title",
                        "New Summary",
                    ),
                )]),
            },
            &config,
        )
        .await
        .expect("amend should not error");

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
            change
                .message
                .get(&MessageSection::LastCommit)
                .expect("The re-read change should have a last commit"),
            &new_oid.to_string(),
            "fetch didn't update Last Commit tag correctly"
        );

        let post_amend_tree = jj
            .get_tree_oid_for_commit(
                jj.resolve_revision_to_commit_id(rev.as_ref())
                    .expect("Failed to get commit for revision"),
            )
            .expect("Failed to get tree for commit");
        assert_ne!(pre_amend_tree, post_amend_tree, "Tree didn't change");
    }

    #[tokio::test]
    async fn rebase_to_new_head() {
        let (_temp_dir, mut jj, _) = testing::setup::repo_with_origin();
        let config = testing::config::basic();
        let pr_url = config.pull_request_url(1);

        let trunk_oid = jj
            .git_repo
            .refname_to_id("HEAD")
            .expect("Failed to revparse HEAD");

        let rev = create_jujutsu_commit(
            &mut jj,
            format!("Test commit\n\n\nPull Request: {pr_url}\nLast Commit: {trunk_oid}",).as_str(),
            "file 1",
        );

        let head =
            testing::git::add_commit_on_and_push_to_remote(&jj.git_repo, "main", [trunk_oid]);
        let head_revset = {
            let head_commit = jj
                .git_repo
                .find_commit(head)
                .expect("Couldn't find commit for head");

            RevSet::from(&head_commit)
        };
        let _ = testing::git::add_commit_on_and_push_to_remote(
            &jj.git_repo,
            "spr/test/test-commit",
            [head, trunk_oid],
        );

        jj.update().expect("Expected to be able to update JJ state");
        super::fetch(
            FetchOptions::default()
                .with_revset(Some(rev.as_ref()))
                .with_pull_code(),
            &mut jj,
            crate::github::fakes::GitHub {
                pull_requests: std::collections::BTreeMap::from([(
                    1,
                    crate::github::fakes::PullRequest::new(
                        "main",
                        "spr/test/test-commit",
                        1,
                        "New Title",
                        "New Summary",
                    ),
                )]),
            },
            &config,
        )
        .await
        .expect("amend should not error");

        let fork_point = jj
            .resolve_revision_to_commit_id(head_revset.fork_point(&RevSet::from(&rev)).as_ref())
            .expect("Couldn't find fork point of new revision and main commit");
        assert_eq!(
            fork_point, head,
            "Revision wasn't based on new head after amend"
        )
    }

    #[tokio::test]
    async fn no_rebase_to_old_head() {
        let (_temp_dir, mut jj, _) = testing::setup::repo_with_origin();
        let config = testing::config::basic();
        let pr_url = config.pull_request_url(1);

        let trunk_oid = jj
            .git_repo
            .refname_to_id("HEAD")
            .expect("Failed to revparse HEAD");

        let rev = create_jujutsu_commit(
            &mut jj,
            format!("Test commit\n\n\nPull Request: {pr_url}\nLast Commit: {trunk_oid}",).as_str(),
            "file 1",
        );

        let head =
            testing::git::add_commit_on_and_push_to_remote(&jj.git_repo, "main", [trunk_oid]);
        let _ = testing::git::add_commit_on_and_push_to_remote(
            &jj.git_repo,
            "spr/test/test-commit",
            [head, trunk_oid],
        );
        let head = testing::git::add_commit_on_and_push_to_remote(&jj.git_repo, "main", [head]);
        let head_revset = {
            let head_commit = jj
                .git_repo
                .find_commit(head)
                .expect("Couldn't find commit for head");

            RevSet::from(&head_commit)
        };
        jj.update().expect("Expected to be able to update JJ state");
        let head_change = jj
            .revset_to_change_id(&head_revset)
            .expect("Expected to find change_id for head");
        jj.rebase_branch(&RevSet::from(&rev), head_change)
            .expect("Should be able to rebase rev");

        super::fetch(
            FetchOptions::default()
                .with_revset(Some(rev.as_ref()))
                .with_pull_code(),
            &mut jj,
            crate::github::fakes::GitHub {
                pull_requests: std::collections::BTreeMap::from([(
                    1,
                    crate::github::fakes::PullRequest::new(
                        "main",
                        "spr/test/test-commit",
                        1,
                        "New Title",
                        "New Summary",
                    ),
                )]),
            },
            &config,
        )
        .await
        .expect("amend should not error");

        let fork_point = jj
            .resolve_revision_to_commit_id(head_revset.fork_point(&RevSet::from(&rev)).as_ref())
            .expect("Couldn't find fork point of new revision and main commit");
        assert_eq!(fork_point, head, "Revision was rebased to older HEAD")
    }
}
