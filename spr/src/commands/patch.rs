/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use crate::{
    error::{Error, Result},
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
    jj.new_revision(
        format!("{}@{}", config.master_ref.branch_name(), config.remote_name),
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

    use super::do_patch;
    use crate::{jj::ChangeId, message::MessageSection, testing};

    #[tokio::test]
    async fn test_single_on_head() {
        let pr_nr = 1;
        let (_temp_dir, jj, _) = testing::setup::repo_with_origin();
        let commit_oid = testing::git::add_commit_and_push_to_remote(&jj.git_repo, "spr/test/test-branch");
        let tree_oid = jj.get_tree_oid_for_commit(commit_oid).expect("Expected to get tree for commit");

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

        let rev = jj
            .read_revision(&testing::config::basic(), ChangeId::from_str("@".into()))
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
