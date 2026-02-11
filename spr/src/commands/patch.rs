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
    jj: &crate::jj::Jujutsu,
    config: &crate::config::Config,
    message: &mut MessageSectionsMap,
    branch_name: &str,
) -> Result<()> {
    jj.run_git_fetch()?;

    let resolved = jj.resolve_reference(
        format!("refs/remotes/{}/{}", config.remote_name, branch_name).as_str(),
    )?;
    message.insert(MessageSection::LastCommit, resolved.to_string());
    let message = build_commit_message(message);
    let branch = jj.git_repo.find_branch(
        format!("{}/{}", config.remote_name, config.master_ref.branch_name()).as_str(),
        git2::BranchType::Remote,
    )?;
    jj.new_revision(
        Some(RevSet::from_remote_branch(branch, config.remote_name.clone())?.unique()),
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

pub async fn patch(
    opts: PatchOptions,
    jj: &crate::jj::Jujutsu,
    gh: &mut crate::github::GitHub,
    config: &crate::config::Config,
) -> Result<()> {
    let mut pr = gh.clone().get_pull_request(opts.pull_request).await?;

    if !pr.base.is_master_branch() {
        return Err(Error::new(format!(
            "Specified PR {} is not based on the target branch. Adopting stacked PRs is not yet supported",
            pr.number
        )));
    }

    do_patch(jj, config, &mut pr.sections, pr.head.branch_name())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use git2::Time;

    use super::do_patch;
    use crate::{jj::RevSet, message::MessageSection, testing};

    #[tokio::test]
    async fn test_single_on_head() {
        let pr_nr = 1;
        let (_temp_dir, jj, _) = testing::setup::repo_with_origin();

        let file_path = jj
            .git_repo
            .workdir()
            .expect("Failed to extract workdir from JJ handle")
            .join("test.txt");
        fs::write(&file_path, "PR change").expect("Failed to write test file");

        let mut index = jj
            .git_repo
            .index()
            .expect("Couldn't get index from git repo");
        index
            .add_path(std::path::Path::new("test.txt"))
            .expect("Failed to add test file to index");
        let sig = git2::Signature::new("User", "user@example.com", &Time::new(0, 0))
            .expect("Failed to build commit signature");
        let tree_oid = index.write_tree().expect("Failed to write tree to disk");
        let tree = jj
            .git_repo
            .find_tree(tree_oid)
            .expect("Failed to find tree from OID");
        let trunk = jj
            .git_repo
            .find_commit(
                jj.git_repo
                    .revparse_single("HEAD")
                    .expect("Failed to parse revparse HEAD")
                    .id(),
            )
            .expect("Failed to find commit for HEAD");
        let commit_oid = jj
            .git_repo
            .commit(None, &sig, &sig, "Test commit", &tree, &[&trunk])
            .expect("Failed to commit to repo");

        let mut remote = jj
            .git_repo
            .find_remote("origin")
            .expect("Expected to find origin as remote");

        remote
            .push(
                &[format!("{}:refs/heads/spr/test/test-branch", commit_oid)],
                None,
            )
            .expect("Failed to push");

        do_patch(
            &jj,
            &testing::config::basic(),
            &mut BTreeMap::from([(
                MessageSection::PullRequest,
                testing::config::basic().pull_request_url(pr_nr),
            )]),
            "spr/test/test-branch",
        )
        .expect("Do not expect do_patch to fail.");

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
    }
}
