use crate::{
    error::{Error, Result, ResultExt},
    github::PullRequest,
    message::{MessageSection, build_github_body},
    output::output,
    utils::run_command,
};
use git2::Oid;
use std::{io::ErrorKind, iter::zip};

#[derive(Debug, clap::Parser)]
pub struct StackedOptions {
    /// Message to be used for commits updating existing pull requests (e.g.
    /// 'rebase' or 'review comments')
    #[clap(long, short = 'm')]
    message: Option<String>,
}

async fn do_stacked<H: AsRef<str>>(
    jj: &crate::jj::Jujutsu,
    config: &crate::config::Config,
    opts: &StackedOptions,
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
            format!("No update necessary for #{}", pr)
        } else {
            "No update necessary".into()
        };
        output("✅", message.as_str())?;
        return Ok(());
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
            output("✅", format!("Updated #{}", pr))?;
        } else {
            output("✅", format!("Rebased #{}", pr))?;
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

async fn handle_revs<I: IntoIterator<Item = (crate::jj::Revision, Option<PullRequest>)>>(
    config: &crate::config::Config,
    jj: &crate::jj::Jujutsu,
    opts: &StackedOptions,
    revisions: I,
    trunk_oid: Oid,
) -> Result<Vec<BranchAction>> {
    // ChangeID, head branch, base branch, existing pr
    let mut seen: Vec<BranchAction> = Vec::new();
    for (mut revision, maybe_pr) in revisions.into_iter() {
        let head_ref: String = if let Some(ref pr) = maybe_pr {
            pr.head.branch_name().into()
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
            if pr.base.is_master_branch() {
                Some(trunk_oid.to_string())
            } else {
                Some(pr.base.local().into())
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

        do_stacked(
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
                .and_then(|r| r.strip_prefix("origin/").map(|s| s.into()))
                .unwrap_or(config.master_ref.branch_name().to_string()),
            existing_nr: maybe_pr.map(|pr| pr.number),
        });
    }
    Ok(seen)
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

pub async fn stacked(
    jj: &crate::jj::Jujutsu,
    gh: &mut crate::github::GitHub,
    config: &crate::config::Config,
    opts: StackedOptions,
) -> Result<()> {
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
    let revisions =
        jj.read_revision_range(config, "::@ ~ (immutable() | description(exact:\"\"))")?;

    // At this point we cannot deal with revisions that have multiple parents :/
    if let Some(r) = revisions.iter().find(|r| r.parent_ids.len() != 1) {
        return Err(Error::new(format!(
            "Found commit with more than one parent {:?}",
            r.id
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

    #[allow(clippy::needless_collect)]
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

    let actions = handle_revs(config, jj, &opts, zip(revisions, pull_requests?), trunk_oid).await?;
    for mut action in actions.into_iter() {
        // We don't know what to do with these yet...
        if let Some(_) = action.existing_nr {
            // This will at least write the current commit message.
            jj.update_revision_message(&action.revision)?;
            continue;
        }

        let pr = gh
            .create_pull_request(
                &action.revision.message,
                action.base_branch,
                action.head_branch,
                false,
            )
            .await?;
        let pull_request_url = config.pull_request_url(pr.number);

        output(
            "✨",
            &format!(
                "Created new Pull Request #{}: {}",
                pr.number, &pull_request_url,
            ),
        )?;

        action
            .revision
            .message
            .insert(MessageSection::PullRequest, pull_request_url);

        jj.update_revision_message(&action.revision)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs::{self, create_dir};
    use tempfile::TempDir;

    use crate::jj::ChangeId;

    use super::handle_revs;

    #[allow(dead_code)]
    fn create_test_config() -> crate::config::Config {
        crate::config::Config::new(
            "test_owner".into(),
            "test_repo".into(),
            "origin".into(),
            "main".into(),
            "spr/test/".into(),
            false,
        )
    }

    #[allow(dead_code)]
    fn create_test_git_repo<P: AsRef<std::path::Path>>(path: P) -> git2::Repository {
        create_dir(&path).expect("Failed to create bare repo");
        let repo = git2::Repository::init_bare(&path).expect("Failed to init git repo");

        repo
    }

    fn clone_repo<P: AsRef<std::path::Path>, Q: AsRef<std::path::Path>>(
        origin: P,
        path: Q,
    ) -> git2::Repository {
        create_dir(&path).expect("Failed to create bare repo");
        let repo = git2::Repository::clone(
            format!(
                "file://{}",
                origin
                    .as_ref()
                    .to_str()
                    .expect("Failed to convert path to str")
            )
            .as_str(),
            &path,
        )
        .expect("Failed to clone repo");

        // Create initial commit
        let signature = git2::Signature::now("Test User", "test@example.com")
            .expect("Failed to create signature");
        let tree_id = {
            let mut index = repo.index().expect("Failed to get index");
            index.write_tree().expect("Failed to write tree")
        };
        let tree = repo.find_tree(tree_id).expect("Failed to find tree");

        let initial_oid = repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "Initial commit",
                &tree,
                &[],
            )
            .expect("Failed to create initial commit");
        drop(tree); // Drop the tree reference before moving repo

        let mut remote = repo.find_remote("origin").expect("Failed to find origin");
        remote
            .push(&[format!("{}:refs/heads/main", initial_oid)], None)
            .expect("Failed to push");
        drop(remote);

        // Initialize a Jujutsu repository
        let output = std::process::Command::new("jj")
            .args(["git", "init", "--colocate"])
            .current_dir(&path)
            .output()
            .expect("Failed to run jj git init");

        if !output.status.success() {
            panic!(
                "Failed to initialize jj repo: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        // Set up basic jj config
        let _ = std::process::Command::new("jj")
            .args(["config", "set", "--repo", "user.name", "Test User"])
            .current_dir(&path)
            .output();

        let _ = std::process::Command::new("jj")
            .args(["config", "set", "--repo", "user.email", "test@example.com"])
            .current_dir(&path)
            .output();

        repo
    }

    fn amend_jujutsu_revision(repo_path: &std::path::Path, file_content: &str) {
        // Create a file
        let file_path = repo_path.join("test.txt");
        fs::write(&file_path, file_content).expect("Failed to write test file");

        // Create a commit using jj
        let output = std::process::Command::new("jj")
            .args(["squash"])
            .current_dir(repo_path)
            .output()
            .expect("Failed to run jj commit");

        if !output.status.success() {
            panic!(
                "Failed to squash jj commit: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    fn rebase_jj_commit(repo_path: &std::path::Path, src: ChangeId, dst: ChangeId) {
        let output = std::process::Command::new("jj")
            .args(["rebase", "-s", src.as_ref(), "-d", dst.as_ref()])
            .current_dir(repo_path)
            .output()
            .expect("Failed to run jj rebase");

        if !output.status.success() {
            panic!(
                "Failed to create jj rebase: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    fn new_on_jj_commit(repo_path: &std::path::Path, base: ChangeId) {
        let output = std::process::Command::new("jj")
            .args(["new", base.as_ref()])
            .current_dir(repo_path)
            .output()
            .expect("Failed to run jj new");

        if !output.status.success() {
            panic!(
                "Failed to create jj new: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    fn create_jujutsu_commit(
        repo_path: &std::path::Path,
        message: &str,
        file_content: &str,
    ) -> String {
        // Create a file
        let file_path = repo_path.join("test.txt");
        fs::write(&file_path, file_content).expect("Failed to write test file");

        // Create a commit using jj
        let output = std::process::Command::new("jj")
            .args(["commit", "-m", message])
            .current_dir(repo_path)
            .output()
            .expect("Failed to run jj commit");

        if !output.status.success() {
            panic!(
                "Failed to create jj commit: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        // Get the change ID of the created commit
        let output = std::process::Command::new("jj")
            .args(["log", "--no-graph", "-r", "@-", "--template", "change_id"])
            .current_dir(repo_path)
            .output()
            .expect("Failed to get change ID");

        String::from_utf8(output.stdout)
            .expect("Invalid UTF-8 in jj output")
            .trim()
            .to_string()
    }

    fn setup_test_env() -> (TempDir, crate::jj::Jujutsu, git2::Repository) {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let bare_path = temp_dir.path().join("bare");
        let repo_path = temp_dir.path().join("clone");

        let bare = create_test_git_repo(bare_path.clone());
        let repo = clone_repo(bare_path.clone(), repo_path.clone());

        let jj = crate::jj::Jujutsu::new(repo).expect("Failed to create JJ object in cloned repo");

        return (temp_dir, jj, bare);
    }

    #[tokio::test]
    async fn test_single_on_head() {
        let (_temp_dir, jj, bare) = setup_test_env();
        let config = create_test_config();
        let trunk_oid = jj
            .git_repo
            .refname_to_id("HEAD")
            .expect("Failed to revparse HEAD");
        let workdir = jj
            .git_repo
            .workdir()
            .expect("Got no workdir on git/jj repo");

        let rev = create_jujutsu_commit(workdir, "Test commit", "file 1");
        let change = jj
            .read_revision(&config, crate::jj::ChangeId::from_str(rev))
            .expect("Failed to read revision");
        let _ = handle_revs(
            &config,
            &jj,
            &super::StackedOptions {
                message: Some("".into()),
            },
            [(change.clone(), None)],
            trunk_oid,
        )
        .await
        .expect("Expected to get branch name");

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
        let (_temp_dir, jj, bare) = setup_test_env();
        let config = create_test_config();
        let trunk_oid = jj
            .git_repo
            .refname_to_id("HEAD")
            .expect("Failed to revparse HEAD");
        let workdir = jj
            .git_repo
            .workdir()
            .expect("Got no workdir on git/jj repo");

        let rev = create_jujutsu_commit(workdir, "Test commit", "file 1");
        let change = jj
            .read_revision(&config, crate::jj::ChangeId::from_str(rev))
            .expect("Failed to read revision");
        let _ = handle_revs(
            &config,
            &jj,
            &super::StackedOptions {
                message: Some("".into()),
            },
            [(change.clone(), None)],
            trunk_oid,
        )
        .await
        .expect("Expected to get branch name");

        let pr_branch = bare
            .find_branch("spr/test/test-commit", git2::BranchType::Local)
            .expect("Expected to find branch on bare upstream");
        let initial_pr_oid = pr_branch
            .get()
            .target()
            .expect("Failed to get oid from pr branch");

        amend_jujutsu_revision(workdir, "file 2");
        let _ = handle_revs(
            &config,
            &jj,
            &super::StackedOptions {
                message: Some("".into()),
            },
            [(
                change.clone(),
                Some(crate::github::PullRequest {
                    number: 1,
                    state: crate::github::PullRequestState::Open,
                    title: String::from(""),
                    body: None,
                    base_oid: git2::Oid::zero(),
                    sections: std::collections::BTreeMap::new(),
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
            trunk_oid,
        )
        .await
        .expect("Expected to get branch name");
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
        let (_temp_dir, jj, bare) = setup_test_env();
        let config = create_test_config();
        let trunk_oid = jj
            .git_repo
            .refname_to_id("HEAD")
            .expect("Failed to revparse HEAD");
        let workdir = jj
            .git_repo
            .workdir()
            .expect("Got no workdir on git/jj repo");

        let rev = create_jujutsu_commit(workdir, "Test commit", "file 1");
        let change = jj
            .read_revision(&config, crate::jj::ChangeId::from_str(rev))
            .expect("Failed to read revision");
        let _ = handle_revs(
            &config,
            &jj,
            &super::StackedOptions {
                message: Some("".into()),
            },
            [(change.clone(), None)],
            trunk_oid,
        )
        .await
        .expect("Expected to get branch name");

        let pr_branch = bare
            .find_branch("spr/test/test-commit", git2::BranchType::Local)
            .expect("Expected to find branch on bare upstream");
        let initial_pr_oid = pr_branch
            .get()
            .target()
            .expect("Failed to get oid from pr branch");

        let child_rev = create_jujutsu_commit(workdir, "Test other commit", "file other");
        let child_change = jj
            .read_revision(&config, crate::jj::ChangeId::from_str(child_rev))
            .expect("Failed to read child revision");
        let _ = handle_revs(
            &config,
            &jj,
            &super::StackedOptions {
                message: Some("".into()),
            },
            [
                (
                    change.clone(),
                    Some(crate::github::PullRequest {
                        number: 1,
                        state: crate::github::PullRequestState::Open,
                        title: String::from(""),
                        body: None,
                        base_oid: git2::Oid::zero(),
                        sections: std::collections::BTreeMap::new(),
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
                ),
                (child_change.clone(), None),
            ],
            trunk_oid,
        )
        .await
        .expect("Expected to get branch name");
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
        let (_temp_dir, jj, bare) = setup_test_env();
        let config = create_test_config();
        let trunk_oid = jj
            .git_repo
            .refname_to_id("HEAD")
            .expect("Failed to revparse HEAD");
        let workdir = jj
            .git_repo
            .workdir()
            .expect("Got no workdir on git/jj repo");

        let rev = create_jujutsu_commit(workdir, "Test commit", "file 1");
        let change = jj
            .read_revision(&config, crate::jj::ChangeId::from_str(rev))
            .expect("Failed to read revision");

        let child_rev = create_jujutsu_commit(workdir, "Test other commit", "file other");
        let child_change = jj
            .read_revision(&config, crate::jj::ChangeId::from_str(child_rev))
            .expect("Failed to read child revision");
        let _ = handle_revs(
            &config,
            &jj,
            &super::StackedOptions {
                message: Some("".into()),
            },
            [(change.clone(), None), (child_change.clone(), None)],
            trunk_oid,
        )
        .await
        .expect("Expected to get branch name");
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
        let (_temp_dir, jj, bare) = setup_test_env();
        let config = create_test_config();
        let trunk_oid = jj
            .git_repo
            .refname_to_id("HEAD")
            .expect("Failed to revparse HEAD");
        let workdir = jj
            .git_repo
            .workdir()
            .expect("Got no workdir on git/jj repo");

        let rev = create_jujutsu_commit(workdir, "Test commit", "file 1");
        let change = jj
            .read_revision(&config, crate::jj::ChangeId::from_str(rev))
            .expect("Failed to read revision");
        let _ = handle_revs(
            &config,
            &jj,
            &super::StackedOptions {
                message: Some("".into()),
            },
            [(change.clone(), None)],
            trunk_oid,
        )
        .await
        .expect("Expected to get branch name");

        let pr_branch = bare
            .find_branch("spr/test/test-commit", git2::BranchType::Local)
            .expect("Expected to find branch on bare upstream");
        let initial_pr_oid = pr_branch
            .get()
            .target()
            .expect("Failed to get oid from pr branch");

        jj.git_repo
            .set_head_detached(trunk_oid)
            .expect("Failed to checkout trunk");
        let _ = create_jujutsu_commit(workdir, "New head", "file 3");
        let updated_trunk_oid = jj
            .git_repo
            .refname_to_id("HEAD")
            .expect("Failed to revparse HEAD");
        jj.git_repo
            .find_remote("origin")
            .expect("Didn't find origin on repo")
            .push(&["HEAD:refs/heads/main"], None)
            .expect("Failed to push new main");

        let _ = handle_revs(
            &config,
            &jj,
            &super::StackedOptions {
                message: Some("".into()),
            },
            [(
                change.clone(),
                Some(crate::github::PullRequest {
                    number: 1,
                    state: crate::github::PullRequestState::Open,
                    title: String::from(""),
                    body: None,
                    base_oid: git2::Oid::zero(),
                    sections: std::collections::BTreeMap::new(),
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
            trunk_oid,
        )
        .await
        .expect("Expected to get branch name");
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
        let (_temp_dir, jj, bare) = setup_test_env();
        let config = create_test_config();
        let trunk_oid = jj
            .git_repo
            .refname_to_id("HEAD")
            .expect("Failed to revparse HEAD");
        let workdir = jj
            .git_repo
            .workdir()
            .expect("Got no workdir on git/jj repo");

        let rev = create_jujutsu_commit(workdir, "Test commit", "file 1");
        let change = jj
            .read_revision(&config, crate::jj::ChangeId::from_str(rev.clone()))
            .expect("Failed to read revision");
        let _ = handle_revs(
            &config,
            &jj,
            &super::StackedOptions {
                message: Some("".into()),
            },
            [(change.clone(), None)],
            trunk_oid,
        )
        .await
        .expect("Expected to get branch name");

        let pr_branch = bare
            .find_branch("spr/test/test-commit", git2::BranchType::Local)
            .expect("Expected to find branch on bare upstream");
        let initial_pr_oid = pr_branch
            .get()
            .target()
            .expect("Failed to get oid from pr branch");

        jj.git_repo
            .set_head_detached(trunk_oid)
            .expect("Failed to checkout trunk");
        let new_trunk = create_jujutsu_commit(workdir, "New head", "file 3");
        let updated_trunk_oid = jj
            .git_repo
            .refname_to_id("HEAD")
            .expect("Failed to revparse HEAD");
        jj.git_repo
            .find_remote("origin")
            .expect("Didn't find origin on repo")
            .push(&["HEAD:refs/heads/main"], None)
            .expect("Failed to push new main");

        rebase_jj_commit(
            workdir,
            crate::jj::ChangeId::from_str(rev),
            crate::jj::ChangeId::from_str(new_trunk),
        );

        let _ = handle_revs(
            &config,
            &jj,
            &super::StackedOptions {
                message: Some("".into()),
            },
            [(
                change.clone(),
                Some(crate::github::PullRequest {
                    number: 1,
                    state: crate::github::PullRequestState::Open,
                    title: String::from(""),
                    body: None,
                    base_oid: git2::Oid::zero(),
                    sections: std::collections::BTreeMap::new(),
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
            updated_trunk_oid,
        )
        .await
        .expect("Expected to get branch name");
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
        let (_temp_dir, jj, bare) = setup_test_env();
        let config = create_test_config();
        let trunk_oid = jj
            .git_repo
            .refname_to_id("HEAD")
            .expect("Failed to revparse HEAD");
        let workdir = jj
            .git_repo
            .workdir()
            .expect("Got no workdir on git/jj repo");

        let rev = create_jujutsu_commit(workdir, "Test commit", "file 1");
        let change = jj
            .read_revision(&config, crate::jj::ChangeId::from_str(rev.clone()))
            .expect("Failed to read revision");

        let child_rev = create_jujutsu_commit(workdir, "Test other commit", "file other");
        let child_change = jj
            .read_revision(&config, crate::jj::ChangeId::from_str(child_rev))
            .expect("Failed to read child revision");
        let _ = handle_revs(
            &config,
            &jj,
            &super::StackedOptions {
                message: Some("".into()),
            },
            [(change.clone(), None), (child_change.clone(), None)],
            trunk_oid,
        )
        .await
        .expect("Expected to get branch name");
        let child_pr_branch = bare
            .find_branch("spr/test/test-other-commit", git2::BranchType::Local)
            .expect("Expected to find other branch on bare upstream");
        let initial_child_pr_oid = child_pr_branch
            .get()
            .target()
            .expect("Failed to get other oid from pr branch");

        new_on_jj_commit(workdir, ChangeId::from_str(rev.clone()));
        amend_jujutsu_revision(workdir, "file 2");
        let _ = handle_revs(
            &config,
            &jj,
            &super::StackedOptions {
                message: Some("".into()),
            },
            [
                (
                    change.clone(),
                    Some(crate::github::PullRequest {
                        number: 1,
                        state: crate::github::PullRequestState::Open,
                        title: String::from(""),
                        body: None,
                        base_oid: git2::Oid::zero(),
                        sections: std::collections::BTreeMap::new(),
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
                ),
                (
                    child_change.clone(),
                    Some(crate::github::PullRequest {
                        number: 1,
                        state: crate::github::PullRequestState::Open,
                        title: String::from(""),
                        body: None,
                        base_oid: git2::Oid::zero(),
                        sections: std::collections::BTreeMap::new(),
                        base: crate::github::GitHubBranch::new_from_branch_name(
                            "spr/test/test-commit",
                            "origin",
                            "main",
                        ),
                        head_oid: git2::Oid::zero(),
                        head: crate::github::GitHubBranch::new_from_branch_name(
                            "spr/test/test-other-commit",
                            "origin",
                            "main",
                        ),
                        merge_commit: None,
                        reviewers: std::collections::HashMap::new(),
                        review_status: None,
                    }),
                ),
            ],
            trunk_oid,
        )
        .await
        .expect("Expected to get branch name");

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
        assert!(
            bare.merge_base(initial_child_pr_oid, child_pr_oid)
                .expect("Failed to get merge oid")
                == initial_child_pr_oid,
            "child PR branch was not based on initial child PR"
        );
    }

    #[tokio::test]
    async fn test_no_update_without_change() {
        let (_temp_dir, jj, bare) = setup_test_env();
        let config = create_test_config();
        let trunk_oid = jj
            .git_repo
            .refname_to_id("HEAD")
            .expect("Failed to revparse HEAD");
        let workdir = jj
            .git_repo
            .workdir()
            .expect("Got no workdir on git/jj repo");

        let rev = create_jujutsu_commit(workdir, "Test commit", "file 1");
        let change = jj
            .read_revision(&config, crate::jj::ChangeId::from_str(rev))
            .expect("Failed to read revision");
        let _ = handle_revs(
            &config,
            &jj,
            &super::StackedOptions {
                message: Some("".into()),
            },
            [(change.clone(), None)],
            trunk_oid,
        )
        .await
        .expect("Expected to get branch name");

        let pr_branch = bare
            .find_branch("spr/test/test-commit", git2::BranchType::Local)
            .expect("Expected to find branch on bare upstream");
        let initial_pr_oid = pr_branch
            .get()
            .target()
            .expect("Failed to get oid from pr branch");
        let _ = handle_revs(
            &config,
            &jj,
            &super::StackedOptions {
                message: Some("".into()),
            },
            [(
                change.clone(),
                Some(crate::github::PullRequest {
                    number: 1,
                    state: crate::github::PullRequestState::Open,
                    title: String::from(""),
                    body: None,
                    base_oid: git2::Oid::zero(),
                    sections: std::collections::BTreeMap::new(),
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
            trunk_oid,
        )
        .await
        .expect("Expected to get branch name");
        let pr_branch = bare
            .find_branch("spr/test/test-commit", git2::BranchType::Local)
            .expect("Expected to find branch on bare upstream");
        let pr_oid = pr_branch
            .get()
            .target()
            .expect("Failed to get oid from pr branch");
        assert!(pr_oid == initial_pr_oid, "PR was updated without changes");
    }
}
