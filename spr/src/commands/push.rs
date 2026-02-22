use crate::{
    error::{Error, Result, ResultExt},
    jj::RevSet,
    message::{MessageSection, build_github_body},
    output::output,
    utils::run_command,
};
use git2::Oid;
use std::{io::ErrorKind, iter::zip};

#[derive(Debug, clap::Parser, Default)]
pub struct PushOptions {
    #[clap(long, short = 'm')]
    message: Option<String>,

    #[clap(long, short = 'r', group = "revs")]
    revset: Option<String>,

    #[clap(long, short = 'a', group = "revs")]
    all: bool,

    #[clap(long, short = 'f')]
    force: bool,
}

#[cfg(test)]
impl PushOptions {
    pub fn with_message<S>(mut self, message: Option<S>) -> Self
    where
        S: Into<String>,
    {
        self.message = message.map(|s| s.into());
        self
    }

    pub fn with_revset<S>(mut self, revset: Option<S>) -> Self
    where
        S: Into<String>,
    {
        self.revset = revset.map(|s| s.into());
        self
    }

    pub fn with_force(mut self, val: bool) -> Self {
        self.force = val;
        self
    }
}

async fn do_push_single<H: AsRef<str>>(
    jj: &crate::jj::Jujutsu,
    config: &crate::config::Config,
    opts: &PushOptions,
    revision: &mut crate::jj::Revision,
    base_ref: String,
    head_branch: H,
) -> Result<()> {
    let base_oid = jj.git_repo.revparse_single(base_ref.as_str())?.id();
    let head_oid = jj
        .git_repo
        .revparse_single(
            format!("{}/{}", config.remote_name.as_str(), head_branch.as_ref()).as_str(),
        )
        .map(|o| o.id())
        .unwrap_or(base_oid.clone());

    let head_tree = jj.get_tree_oid_for_commit(head_oid).map_err(|mut err| {
        err.push("tree_oid_for_commit".into());
        err
    })?;

    let target_oid = jj
        .resolve_revision_to_commit_id(revision.id.as_ref())
        .map_err(|mut err| {
            err.push("resolve revision".into());
            err
        })?;
    let target_tree = jj.get_tree_oid_for_commit(target_oid).map_err(|mut err| {
        err.push("resolve tree".into());
        err
    })?;

    let base_base = jj
        .git_repo
        .merge_base(head_oid, base_oid)
        .map_err(|err| std::io::Error::new(ErrorKind::InvalidInput, err.to_string()))?;
    let parents: &[Oid] = if base_base != base_oid {
        &[head_oid, base_oid]
    } else {
        &[head_oid]
    };

    if target_tree == head_tree && base_base == base_oid {
        let message = if let Some(pr) = revision.pull_request_number {
            format!("No update necessary for #{}", config.pull_request_url(pr))
        } else {
            "No update necessary".into()
        };
        output("✅", message.as_str())?;
        return Ok(());
    }

    if !opts.force
        && let Some(old) = revision.message.get(&MessageSection::LastCommit)
        && git2::Oid::from_str(old)? != head_oid
    {
        return Err(crate::error::Error::new(format!(
            "Cannot update {}. It has an unexpected upstream.",
            config.pull_request_url(revision.pull_request_number.unwrap_or(0))
        )));
    }

    let message = if head_oid == base_oid
        && let Some(title) = revision.message.get(&MessageSection::Title)
    {
        format!("{}\n\n{}", title, build_github_body(&revision.message))
    } else if let Some(ref msg) = opts.message {
        msg.clone()
    } else {
        dialoguer::Input::<String>::new()
            .with_prompt("Message")
            .with_initial_text("")
            .allow_empty(true)
            .interact_text()?
    };

    // Create the new commit
    let pr_commit = jj
        .create_derived_commit(
            target_oid,
            &format!("{}\n\nCreated using jj-spr", message),
            target_tree,
            parents,
        )
        .map_err(|mut err| {
            err.push("derive commit".into());
            err
        })?;

    revision
        .message
        .insert(MessageSection::LastCommit, pr_commit.clone().to_string());
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("-C")
        .arg(jj.git_repo.path())
        .arg("push")
        .arg("--atomic")
        .arg("--no-verify")
        .arg("--")
        .arg(&config.remote_name)
        .arg(format!("{}:refs/heads/{}", pr_commit, head_branch.as_ref()));

    run_command(&mut cmd)
        .await
        .reword("git push failed".to_string())?;

    if let Some(pr) = revision.pull_request_number {
        if parents.len() == 1 {
            output("✅", format!("Updated {}", config.pull_request_url(pr)))?;
        } else {
            output("✅", format!("Rebased {}", config.pull_request_url(pr)))?;
        }
    };
    Ok(())
}

#[derive(Debug, Clone)]
struct BranchAction {
    revision: crate::jj::Revision,
    head_branch: String,
    base_branch: String,
    existing_nr: Option<u64>,
}

async fn do_push<I, PR>(
    config: &crate::config::Config,
    jj: &crate::jj::Jujutsu,
    opts: &PushOptions,
    revisions: I,
    trunk_oid: Oid,
) -> Result<Vec<BranchAction>>
where
    PR: crate::github::GHPullRequest,
    I: IntoIterator<Item = (crate::jj::Revision, Option<PR>)>,
{
    // ChangeID, head branch, base branch, existing pr
    let mut seen: Vec<BranchAction> = Vec::new();
    for (mut revision, maybe_pr) in revisions.into_iter() {
        let head_ref: String = if let Some(ref pr) = maybe_pr {
            pr.head_branch_name().into()
        } else if let Some(bookmark) = revision.bookmarks.first() {
            bookmark.clone()
        } else {
            // We have to come up with something...
            let title = revision
                .message
                .get(&MessageSection::Title)
                .map(|t| &t[..])
                .unwrap_or("");
            config.get_new_branch_name(&jj.get_all_ref_names()?, title)
        };
        let base_ref = if let Some(ref pr) = maybe_pr {
            if pr.base_branch_name() == config.master_ref {
                Some(trunk_oid.to_string())
            } else {
                Some(format!("{}/{}", config.remote_name, pr.base_branch_name()))
            }
        } else if let Some(ba) = seen
            .iter()
            .find(|ba| ba.revision.id == revision.parent_ids[0])
        {
            // Ok, there is no PR. We'll have to guess a good parent.
            Some(format!("{}/{}", config.remote_name, ba.head_branch))
        } else {
            None
        };

        do_push_single(
            jj,
            config,
            opts,
            &mut revision,
            base_ref.clone().unwrap_or(trunk_oid.clone().to_string()),
            &head_ref,
        )
        .await
        .map_err(|mut err| {
            err.push("do_stacked".into());
            err
        })?;

        seen.push(BranchAction {
            revision,
            head_branch: head_ref,
            base_branch: base_ref
                .and_then(|r| {
                    r.strip_prefix(format!("{}/", config.remote_name).as_str())
                        .map(|s| s.into())
                })
                .unwrap_or(config.master_ref.clone()),
            existing_nr: maybe_pr.map(|pr| pr.pr_number()),
        });
    }
    Ok(seen)
}

