/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use crate::{
    error::{Error, Result},
    message::{build_commit_message, MessageSection, MessageSectionsMap},
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

    let resolved = jj.resolve_reference(format!("{}/{}", config.remote_name, branch_name).as_str())?;
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