fn prepare_revision_comment(tree: &crate::tree::Tree<crate::jj::Revision>) -> Vec<String> {
    let mut lines = Vec::new();
    // The node itself doesn't need indents.
    // It is indented by the parent if necessary
    lines.push(format!(
        "• [{}]({})",
        tree.get().title,
        tree.get().pull_request_number.unwrap_or(0)
    ));

    let children = tree.get_children();
    match children.as_slice() {
        [] => {}
        [next] => {
            lines.extend(prepare_revision_comment(next));
        }
        // We have more than one child branch.
        // We need to actually build an unicode-art tree
        children => {
            let mut child_lines = Vec::new();
            for child in children {
                let indent = [String::from(" ")]
                    .into_iter()
                    .cycle()
                    .take(child.width() * 2 - 1)
                    .reduce(|l, r| format!("{l}{r}"))
                    .unwrap_or(String::from(" "));
                let new_lines = prepare_revision_comment(child);
                let old_lines = child_lines.into_iter().map(|l| format!("│{}{}", indent, l));
                child_lines = old_lines.collect();
                child_lines.extend(new_lines);
            }

            lines.extend(child_lines);
        }
    }

    return lines;
}

fn finalize_revision_comment(revision: &crate::jj::Revision, prepared: &Vec<String>) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "This PR is part of a {} changes series",
        prepared.len()
    ));

    lines.extend_from_slice(prepared.as_slice());
    if let Some(number) = revision.pull_request_number {
        let pattern = format!("[{}]({})", revision.title, number);
        lines = lines
            .into_iter()
            .map(|s| {
                if s.contains(&pattern) {
                    s.replace(&pattern, &revision.title)
                } else {
                    s
                }
            })
            .collect();
    }

    lines.join("\n")
}

pub async fn push<GH, PR>(
    jj: &mut crate::jj::Jujutsu,
    mut gh: GH,
    config: &crate::config::Config,
    opts: PushOptions,
) -> Result<()>
where
    PR: crate::github::GHPullRequest,
    GH: crate::github::GitHubAdapter<PRAdapter = PR>,
{
    let heads = opts
        .revset
        .as_ref()
        .map(|s| RevSet::from_arg(s))
        .unwrap_or(if opts.all {
            RevSet::mutable().heads()
        } else {
            RevSet::current()
        });
    // Get revisions to process
    // The pattern builds:
    // * ::@: Every ancestor of the current revision (including the current reveision)
    // * ~: except if it's also
    // * immutable(): Commits jj considers "merged"
    // * |: or (notice that this ORs the exclusion)
    // * description(""): does not have a description
    // i.e. all revisions between the current and upstream that have descriptions.
    // This somewhat funky pattern allows us to work both in the `jj new` case where changes need to be squashed into the main revision
    // and in the `jj edit` (or `jj new` + `jj describe`) case where the current `@` is the intended PR commit.
    let revset = heads
        .ancestors()
        .without(&RevSet::immutable().or(&RevSet::description("exact:\"\"")));
    let revisions = jj.read_revision_range(config, &revset)?;

    let blockers = jj.revset_to_change_ids(
        &revset.and(
            &RevSet::conflicts()
                .or(&RevSet::divergent())
                .or(&RevSet::merges()),
        ),
    )?;
    if !blockers.is_empty() {
        return Err(Error::new(format!(
            "Found invalid commits: {:?}. (They can be divergent, conflicted or merge commits.)",
            blockers
        )));
    }

    // At this point it's guaranteed that our commits are single parent and the chain goes up to trunk()
    // We need the trunk's commit's OID. The first pull request (made against upstream trunk) needs it to start the chain.
    let trunk_oid = if let Some(first_revision) = revisions.first() {
        jj.resolve_revision_to_commit_id(first_revision.parent_ids[0].as_ref())
    } else {
        output("👋", "No commits found - nothing to do. Good bye!")?;
        return Ok(());
    }?;

    let pull_requests = gh
        .pull_requests(revisions.iter().map(|r| r.pull_request_number))
        .await?;

    let mut actions = do_push(config, jj, &opts, zip(revisions, pull_requests), trunk_oid).await?;
    for action in actions.iter_mut().into_iter() {
        // We don't know what to do with these yet...
        if let Some(_) = action.existing_nr {
            // This will at least write the current commit message.
            jj.update_revision_message(&action.revision)?;
            continue;
        }

        let title = action
            .revision
            .message
            .get(&MessageSection::Title)
            .map_or("Missing Title", |s| s.as_str());
        let body = action
            .revision
            .message
            .get(&MessageSection::Summary)
            .map_or("", |s| s.as_str());
        let pr = gh
            .new_pull_request(title, body, &action.base_branch, &action.head_branch, false)
            .await?;
        if let Some(reviewers) = action.revision.message.get(&MessageSection::Reviewers) {
            gh.add_reviewers(&pr, reviewers.split(",").map(|s| s.trim()))
                .await?;
        }
        if let Some(assignees) = action.revision.message.get(&MessageSection::Assignees) {
            gh.add_assignees(&pr, assignees.split(",").map(|s| s.trim()))
                .await?;
        }

        let pull_request_url = config.pull_request_url(pr.pr_number());

        output(
            "✨",
            &format!(
                "Created new Pull Request #{}: {}",
                pr.pr_number(),
                &pull_request_url,
            ),
        )?;

        action
            .revision
            .message
            .insert(MessageSection::PullRequest, pull_request_url);
        action.revision.pull_request_number = Some(pr.pr_number());

        jj.update_revision_message(&action.revision)?;
    }

    let mut forest: crate::tree::Forest<crate::jj::Revision> = crate::tree::Forest::new();
    for ba in actions {
        let parent = ba
            .revision
            .parent_ids
            .first()
            .map(|p| p.clone())
            .ok_or_else(|| {
                crate::error::Error::new(format!(
                    "Found reivions {:?} in postprocessing that has no parents..?",
                    ba.revision.id
                ))
            })?;
        forest.insert_below(&|p: &crate::jj::Revision| p.id == parent, ba.revision);
    }

    for tree in forest.into_trees() {
        let prepared = prepare_revision_comment(&tree);
        for rev in tree.into_iter() {
            match rev.pull_request_number {
                Some(number) => {
                    let content = finalize_revision_comment(&rev, &prepared);
                    gh.update_pr_comment(number, &content).await?;
                }
                None => {
                    output(
                        "X",
                        format!(
                            "Change {:?} has no PR attached. This is a bug at this point",
                            rev.id
                        ),
                    )?;
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
pub mod tests {
    use crate::jj::{ChangeId, RevSet};
    use crate::testing;
    use std::fs;

    fn amend_jujutsu_revision(jj: &mut crate::jj::Jujutsu, file_content: &str) {
        // Create a file
        let file_path = jj
            .git_repo
            .workdir()
            .expect("Failed to extract workdir from JJ handle")
            .join("test.txt");
        fs::write(&file_path, file_content).expect("Failed to write test file");

        jj.squash().expect("Failed to squash revision");
    }

    fn create_jujutsu_commit_in_file(
        jj: &mut crate::jj::Jujutsu,
        message: &str,
        file_content: &str,
        path: &str,
    ) -> ChangeId {
        // Create a file
        let file_path = jj
            .git_repo
            .workdir()
            .expect("Failed to extract workdir from JJ handle")
            .join(path);
        fs::write(&file_path, file_content).expect("Failed to write test file");

        jj.commit(message).expect("Failed to commit revision");
        ChangeId::from(
            jj.revset_to_change_id(&RevSet::current().parent())
                .expect("Failed to get changeid of '@-'"),
        )
    }

    fn create_jujutsu_commit(
        jj: &mut crate::jj::Jujutsu,
        message: &str,
        file_content: &str,
    ) -> ChangeId {
        create_jujutsu_commit_in_file(jj, message, file_content, "test.txt")
    }

    #[tokio::test]
    async fn test_single_on_head() {
        let (_temp_dir, mut jj, bare) = testing::setup::repo_with_origin();
        let trunk_oid = jj
            .git_repo
            .refname_to_id("HEAD")
            .expect("Failed to revparse HEAD");

        let _ = create_jujutsu_commit(&mut jj, "Test commit", "file 1");

        let gh = crate::github::fakes::GitHub {
            pull_requests: std::collections::BTreeMap::new(),
        };
        super::push(
            &mut jj,
            gh,
            &testing::config::basic(),
            super::PushOptions::default().with_message(Some("message")),
        )
        .await
        .expect("stacked shouldn't fail");

        // Validate the initial push looks good
        let pr_branch = bare
            .find_branch("spr/test/test-commit", git2::BranchType::Local)
            .expect("Expected to find branch on bare upstream");
        let pr_oid = pr_branch
            .get()
            .target()
            .expect("Failed to get oid from pr branch");
        assert!(trunk_oid != pr_oid, "PR and trunk should not be equal");
        assert!(
            bare.merge_base(pr_oid, trunk_oid)
                .expect("Failed to get merge oid")
                == trunk_oid,
            "PR branch was not based on trunk"
        );
    }

    #[tokio::test]
    async fn test_update_pr_on_change() {
        let (_temp_dir, mut jj, bare) = testing::setup::repo_with_origin();

        let _ = create_jujutsu_commit(&mut jj, "Test commit", "file 1");
        let mut gh = crate::github::fakes::GitHub {
            pull_requests: std::collections::BTreeMap::new(),
        };
        super::push(
            &mut jj,
            &mut gh,
            &testing::config::basic(),
            super::PushOptions::default().with_message(Some("message")),
        )
        .await
        .expect("stacked shouldn't fail");

        let pr_branch = bare
            .find_branch("spr/test/test-commit", git2::BranchType::Local)
            .expect("Expected to find branch on bare upstream");
        let initial_pr_oid = pr_branch
            .get()
            .target()
            .expect("Failed to get oid from pr branch");

        amend_jujutsu_revision(&mut jj, "file 2");
        super::push(
            &mut jj,
            &mut gh,
            &testing::config::basic(),
            super::PushOptions::default().with_message(Some("message")),
        )
        .await
        .expect("stacked shouldn't fail");

        let pr_branch = bare
            .find_branch("spr/test/test-commit", git2::BranchType::Local)
            .expect("Expected to find branch on bare upstream");
        let pr_oid = pr_branch
            .get()
            .target()
            .expect("Failed to get oid from pr branch");
        assert!(
            bare.merge_base(pr_oid, initial_pr_oid)
                .expect("Failed to get merge oid")
                == initial_pr_oid,
            "PR branch was not based on previous commit"
        );
    }

    #[tokio::test]
    async fn test_stack_on_existing() {
        let (_temp_dir, mut jj, bare) = testing::setup::repo_with_origin();
        let _ = create_jujutsu_commit(&mut jj, "Test commit", "file 1");
        let mut gh = crate::github::fakes::GitHub {
            pull_requests: std::collections::BTreeMap::new(),
        };
        super::push(
            &mut jj,
            &mut gh,
            &testing::config::basic(),
            super::PushOptions::default().with_message(Some("message")),
        )
        .await
        .expect("stacked shouldn't fail");

        let pr_branch = bare
            .find_branch("spr/test/test-commit", git2::BranchType::Local)
            .expect("Expected to find branch on bare upstream");
        let initial_pr_oid = pr_branch
            .get()
            .target()
            .expect("Failed to get oid from pr branch");

        let _ = create_jujutsu_commit(&mut jj, "Test other commit", "file other");
        super::push(
            &mut jj,
            &mut gh,
            &testing::config::basic(),
            super::PushOptions::default().with_message(Some("message")),
        )
        .await
        .expect("stacked shouldn't fail");

        let pr_branch = bare
            .find_branch("spr/test/test-commit", git2::BranchType::Local)
            .expect("Expected to find branch on bare upstream");
        let pr_oid = pr_branch
            .get()
            .target()
            .expect("Failed to get oid from pr branch");
        assert_eq!(pr_oid, initial_pr_oid, "PR was changed while pushing child");

        let child_pr_branch = bare
            .find_branch("spr/test/test-other-commit", git2::BranchType::Local)
            .expect("Expected to find other branch on bare upstream");
        let child_pr_oid = child_pr_branch
            .get()
            .target()
            .expect("Failed to get other oid from pr branch");
        assert!(
            bare.merge_base(pr_oid, child_pr_oid)
                .expect("Failed to get merge oid")
                == pr_oid,
            "child PR branch was not based on PR"
        );
    }

    #[tokio::test]
    async fn stack_multi_in_pr() {
        let (_temp_dir, mut jj, bare) = testing::setup::repo_with_origin();
        let trunk_oid = jj
            .git_repo
            .refname_to_id("HEAD")
            .expect("Failed to revparse HEAD");

        let _ = create_jujutsu_commit(&mut jj, "Test commit", "file 1");
        let _ = create_jujutsu_commit(&mut jj, "Test other commit", "file other");
        let mut gh = crate::github::fakes::GitHub {
            pull_requests: std::collections::BTreeMap::new(),
        };
        super::push(
            &mut jj,
            &mut gh,
            &testing::config::basic(),
            super::PushOptions::default().with_message(Some("message")),
        )
        .await
        .expect("stacked shouldn't fail");

        let pr_branch = bare
            .find_branch("spr/test/test-commit", git2::BranchType::Local)
            .expect("Expected to find branch on bare upstream");
        let pr_oid = pr_branch
            .get()
            .target()
            .expect("Failed to get oid from pr branch");
        assert!(pr_oid != trunk_oid, "base PR was equal to trunk");

        let child_pr_branch = bare
            .find_branch("spr/test/test-other-commit", git2::BranchType::Local)
            .expect("Expected to find other branch on bare upstream");
        let child_pr_oid = child_pr_branch
            .get()
            .target()
            .expect("Failed to get other oid from pr branch");
        assert!(
            bare.merge_base(pr_oid, child_pr_oid)
                .expect("Failed to get merge oid")
                == pr_oid,
            "child PR branch was not based on PR"
        );
    }

    #[tokio::test]
    async fn no_rebase_when_change_is_not_rebased() {
        let (_temp_dir, mut jj, bare) = testing::setup::repo_with_origin();
        let trunk_oid = jj
            .git_repo
            .refname_to_id("HEAD")
            .expect("Failed to revparse HEAD");

        let _ = create_jujutsu_commit(&mut jj, "Test commit", "file 1");
        let mut gh = crate::github::fakes::GitHub {
            pull_requests: std::collections::BTreeMap::new(),
        };
        super::push(
            &mut jj,
            &mut gh,
            &testing::config::basic(),
            super::PushOptions::default().with_message(Some("message")),
        )
        .await
        .expect("stacked shouldn't fail");

        let pr_branch = bare
            .find_branch("spr/test/test-commit", git2::BranchType::Local)
            .expect("Expected to find branch on bare upstream");
        let initial_pr_oid = pr_branch
            .get()
            .target()
            .expect("Failed to get oid from pr branch");

        let updated_trunk_oid =
            testing::git::add_commit_on_and_push_to_remote(&jj.git_repo, "main", [trunk_oid]);

        super::push(
            &mut jj,
            &mut gh,
            &testing::config::basic(),
            super::PushOptions::default().with_message(Some("message")),
        )
        .await
        .expect("stacked shouldn't fail");

        let pr_branch = bare
            .find_branch("spr/test/test-commit", git2::BranchType::Local)
            .expect("Expected to find branch on bare upstream");
        let pr_oid = pr_branch
            .get()
            .target()
            .expect("Failed to get oid from pr branch");
        assert!(
            bare.merge_base(pr_oid, initial_pr_oid)
                .expect("Failed to get merge oid")
                == initial_pr_oid,
            "PR branch was not based on previous commit"
        );
        let head_base = bare
            .merge_base(pr_oid, updated_trunk_oid)
            .expect("Failed to get merge oid");
        assert!(head_base != updated_trunk_oid, "PR was rebased to HEAD");
        assert!(
            head_base == trunk_oid,
            "Pr HEAD is no longer based on the previous trunk"
        );
    }

    #[tokio::test]
    async fn rebase_to_new_base() {
        let (_temp_dir, mut jj, bare) = testing::setup::repo_with_origin();
        let trunk_oid = jj
            .git_repo
            .refname_to_id("HEAD")
            .expect("Failed to revparse HEAD");

        let rev = create_jujutsu_commit(&mut jj, "Test commit", "file 1");
        let mut gh = crate::github::fakes::GitHub {
            pull_requests: std::collections::BTreeMap::new(),
        };
        super::push(
            &mut jj,
            &mut gh,
            &testing::config::basic(),
            super::PushOptions::default().with_message(Some("message")),
        )
        .await
        .expect("stacked shouldn't fail");

        let pr_branch = bare
            .find_branch("spr/test/test-commit", git2::BranchType::Local)
            .expect("Expected to find branch on bare upstream");
        let initial_pr_oid = pr_branch
            .get()
            .target()
            .expect("Failed to get oid from pr branch");

        let updated_trunk_oid = testing::git::add_commit_on_and_push_to_remote_file(
            &jj.git_repo,
            "main",
            [trunk_oid],
            "file.txt",
        );
        jj.update().expect("Update isn't supposed to fail");
        let updated_trunk_change_id = {
            let commit = jj
                .git_repo
                .find_commit(updated_trunk_oid)
                .expect("Should be able to find newly created commit");

            jj.revset_to_change_id(&RevSet::from(&commit))
                .expect("Should be able to find a jj id for the commit")
        };

        jj.rebase_branch(&RevSet::from(&rev), updated_trunk_change_id)
            .expect("Failed to rebase revision");
        super::push(
            &mut jj,
            &mut gh,
            &testing::config::basic(),
            super::PushOptions::default().with_message(Some("message")),
        )
        .await
        .expect("stacked shouldn't fail");

        let pr_branch = bare
            .find_branch("spr/test/test-commit", git2::BranchType::Local)
            .expect("Expected to find branch on bare upstream");
        let pr_oid = pr_branch
            .get()
            .target()
            .expect("Failed to get oid from pr branch");
        assert!(
            bare.merge_base(pr_oid, initial_pr_oid)
                .expect("Failed to get merge oid")
                == initial_pr_oid,
            "PR branch was not based on previous commit"
        );
        let head_base = bare
            .merge_base(pr_oid, updated_trunk_oid)
            .expect("Failed to get merge oid");
        assert!(head_base == updated_trunk_oid, "PR was not rebased to HEAD");
    }

    #[tokio::test]
    async fn rebase_stacked_pr() {
        let (_temp_dir, mut jj, bare) = testing::setup::repo_with_origin();
        let trunk_oid = jj
            .git_repo
            .refname_to_id("HEAD")
            .expect("Failed to revparse HEAD");

        let rev = create_jujutsu_commit(&mut jj, "Test commit", "file 1");
        let child_rev =
            create_jujutsu_commit_in_file(&mut jj, "Test other commit", "file other", "other file");

        let mut gh = crate::github::fakes::GitHub {
            pull_requests: std::collections::BTreeMap::new(),
        };
        super::push(
            &mut jj,
            &mut gh,
            &testing::config::basic(),
            super::PushOptions::default().with_message(Some("message")),
        )
        .await
        .expect("stacked shouldn't fail");

        let pr_branch = bare
            .find_branch("spr/test/test-commit", git2::BranchType::Local)
            .expect("Expected to find other branch on bare upstream");
        let initial_pr_oid = pr_branch
            .get()
            .target()
            .expect("Failed to get other oid from pr branch");

        let child_pr_branch = bare
            .find_branch("spr/test/test-other-commit", git2::BranchType::Local)
            .expect("Expected to find other branch on bare upstream");
        let initial_child_pr_oid = child_pr_branch
            .get()
            .target()
            .expect("Failed to get other oid from pr branch");

        jj.new_revision(Some(RevSet::from(&rev)), None as Option<&str>, false)
            .expect("Failed to create new revision");
        amend_jujutsu_revision(&mut jj, "file 2");
        jj.new_revision(Some(RevSet::from(&child_rev)), None as Option<&str>, false)
            .expect("Failed to create new revision");
        super::push(
            &mut jj,
            &mut gh,
            &testing::config::basic(),
            super::PushOptions::default().with_message(Some("message")),
        )
        .await
        .expect("stacked shouldn't fail");

        let pr_branch = bare
            .find_branch("spr/test/test-commit", git2::BranchType::Local)
            .expect("Expected to find branch on bare upstream");
        let pr_oid = pr_branch
            .get()
            .target()
            .expect("Failed to get oid from pr branch");
        assert!(pr_oid != trunk_oid, "base PR was equal to trunk");
        assert_ne!(initial_pr_oid, pr_oid, "Base PR wasn't amended");

        let child_pr_branch = bare
            .find_branch("spr/test/test-other-commit", git2::BranchType::Local)
            .expect("Expected to find other branch on bare upstream");
        let child_pr_oid = child_pr_branch
            .get()
            .target()
            .expect("Failed to get other oid from pr branch");
        assert!(
            bare.merge_base(pr_oid, child_pr_oid)
                .expect("Failed to get merge oid")
                == pr_oid,
            "child PR branch was not based on PR"
        );
        assert!(
            bare.merge_base(initial_child_pr_oid, child_pr_oid)
                .expect("Failed to get merge oid")
                == initial_child_pr_oid,
            "child PR branch was not based on initial child PR"
        );
    }

    #[tokio::test]
    async fn test_no_update_without_change() {
        let (_temp_dir, mut jj, bare) = testing::setup::repo_with_origin();
        let _ = create_jujutsu_commit(&mut jj, "Test commit", "file 1");
        let mut gh = crate::github::fakes::GitHub {
            pull_requests: std::collections::BTreeMap::new(),
        };
        super::push(
            &mut jj,
            &mut gh,
            &testing::config::basic(),
            super::PushOptions::default().with_message(Some("message")),
        )
        .await
        .expect("stacked shouldn't fail");

        let pr_branch = bare
            .find_branch("spr/test/test-commit", git2::BranchType::Local)
            .expect("Expected to find branch on bare upstream");
        let initial_pr_oid = pr_branch
            .get()
            .target()
            .expect("Failed to get oid from pr branch");
        super::push(
            &mut jj,
            &mut gh,
            &testing::config::basic(),
            super::PushOptions::default().with_message(Some("message")),
        )
        .await
        .expect("stacked shouldn't fail");

        let pr_branch = bare
            .find_branch("spr/test/test-commit", git2::BranchType::Local)
            .expect("Expected to find branch on bare upstream");
        let pr_oid = pr_branch
            .get()
            .target()
            .expect("Failed to get oid from pr branch");
        assert!(pr_oid == initial_pr_oid, "PR was updated without changes");
    }

    #[tokio::test]
    async fn test_use_existing_bookmark() {
        let (_temp_dir, mut jj, bare) = testing::setup::repo_with_origin();
        let trunk_oid = jj
            .git_repo
            .refname_to_id("HEAD")
            .expect("Failed to revparse HEAD");

        let commit_id = create_jujutsu_commit(&mut jj, "Test commit", "file 1");

        // Create a bookmark for the current commit
        jj.bookmark_create("my-custom-bookmark", Some(commit_id.as_ref()))
            .expect("Failed to create bookmark");

        let gh = crate::github::fakes::GitHub {
            pull_requests: std::collections::BTreeMap::new(),
        };
        super::push(
            &mut jj,
            gh,
            &testing::config::basic(),
            super::PushOptions::default().with_message(Some("message")),
        )
        .await
        .expect("stacked shouldn't fail");

        // Validate that the PR was created with the bookmark name
        let pr_branch = bare
            .find_branch("my-custom-bookmark", git2::BranchType::Local)
            .expect("Expected to find branch 'my-custom-bookmark' on bare upstream");

        let pr_oid = pr_branch
            .get()
            .target()
            .expect("Failed to get oid from pr branch");
        assert!(trunk_oid != pr_oid, "PR and trunk should not be equal");
    }

    #[tokio::test]
    async fn independent_heads() {
        let (_temp_dir, mut jj, _) = testing::setup::repo_with_origin();
        let base_change = jj
            .revset_to_change_id(&RevSet::current().parent())
            .expect("Should be able to find the base commit");

        let _ = create_jujutsu_commit(&mut jj, "Test commit", "file 1");
        let left_id = create_jujutsu_commit(&mut jj, "Other commit", "file 2");
        jj.new_revision(
            Some(RevSet::from(&base_change)),
            None as Option<&str>,
            false,
        )
        .expect("Couldn't create new revision on base");
        let right_id = create_jujutsu_commit(&mut jj, "More commit", "file 3");

        let mut gh = crate::github::fakes::GitHub {
            pull_requests: std::collections::BTreeMap::new(),
        };
        super::push(
            &mut jj,
            &mut gh,
            &testing::config::basic(),
            super::PushOptions::default()
                .with_message(Some("message"))
                .with_revset(Some(
                    RevSet::from(&left_id).or(&RevSet::from(&right_id)).as_ref(),
                )),
        )
        .await
        .expect("stacked shouldn't fail");

        let left_revision = jj
            .read_revision(&testing::config::basic(), left_id)
            .expect("Couldn't read left revision");
        let right_revision = jj
            .read_revision(&testing::config::basic(), right_id)
            .expect("Couldn't read right revision");

        assert_eq!(
            gh.pull_requests
                .get(
                    &right_revision
                        .pull_request_number
                        .expect("couldn't get PR# from right revision")
                )
                .expect("Couldn't get PR from right revision")
                .base,
            testing::config::basic().master_ref,
            "Right revision PR was created to wrong branch"
        );
        assert_ne!(
            gh.pull_requests
                .get(
                    &left_revision
                        .pull_request_number
                        .expect("couldn't get PR# from left revision")
                )
                .expect("Couldn't get PR from left revision")
                .base,
            testing::config::basic().master_ref,
            "left revision PR was created to wrong branch"
        );
    }

    mod intended_fails {
        use crate::{jj::RevSet, testing};

        #[tokio::test]
        async fn multi_parent() {
            let (_temp_dir, mut jj, _) = testing::setup::repo_with_origin();

            jj.new_revision(Some(RevSet::root()), Some("Left"), false)
                .expect("Failed to create left revision");
            let left = jj
                .revset_to_change_id(&RevSet::current())
                .expect("Failed to resolve left change id");
            jj.new_revision(Some(RevSet::root()), Some("Right"), false)
                .expect("Failed to create left revision");
            let right = jj
                .revset_to_change_id(&RevSet::current())
                .expect("Failed to resolve left change id");

            let _ = jj
                .new_revision(
                    Some(RevSet::from(&left).or(&RevSet::from(&right))),
                    Some("Parent"),
                    false,
                )
                .expect("Failed to create left revision");

            let mut gh = crate::github::fakes::GitHub {
                pull_requests: std::collections::BTreeMap::new(),
            };
            super::super::push(
                &mut jj,
                &mut gh,
                &testing::config::basic(),
                super::super::PushOptions::default().with_message(Some("message")),
            )
            .await
            .expect_err("Stacked should refuse to handle multi-parent change");
        }

        #[tokio::test]
        async fn conflicted() {
            let (_temp_dir, mut jj, _) = testing::setup::repo_with_origin();

            jj.new_revision(Some(RevSet::root()), None as Option<&'static str>, false)
                .expect("Failed to create left revision");
            let main = super::create_jujutsu_commit(&mut jj, "Message", "content 1");
            jj.new_revision(Some(RevSet::root()), None as Option<&str>, false)
                .expect("Failed to create left revision");
            let second = super::create_jujutsu_commit(&mut jj, "Other", "content 2");

            jj.squash_from_into(&RevSet::from(&second), &RevSet::from(&main))
                .expect("Didn't expect squash to fail");
            jj.new_revision(Some(RevSet::from(&main)), None as Option<&str>, false)
                .expect("Failed to create new change on conflicted change :o");

            let mut gh = crate::github::fakes::GitHub {
                pull_requests: std::collections::BTreeMap::new(),
            };
            super::super::push(
                &mut jj,
                &mut gh,
                &testing::config::basic(),
                super::super::PushOptions::default().with_message(Some("message")),
            )
            .await
            .expect_err("Stacked should refuse to handle change with conflicts");
        }
    }

    pub mod fore_testing {
        use crate::testing;

        pub async fn setup() -> (
            tempfile::TempDir,
            crate::jj::Jujutsu,
            crate::github::fakes::GitHub,
        ) {
            let (temp_dir, mut jj, _) = testing::setup::repo_with_origin();
            super::create_jujutsu_commit(&mut jj, "Test Commit", "my change");

            let mut gh = crate::github::fakes::GitHub {
                pull_requests: std::collections::BTreeMap::new(),
            };
            super::super::push(
                &mut jj,
                &mut gh,
                &testing::config::basic(),
                super::super::PushOptions::default().with_message(Some("message")),
            )
            .await
            .expect("Do not expect sync to fail during setup phase");

            {
                let branch = jj
                    .git_repo
                    .find_branch("origin/spr/test/test-commit", git2::BranchType::Remote)
                    .expect("Should be able to find remote branch");
                let oid = branch
                    .into_reference()
                    .target()
                    .expect("Remtoe branch should have an OID");

                let _ = testing::git::add_commit_on_and_push_to_remote_file(
                    &jj.git_repo,
                    "spr/test/test-commit",
                    [oid],
                    "other-file",
                );
            }
            super::amend_jujutsu_revision(&mut jj, "changed change");

            (temp_dir, jj, gh)
        }

        #[tokio::test]
        async fn fails_when_remote_is_ahead() {
            let (_temp_dir, mut jj, mut gh) = setup().await;

            super::super::push(
                &mut jj,
                &mut gh,
                &testing::config::basic(),
                super::super::PushOptions::default().with_message(Some("message")),
            )
            .await
            .expect_err("push should fail when upstream is ahead of what we expect");
        }

        #[tokio::test]
        async fn force_ignores_ahead_upstream() {
            let (_temp_dir, mut jj, mut gh) = setup().await;

            super::super::push(
                &mut jj,
                &mut gh,
                &testing::config::basic(),
                super::super::PushOptions::default()
                    .with_message(Some("message"))
                    .with_force(true),
            )
            .await
            .expect("push should succeed with --force flag and ahead upstream");
        }
    }

    mod user_assignments {
        use crate::testing;

        #[tokio::test]
        async fn reviewers_are_set() {
            let (_temp_dir, mut jj, bare) = testing::setup::repo_with_origin();
            let trunk_oid = jj
                .git_repo
                .refname_to_id("HEAD")
                .expect("Failed to revparse HEAD");

            let _ = super::create_jujutsu_commit(
                &mut jj,
                "Test commit\n\nReviewers: rev1,rev2, rev3",
                "file 1",
            );

            let mut gh = crate::github::fakes::GitHub {
                pull_requests: std::collections::BTreeMap::new(),
            };
            super::super::push(
                &mut jj,
                &mut gh,
                &testing::config::basic(),
                super::super::PushOptions::default().with_message(Some("message")),
            )
            .await
            .expect("stacked shouldn't fail");

            // Validate the initial push looks good
            let pr_branch = bare
                .find_branch("spr/test/test-commit", git2::BranchType::Local)
                .expect("Expected to find branch on bare upstream");
            let pr_oid = pr_branch
                .get()
                .target()
                .expect("Failed to get oid from pr branch");
            assert!(trunk_oid != pr_oid, "PR and trunk should not be equal");
            assert!(
                bare.merge_base(pr_oid, trunk_oid)
                    .expect("Failed to get merge oid")
                    == trunk_oid,
                "PR branch was not based on trunk"
            );
            let revs: Vec<&str> = gh
                .pull_requests
                .get(&1)
                .expect("Push must have created PR")
                .reviewers
                .iter()
                .map(|s| s.as_str())
                .collect();
            assert_eq!(
                revs.as_slice(),
                &["rev1", "rev2", "rev3"],
                "Reviewers didn't get updated"
            )
        }

        #[tokio::test]
        async fn assignees_are_set() {
            let (_temp_dir, mut jj, bare) = testing::setup::repo_with_origin();
            let trunk_oid = jj
                .git_repo
                .refname_to_id("HEAD")
                .expect("Failed to revparse HEAD");

            let _ = super::create_jujutsu_commit(
                &mut jj,
                "Test commit\n\nAssignees: ass1,ass2, ass3",
                "file 1",
            );

            let mut gh = crate::github::fakes::GitHub {
                pull_requests: std::collections::BTreeMap::new(),
            };
            super::super::push(
                &mut jj,
                &mut gh,
                &testing::config::basic(),
                super::super::PushOptions::default().with_message(Some("message")),
            )
            .await
            .expect("stacked shouldn't fail");

            // Validate the initial push looks good
            let pr_branch = bare
                .find_branch("spr/test/test-commit", git2::BranchType::Local)
                .expect("Expected to find branch on bare upstream");
            let pr_oid = pr_branch
                .get()
                .target()
                .expect("Failed to get oid from pr branch");
            assert!(trunk_oid != pr_oid, "PR and trunk should not be equal");
            assert!(
                bare.merge_base(pr_oid, trunk_oid)
                    .expect("Failed to get merge oid")
                    == trunk_oid,
                "PR branch was not based on trunk"
            );
            let revs: Vec<&str> = gh
                .pull_requests
                .get(&1)
                .expect("Push must have created PR")
                .assignees
                .iter()
                .map(|s| s.as_str())
                .collect();
            assert_eq!(
                revs.as_slice(),
                &["ass1", "ass2", "ass3"],
                "Assignees didn't get updated"
            )
        }
    }

    mod tree_formatting {
        #[test]
        fn single() {
            let lines = super::super::prepare_revision_comment(&crate::tree::Tree::new(
                crate::jj::Revision {
                    id: crate::jj::ChangeId::from("change"),
                    parent_ids: Vec::new(),
                    pull_request_number: Some(1),
                    title: String::from("My Title"),
                    message: std::collections::BTreeMap::new(),
                    bookmarks: Vec::new(),
                },
            ));
            let str_lines: Vec<_> = lines.iter().map(|s| s.as_str()).collect();

            assert_eq!(
                str_lines.as_slice(),
                &["• [My Title](1)"],
                "Lines didn't match expectation"
            );
        }

        #[test]
        fn list() {
            let mut tree = crate::tree::Tree::new(crate::jj::Revision {
                id: crate::jj::ChangeId::from("change"),
                parent_ids: Vec::new(),
                pull_request_number: Some(1),
                title: String::from("My Title"),
                message: std::collections::BTreeMap::new(),
                bookmarks: Vec::new(),
            });
            tree.add_child_value(crate::jj::Revision {
                id: crate::jj::ChangeId::from("change"),
                parent_ids: Vec::new(),
                pull_request_number: Some(2),
                title: String::from("My Other Title"),
                message: std::collections::BTreeMap::new(),
                bookmarks: Vec::new(),
            });
            let lines = super::super::prepare_revision_comment(&tree);
            let str_lines: Vec<_> = lines.iter().map(|s| s.as_str()).collect();

            assert_eq!(
                str_lines.as_slice(),
                &["• [My Title](1)", "• [My Other Title](2)"],
                "Lines didn't match expectation"
            );
        }

        #[test]
        fn tree() {
            let mut tree = crate::tree::Tree::new(crate::jj::Revision {
                id: crate::jj::ChangeId::from("change"),
                parent_ids: Vec::new(),
                pull_request_number: Some(1),
                title: String::from("My Title"),
                message: std::collections::BTreeMap::new(),
                bookmarks: Vec::new(),
            });
            tree.add_child_value(crate::jj::Revision {
                id: crate::jj::ChangeId::from("change"),
                parent_ids: Vec::new(),
                pull_request_number: Some(2),
                title: String::from("My Other Title"),
                message: std::collections::BTreeMap::new(),
                bookmarks: Vec::new(),
            });
            tree.add_child_value(crate::jj::Revision {
                id: crate::jj::ChangeId::from("change"),
                parent_ids: Vec::new(),
                pull_request_number: Some(3),
                title: String::from("My Third Title"),
                message: std::collections::BTreeMap::new(),
                bookmarks: Vec::new(),
            });
            let lines = super::super::prepare_revision_comment(&tree);
            let str_lines: Vec<_> = lines.iter().map(|s| s.as_str()).collect();

            assert_eq!(
                str_lines.as_slice(),
                &[
                    "• [My Title](1)",
                    "│ • [My Other Title](2)",
                    "• [My Third Title](3)"
                ],
                "Lines didn't match: {str_lines:?}",
            );
        }
    }

    mod overview_comments {
        use crate::testing;

        #[tokio::test]
        async fn creates_comment() {
            let (_temp_dir, mut jj, bare) = testing::setup::repo_with_origin();
            let trunk_oid = jj
                .git_repo
                .refname_to_id("HEAD")
                .expect("Failed to revparse HEAD");

            let _ = super::create_jujutsu_commit(&mut jj, "Test commit", "file 1");

            let mut gh = crate::github::fakes::GitHub {
                pull_requests: std::collections::BTreeMap::new(),
            };
            super::super::push(
                &mut jj,
                &mut gh,
                &testing::config::basic(),
                super::super::PushOptions::default().with_message(Some("message")),
            )
            .await
            .expect("stacked shouldn't fail");

            // Validate the initial push looks good
            let pr_branch = bare
                .find_branch("spr/test/test-commit", git2::BranchType::Local)
                .expect("Expected to find branch on bare upstream");
            let pr_oid = pr_branch
                .get()
                .target()
                .expect("Failed to get oid from pr branch");
            assert!(trunk_oid != pr_oid, "PR and trunk should not be equal");
            assert!(
                bare.merge_base(pr_oid, trunk_oid)
                    .expect("Failed to get merge oid")
                    == trunk_oid,
                "PR branch was not based on trunk"
            );
            let comments = gh
                .pull_requests
                .get(&1)
                .expect("Push must have created PR")
                .comments
                .clone();
            assert!(!comments.is_empty(), "Didn't post a PR comment",)
        }

        #[tokio::test]
        async fn updates_existing_comment() {
            let (_temp_dir, mut jj, bare) = testing::setup::repo_with_origin();
            let trunk_oid = jj
                .git_repo
                .refname_to_id("HEAD")
                .expect("Failed to revparse HEAD");

            let _ = super::create_jujutsu_commit(&mut jj, "Test commit", "file 1");

            let mut gh = crate::github::fakes::GitHub {
                pull_requests: std::collections::BTreeMap::new(),
            };
            super::super::push(
                &mut jj,
                &mut gh,
                &testing::config::basic(),
                super::super::PushOptions::default().with_message(Some("message")),
            )
            .await
            .expect("stacked shouldn't fail");
            super::super::push(
                &mut jj,
                &mut gh,
                &testing::config::basic(),
                super::super::PushOptions::default().with_message(Some("message")),
            )
            .await
            .expect("stacked shouldn't fail");

            // Validate the initial push looks good
            let pr_branch = bare
                .find_branch("spr/test/test-commit", git2::BranchType::Local)
                .expect("Expected to find branch on bare upstream");
            let pr_oid = pr_branch
                .get()
                .target()
                .expect("Failed to get oid from pr branch");
            assert!(trunk_oid != pr_oid, "PR and trunk should not be equal");
            assert!(
                bare.merge_base(pr_oid, trunk_oid)
                    .expect("Failed to get merge oid")
                    == trunk_oid,
                "PR branch was not based on trunk"
            );
            let comments = gh
                .pull_requests
                .get(&1)
                .expect("Push must have created PR")
                .comments
                .clone();
            assert!(comments.len() == 1, "Commenting logic double posted",)
        }

        #[tokio::test]
        async fn does_not_update_unrelated() {
            let (_temp_dir, mut jj, bare) = testing::setup::repo_with_origin();
            let trunk_oid = jj
                .git_repo
                .refname_to_id("HEAD")
                .expect("Failed to revparse HEAD");

            let _ = super::create_jujutsu_commit(
                &mut jj,
                "Test commit\n\nPull Request: https://github.com/Ongy/jj-spr/pull/1",
                "file 1",
            );

            let mut pr = crate::github::fakes::PullRequest::new(
                "main",
                "spr/test/test-commit",
                1,
                "Test commit",
                "",
            );
            pr.comments.push(crate::github::fakes::PullRequestComment {
                id: String::from("test-comment"),
                editable: true,
                content: String::from("Some other content"),
            });
            let mut gh = crate::github::fakes::GitHub {
                pull_requests: std::collections::BTreeMap::from([(1, pr)]),
            };
            super::super::push(
                &mut jj,
                &mut gh,
                &testing::config::basic(),
                super::super::PushOptions::default().with_message(Some("message")),
            )
            .await
            .expect("stacked shouldn't fail");

            // Validate the initial push looks good
            let pr_branch = bare
                .find_branch("spr/test/test-commit", git2::BranchType::Local)
                .expect("Expected to find branch on bare upstream");
            let pr_oid = pr_branch
                .get()
                .target()
                .expect("Failed to get oid from pr branch");
            assert!(trunk_oid != pr_oid, "PR and trunk should not be equal");
            assert!(
                bare.merge_base(pr_oid, trunk_oid)
                    .expect("Failed to get merge oid")
                    == trunk_oid,
                "PR branch was not based on trunk"
            );
            let comments = gh
                .pull_requests
                .get(&1)
                .expect("Push must have created PR")
                .comments
                .clone();
            assert!(
                comments.len() == 2,
                "Updated unrelated comment\nHad {} comment(s)",
                comments.len()
            );
        }
    }
}
