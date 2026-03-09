use crate::{
    error::{Error, Result, ResultExt},
    jj::RevSet,
    message::{MessageSection, build_github_body},
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

    #[clap(long, group = "revs")]
    existing: bool,

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

    pub fn with_all(mut self, val: bool) -> Self {
        self.all = val;
        self
    }

    pub fn with_existing(mut self, val: bool) -> Self {
        self.existing = val;
        self
    }
}

enum WorkEvent<'a> {
    Rebased(&'a crate::config::Config),
    Updated(&'a crate::config::Config),
    PRCreated(&'a crate::config::Config),
    ReviewRequested(&'a crate::config::Config),
    Assigned(&'a crate::config::Config),
}

type WorkLog<'a> = Vec<WorkEvent<'a>>;
impl<'a> std::fmt::Display for WorkEvent<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (name, icon) = match self {
            WorkEvent::Rebased(c) => ("Rebased", c.icons.refresh.as_ref()),
            WorkEvent::Updated(c) => ("Updated", c.icons.working.as_ref()),
            WorkEvent::PRCreated(c) => ("Create Pull Request", c.icons.sparkle.as_ref()),
            WorkEvent::ReviewRequested(c) => ("Requested Reviews", c.icons.eyes.as_ref()),
            WorkEvent::Assigned(c) => ("Assigned users", c.icons.ok.as_ref()),
        };

        f.write_str(name)?;
        f.write_str(icon)?;
        Ok(())
    }
}

struct WorkSet<'a, PR> {
    revision: crate::jj::Revision,
    progress_bar: indicatif::ProgressBar,
    pull_request: PR,
    work_done: WorkLog<'a>,
}

impl<'a, PR> WorkSet<'a, PR> {
    fn map<F, O>(self, fun: F) -> WorkSet<'a, O>
    where
        F: FnOnce(PR) -> O,
    {
        WorkSet {
            revision: self.revision,
            progress_bar: self.progress_bar,
            pull_request: fun(self.pull_request),
            work_done: self.work_done,
        }
    }

    fn format_worklog(&self, config: &crate::config::Config) -> String {
        if self.work_done.is_empty() {
            return format!("Nothing to be done {}.", config.icons.sleeping.as_ref());
        }

        let stringified: Vec<_> = self.work_done.iter().map(|e| format!("{}", e)).collect();
        stringified.join("&")
    }
}

async fn do_push_single<'a, PR, H: AsRef<str>>(
    jj: &mut crate::jj::Jujutsu,
    config: &'a crate::config::Config,
    opts: &PushOptions,
    base_ref: &crate::jj::RevSet,
    head_branch: H,
    ws: &mut WorkSet<'a, PR>,
) -> Result<()> {
    ws.progress_bar.set_message("Building new commit");
    let base_oid = jj
        .resolve_revision_to_commit_id(base_ref.as_ref())
        .context(String::from("Resolve base_ref to OID"))?;
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
        .resolve_revision_to_commit_id(ws.revision.id.as_ref())
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
        ws.progress_bar.set_message("Git is already up to date");
        return Ok(());
    }

    if !opts.force
        && let Some(old) = ws.revision.message.get(&MessageSection::LastCommit)
        && git2::Oid::from_str(old)? != head_oid
    {
        return Err(crate::error::Error::new(format!(
            "Cannot update {}. It has an unexpected upstream.",
            config.pull_request_url(ws.revision.pull_request_number.unwrap_or(0))
        )));
    }

    let real_change = parents.len() == 1 || {
        let pre_rev = {
            let pre_commit = jj
                .create_derived_commit(
                    target_oid,
                    "Staging commit for change detection via jj-spr",
                    parents,
                )
                .context(String::from("Creating pre-commit"))?;

            crate::jj::RevSet::from(&pre_commit)
        };

        // We need to update so JJ learns about the commit we just created.
        jj.update()
            .context(String::from("Update JJ to learn about pre-commit"))?;
        let changed = !jj
            .is_empty(
                &jj.revset_to_change_id(&pre_rev)
                    .context(String::from("Resolve commit to ChangeID for pre-commit"))?,
            )
            .context(String::from("Check if pre-commit is empty"))?;
        // Since this is a local-only change, we have to abandon it.
        // Otherwise it'll show up in `jj log` for the user afterwards.
        jj.abandon(&pre_rev).context(String::from(
            "Failed to abandon temporary change for change detection",
        ))?;
        changed
    };

    let message = if head_oid == base_oid
        && let Some(title) = ws.revision.message.get(&MessageSection::Title)
    {
        format!("{}\n\n{}", title, build_github_body(&ws.revision.message))
    } else if !real_change {
        String::from("Rebasing with jj-spr")
    } else if let Some(ref msg) = opts.message {
        msg.clone()
    } else {
        dialoguer::Input::<String>::new()
            .with_prompt(format!("Message for '{}'", ws.revision.title))
            .with_initial_text("")
            .allow_empty(true)
            .interact_text()?
    };

    // Create the new commit
    let change = jj
        .create_derived_commit(
            target_oid,
            &format!("{}\n\nCreated using jj-spr", message),
            parents,
        )
        .map_err(|mut err| {
            err.push("derive commit".into());
            err
        })?;
    let pr_commit = jj.resolve_revision_to_commit_id(RevSet::from(&change).as_ref())?;

    ws.progress_bar.set_message("Pushing to GitHub");
    ws.revision
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
    // Looks like the above activity makes the change immutable.
    // So we don't have to (and cannot) abandon it here.
    jj.update()?;

    ws.progress_bar.set_message(if parents.len() == 1 {
        format!("Updated")
    } else {
        format!("Rebased")
    });
    if real_change {
        ws.work_done.push(WorkEvent::Updated(config));
    }
    if parents.len() > 1 {
        ws.work_done.push(WorkEvent::Rebased(config));
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct BranchAction<PR> {
    head_branch: String,
    base_branch: String,
    old_pr: Option<PR>,
}

async fn do_push<'a, I, PR>(
    config: &'a crate::config::Config,
    jj: &mut crate::jj::Jujutsu,
    opts: &PushOptions,
    work: I,
    trunk_head: &crate::jj::RevSet,
) -> Result<Vec<WorkSet<'a, BranchAction<PR>>>>
where
    PR: crate::github::GHPullRequest,
    I: IntoIterator<Item = WorkSet<'a, Option<PR>>>,
{
    // ChangeID, head branch, base branch, existing pr
    let mut seen: Vec<WorkSet<BranchAction<PR>>> = Vec::new();
    for mut ws in work.into_iter() {
        ws.progress_bar.set_message("Finding base commits");

        let head_ref: String = if let Some(ref pr) = ws.pull_request {
            pr.head_branch_name().into()
        } else if let Some(bookmark) = ws.revision.bookmarks.first() {
            bookmark.clone()
        } else {
            // We have to come up with something...
            let title = ws
                .revision
                .message
                .get(&MessageSection::Title)
                .map(|t| &t[..])
                .unwrap_or("");
            config.get_new_branch_name(&jj.get_all_ref_names()?, title)
        };
        let base_branch = if let Some(ba) = seen
            .iter()
            .find(|ba| ba.revision.id == ws.revision.parent_ids[0])
        {
            Some(ba.pull_request.head_branch.clone())
        } else {
            None
        };

        let base_ref = if let Some(base_branch) = base_branch.as_ref()
            && *base_branch != config.master_ref
        {
            let branch = jj.git_repo.find_branch(
                format!("{}/{}", config.remote_name, base_branch).as_str(),
                git2::BranchType::Remote,
            )?;
            RevSet::from_remote_branch(&branch, &config.remote_name)?
        } else {
            trunk_head.fork_point(&crate::jj::RevSet::from(&ws.revision.id))
        };

        do_push_single(jj, config, opts, &base_ref, &head_ref, &mut ws)
            .await
            .map_err(|mut err| {
                err.push("do_push_single".into());
                err
            })?;

        let owned_base = base_branch.unwrap_or(config.master_ref.clone()).into();
        seen.push(ws.map(|pr| BranchAction {
            head_branch: head_ref,
            base_branch: owned_base,
            old_pr: pr,
        }));
    }
    Ok(seen)
}

fn prepare_revision_comment(
    tree: &crate::tree::Tree<crate::jj::Revision>,
    config: &crate::config::Config,
) -> Vec<String> {
    let mut lines = Vec::new();
    // The node itself doesn't need indents.
    // It is indented by the parent if necessary
    lines.push(format!(
        "• [{}]({})",
        tree.get().title,
        if let Some(num) = tree.get().pull_request_number {
            config.pull_request_url(num)
        } else {
            format!(
                "Revision {:?} doesn't have a pull request yet. This is a bug.",
                tree.get().id
            )
        }
    ));

    let children = tree.get_children();
    match children.as_slice() {
        [] => {}
        [next] => {
            lines.extend(prepare_revision_comment(next, config));
        }
        // We have more than one child branch.
        // We need to actually build an unicode-art tree
        children => {
            let mut child_lines = Vec::new();
            for child in children {
                let indent = [config.drawing.space.clone()]
                    .into_iter()
                    .cycle()
                    .take(child.width() * 2 - 1)
                    .reduce(|l, r| format!("{l}{r}"))
                    .unwrap_or(config.drawing.space.clone());
                let new_lines = prepare_revision_comment(child, config);
                let old_lines = child_lines.into_iter().enumerate().map(|(i, l)| {
                    format!(
                        "{}{}{}",
                        if i == 0 {
                            &config.drawing.fork
                        } else {
                            &config.drawing.cont
                        },
                        indent,
                        l
                    )
                });
                child_lines = old_lines.collect();
                child_lines.extend(new_lines);
            }

            lines.extend(child_lines);
        }
    }

    return lines;
}

fn finalize_revision_comment(
    revision: &crate::jj::Revision,
    config: &crate::config::Config,
    prepared: &Vec<String>,
) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "This PR is part of a {} changes series",
        prepared.len()
    ));

    lines.extend_from_slice(prepared.as_slice());
    if let Some(number) = revision.pull_request_number {
        let pattern = format!("[{}]({})", revision.title, config.pull_request_url(number));
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
    let multi = indicatif::MultiProgress::new();
    let setup = multi.add(
        indicatif::ProgressBar::new(100).with_style(
            indicatif::ProgressStyle::default_bar()
                .template("{msg}")
                .expect("Indicatif template shouldn't fail"),
        ),
    );
    let heads = opts.revset.as_ref().map(|s| RevSet::from_arg(s)).unwrap_or(
        match (opts.all, opts.existing) {
            (true, false) => RevSet::mutable().heads(),
            (false, true) => {
                RevSet::mutable().and(&RevSet::description("substring:\"Last Commit:\""))
            }
            (false, false) => RevSet::current(),
            _ => unreachable!(),
        },
    );
    setup.set_message(format!("Finding revisions for '{}'", heads.as_ref()));

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

    setup.set_message(format!("Validating revisions are ready"));
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
    if revisions.is_empty() {
        setup.finish_and_clear();
        crate::output::output(
            &config.icons.wave,
            "No commits found - nothing to do. Good bye!",
        )?;
        return Ok(());
    };

    setup.set_message("Finding pull requests for revisions");
    let workset: Vec<WorkSet<()>> = revisions
        .into_iter()
        .map(|revision| {
            let progress_bar = multi.add(
                indicatif::ProgressBar::new(100).with_style(
                    indicatif::ProgressStyle::default_bar()
                        .template("{prefix}: {msg}")
                        .expect("Indicatif template shouldn't fail"),
                ),
            );
            progress_bar.set_prefix(format!("{}", revision.title));
            progress_bar.set_message("Figuring it out");
            WorkSet {
                revision,
                progress_bar,
                pull_request: (),
                work_done: Vec::new(),
            }
        })
        .collect();

    {
        let mut seen = Vec::new();
        for pr_num in workset
            .iter()
            .filter_map(|r| r.revision.pull_request_number)
        {
            if seen.contains(&pr_num) {
                return Err(crate::error::Error::new(
                    format!("Found PR {pr_num} in more than one revision").as_str(),
                ));
            } else {
                seen.push(pr_num);
            }
        }
    }

    let pull_requests = gh
        .pull_requests(workset.iter().map(|ws| ws.revision.pull_request_number))
        .await?;

    let trunk = {
        let branch = jj.git_repo.find_branch(
            format!("{}/{}", config.remote_name, config.master_ref).as_ref(),
            git2::BranchType::Remote,
        )?;
        crate::jj::RevSet::from_remote_branch(&branch, &config.remote_name)?
    };
    let work = zip(workset, pull_requests).into_iter().map(|(ws, pr)| {
        if let Some(ref pr) = pr {
            ws.progress_bar.set_prefix(format!(
                "{} ({})",
                ws.revision.title,
                config.pull_request_url(pr.pr_number())
            ));
        }
        ws.progress_bar.set_message("Figured out PRs");
        ws.map(|_| pr)
    });

    setup.set_message("Pushing revisions");
    let mut actions = do_push(config, jj, &opts, work, &trunk).await?;
    setup.set_message("Setting up PRs");
    for workset in actions.iter_mut().into_iter() {
        // We don't know what to do with these yet...
        if let Some(ref pr) = workset.pull_request.old_pr {
            if workset.pull_request.base_branch != pr.base_branch_name() {
                workset.progress_bar.set_message("Rebasing on GitHub");
                gh.rebase_pr(pr.pr_number(), &workset.pull_request.base_branch)
                    .await?;
            }
            workset
                .progress_bar
                .set_message("Updating revision description");
            // This will at least write the current commit message.
            jj.update_revision_message(&workset.revision)?;
            workset.progress_bar.set_message("Handled post actions");
            continue;
        }

        workset.progress_bar.set_message("Create Pull Request");
        let title = workset
            .revision
            .message
            .get(&MessageSection::Title)
            .map_or("Missing Title", |s| s.as_str());
        let body = workset
            .revision
            .message
            .get(&MessageSection::Summary)
            .map_or("", |s| s.as_str());
        let pr = gh
            .new_pull_request(
                title,
                body,
                &workset.pull_request.base_branch,
                &workset.pull_request.head_branch,
                false,
            )
            .await?;

        workset.work_done.push(WorkEvent::PRCreated(config));
        workset.progress_bar.set_prefix(format!(
            "{} ({})",
            workset.revision.title,
            config.pull_request_url(pr.pr_number())
        ));
        if let Some(reviewers) = workset.revision.message.get(&MessageSection::Reviewers) {
            workset.progress_bar.set_message("Requesting reviewers");
            gh.add_reviewers(&pr, reviewers.split(",").map(|s| s.trim()))
                .await?;
            workset.work_done.push(WorkEvent::ReviewRequested(config));
        }
        if let Some(assignees) = workset.revision.message.get(&MessageSection::Assignees) {
            workset.progress_bar.set_message("Requesting assignees");
            gh.add_assignees(&pr, assignees.split(",").map(|s| s.trim()))
                .await?;
            workset.work_done.push(WorkEvent::Assigned(config));
        }

        let pull_request_url = config.pull_request_url(pr.pr_number());
        crate::output::output(
            &config.icons.sparkle,
            &format!(
                "Created new Pull Request #{}: {}",
                pr.pr_number(),
                &pull_request_url,
            ),
        )?;

        workset
            .progress_bar
            .set_message("Updating revisions description");
        workset
            .revision
            .message
            .insert(MessageSection::PullRequest, pull_request_url);
        workset.revision.pull_request_number = Some(pr.pr_number());

        jj.update_revision_message(&workset.revision)?;
        workset.progress_bar.set_message("Created PR");
    }
    for ws in actions.iter() {
        ws.progress_bar
            .finish_with_message(ws.format_worklog(config));
    }
    setup.set_message("Figuring out PR tree");

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

    setup.set_message("Updating tree overviews");
    for tree in forest.into_trees() {
        let prepared = prepare_revision_comment(&tree, config);
        for rev in tree.into_iter() {
            match rev.pull_request_number {
                Some(number) => {
                    let content = finalize_revision_comment(&rev, config, &prepared);
                    gh.update_pr_comment(number, &content).await?;
                }
                None => {
                    crate::output::output(
                        &config.icons.error,
                        format!(
                            "Change {:?} has no PR attached. This is a bug at this point",
                            rev.id
                        ),
                    )?;
                }
            }
        }
    }
    setup.finish_and_clear();

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

    mod rebase_prs {
        use crate::testing;

        #[tokio::test]
        async fn to_main() {
            let (_temp_dir, mut jj, _) = testing::setup::repo_with_origin();
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
            gh.pull_requests
                .get_mut(&1)
                .expect("Should have created PR 1")
                .base = String::from("spr/test/my-commit");
            super::super::push(
                &mut jj,
                &mut gh,
                &testing::config::basic(),
                super::super::PushOptions::default().with_message(Some("message")),
            )
            .await
            .expect("stacked shouldn't fail");

            assert_eq!(
                gh.pull_requests.get(&1).expect("Should still have PR").base,
                testing::config::basic().master_ref,
                "PR base was not updated"
            );
        }

        #[tokio::test]
        async fn to_base_main() {
            let (_temp_dir, mut jj, _) = testing::setup::repo_with_origin();
            let trunk_rev = crate::jj::RevSet::from(
                &jj.revset_to_change_id(&crate::jj::RevSet::current().parent())
                    .expect("Should have changeID for current"),
            );
            let first_id = super::create_jujutsu_commit(&mut jj, "Test commit", "file 1");
            jj.new_revision(Some(trunk_rev), None as Option<&str>, false)
                .expect("Should be able to new onto something else");

            let second_id = super::create_jujutsu_commit_in_file(
                &mut jj,
                "Other commit",
                "Other content",
                "other file",
            );
            let mut gh = crate::github::fakes::GitHub {
                pull_requests: std::collections::BTreeMap::new(),
            };
            super::super::push(
                &mut jj,
                &mut gh,
                &testing::config::basic(),
                super::super::PushOptions::default()
                    .with_all(true)
                    .with_message(Some("message")),
            )
            .await
            .expect("stacked shouldn't fail");
            jj.rebase_branch(&crate::jj::RevSet::from(&second_id), first_id)
                .expect("Should be able to rebase change");
            super::super::push(
                &mut jj,
                &mut gh,
                &testing::config::basic(),
                super::super::PushOptions::default().with_message(Some("message")),
            )
            .await
            .expect("stacked shouldn't fail");

            assert_ne!(
                gh.pull_requests.get(&2).expect("Should still have PR").base,
                testing::config::basic().master_ref,
                "PR base was not updated"
            );
        }
    }

    mod revset {
        use crate::testing;

        #[tokio::test]
        async fn existing() {
            let (_temp_dir, mut jj, bare) = testing::setup::repo_with_origin();
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

            let pr_branch = bare
                .find_branch("spr/test/test-commit", git2::BranchType::Local)
                .expect("Expected to find branch on bare upstream");
            let initial_pr_oid = pr_branch
                .get()
                .target()
                .expect("Failed to get oid from pr branch");

            let _ = super::create_jujutsu_commit(&mut jj, "Test other commit", "file other");
            super::super::push(
                &mut jj,
                &mut gh,
                &testing::config::basic(),
                super::super::PushOptions::default()
                    .with_existing(true)
                    .with_message(Some("message")),
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

            let _ = bare
                .find_branch("spr/test/test-other-commit", git2::BranchType::Local)
                .map(|_| ())
                .expect_err("there shouldn't be abrnach for the second commit");
            assert_eq!(
                gh.pull_requests.len(),
                1,
                "There should be exactly one PR created from the initial push"
            );
        }
    }

    mod independent_heads {
        use crate::testing;

        #[tokio::test]
        async fn same_base() {
            let (_temp_dir, mut jj, _) = testing::setup::repo_with_origin();
            let base_change = jj
                .revset_to_change_id(&crate::jj::RevSet::current().parent())
                .expect("Should be able to find the base commit");

            let _ = super::create_jujutsu_commit(&mut jj, "Test commit", "file 1");
            let left_id = super::create_jujutsu_commit(&mut jj, "Other commit", "file 2");
            jj.new_revision(
                Some(crate::jj::RevSet::from(&base_change)),
                None as Option<&str>,
                false,
            )
            .expect("Couldn't create new revision on base");
            let right_id = super::create_jujutsu_commit(&mut jj, "More commit", "file 3");

            let mut gh = crate::github::fakes::GitHub {
                pull_requests: std::collections::BTreeMap::new(),
            };
            super::super::push(
                &mut jj,
                &mut gh,
                &testing::config::basic(),
                super::super::PushOptions::default()
                    .with_message(Some("message"))
                    .with_revset(Some(
                        crate::jj::RevSet::from(&left_id)
                            .or(&crate::jj::RevSet::from(&right_id))
                            .as_ref(),
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

        #[tokio::test]
        async fn different_bases() {
            let (_temp_dir, mut jj, _) = testing::setup::repo_with_origin();
            let base_change = jj
                .revset_to_change_id(&crate::jj::RevSet::current().parent())
                .expect("Should be able to find the base commit");
            let second_base = testing::git::add_commit_on_and_push_to_remote(
                &jj.git_repo,
                "main",
                [jj.resolve_revision_to_commit_id(base_change.as_ref())
                    .expect("Should be able to find a commit for base change")],
            );
            jj.run_git_fetch().expect("Should be able to run git fetch");
            let second_change = jj
                .revset_to_change_id(&crate::jj::RevSet::from_arg(second_base.to_string()))
                .expect("Expecting to find a change for the second base change");

            jj.new_revision(
                Some(crate::jj::RevSet::from(&base_change)),
                None as Option<&str>,
                false,
            )
            .expect("Couldn't create new revision on base");
            let _ = super::create_jujutsu_commit(&mut jj, "Test commit", "file 1");
            let left_id = super::create_jujutsu_commit(&mut jj, "Other commit", "file 2");
            jj.new_revision(
                Some(crate::jj::RevSet::from(&second_change)),
                None as Option<&str>,
                false,
            )
            .expect("Couldn't create new revision on base");
            let right_id = super::create_jujutsu_commit(&mut jj, "More commit", "file 3");

            let mut gh = crate::github::fakes::GitHub {
                pull_requests: std::collections::BTreeMap::new(),
            };
            super::super::push(
                &mut jj,
                &mut gh,
                &testing::config::basic(),
                super::super::PushOptions::default()
                    .with_message(Some("message"))
                    .with_revset(Some(
                        crate::jj::RevSet::from(&left_id)
                            .or(&crate::jj::RevSet::from(&right_id))
                            .as_ref(),
                    )),
            )
            .await
            .expect("stacked shouldn't fail");

            let left_revision = jj
                .read_revision(&testing::config::basic(), left_id.clone())
                .expect("Couldn't read left revision");
            let right_revision = jj
                .read_revision(&testing::config::basic(), right_id.clone())
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

            let left_pushed_id = {
                let branch = jj
                    .git_repo
                    .find_branch("origin/spr/test/other-commit", git2::BranchType::Remote)
                    .expect("Should be able to find a branch for the left side");

                jj.revset_to_change_id(
                    &crate::jj::RevSet::from_remote_branch(&branch, "origin")
                        .expect("Should be able to build a revset for the left branch"),
                )
                .expect("Should be able to find the ~hangeId")
            };
            let right_pushed_id = {
                let branch = jj
                    .git_repo
                    .find_branch("origin/spr/test/more-commit", git2::BranchType::Remote)
                    .expect("Should be able to find a branch for the right side");

                jj.revset_to_change_id(
                    &crate::jj::RevSet::from_remote_branch(&branch, "origin")
                        .expect("Should be able to build a revset for the right branch"),
                )
                .expect("Should be able to find the ~hangeId")
            };

            assert_eq!(
                base_change,
                jj.revset_to_change_id(
                    &crate::jj::RevSet::from(&second_change)
                        .fork_point(&crate::jj::RevSet::from(&left_pushed_id))
                )
                .expect("Should be able to find a fork point for left branch"),
                "The left side was based on too new a base"
            );
            assert_eq!(
                second_change,
                jj.revset_to_change_id(
                    &crate::jj::RevSet::from(&second_change)
                        .fork_point(&crate::jj::RevSet::from(&right_pushed_id))
                )
                .expect("Should be able to find a fork point for right branch"),
                "The right side was based on the older base"
            );
        }
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

        #[tokio::test]
        async fn multi_pr_reference() {
            let (_temp_dir, mut jj, _) = testing::setup::repo_with_origin();

            jj.new_revision(
                Some(RevSet::root()),
                Some(format!(
                    "Left\nPull Request: {}",
                    testing::config::basic().pull_request_url(1)
                )),
                false,
            )
            .expect("Failed to create left revision");
            let left = jj
                .revset_to_change_id(&RevSet::current())
                .expect("Failed to resolve left change id");
            jj.new_revision(
                Some(RevSet::root()),
                Some(format!(
                    "Right\nPull Request: {}",
                    testing::config::basic().pull_request_url(1)
                )),
                false,
            )
            .expect("Failed to create left revision");
            let right = jj
                .revset_to_change_id(&RevSet::current())
                .expect("Failed to resolve left change id");

            let mut gh = crate::github::fakes::GitHub {
                pull_requests: std::collections::BTreeMap::new(),
            };
            super::super::push(
                &mut jj,
                &mut gh,
                &testing::config::basic(),
                super::super::PushOptions::default().with_message(Some("message")).with_revset(Some(crate::jj::RevSet::from(&left).or(&crate::jj::RevSet::from(&right)).as_ref())),
            )
            .await
            .expect_err("Stacked should refuse to handle revset with multiple revisions pointing at same PR");
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
        use crate::testing;

        #[test]
        fn single() {
            let lines = super::super::prepare_revision_comment(
                &crate::tree::Tree::new(crate::jj::Revision {
                    id: crate::jj::ChangeId::from("change"),
                    parent_ids: Vec::new(),
                    pull_request_number: Some(1),
                    title: String::from("My Title"),
                    message: std::collections::BTreeMap::new(),
                    bookmarks: Vec::new(),
                }),
                &testing::config::basic(),
            );
            let str_lines: Vec<_> = lines.iter().map(|s| s.as_str()).collect();

            assert_eq!(
                str_lines.as_slice(),
                &["• [My Title](https://github.com/test_owner/test_repo/pull/1)"],
                "Lines didn't match expectation: {str_lines:?}"
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
            let lines = super::super::prepare_revision_comment(&tree, &testing::config::basic());
            let str_lines: Vec<_> = lines.iter().map(|s| s.as_str()).collect();

            assert_eq!(
                str_lines.as_slice(),
                &[
                    "• [My Title](https://github.com/test_owner/test_repo/pull/1)",
                    "• [My Other Title](https://github.com/test_owner/test_repo/pull/2)"
                ],
                "Lines didn't match expectation {str_lines:?}"
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
            let config = testing::config::basic();
            let lines = super::super::prepare_revision_comment(&tree, &config);
            let str_lines: Vec<_> = lines.iter().map(|s| s.as_str()).collect();

            assert_eq!(
                str_lines.as_slice(),
                &[
                    "• [My Title](https://github.com/test_owner/test_repo/pull/1)",
                    format!(
                        "{}{}{}",
                        config.drawing.fork,
                        config.drawing.space,
                        "• [My Other Title](https://github.com/test_owner/test_repo/pull/2)"
                    )
                    .as_ref(),
                    "• [My Third Title](https://github.com/test_owner/test_repo/pull/3)"
                ],
                "Lines didn't match: {str_lines:?}",
            );
        }

        #[test]
        fn with_cont() {
            let mut tree = crate::tree::Tree::new(crate::jj::Revision {
                id: crate::jj::ChangeId::from("change"),
                parent_ids: Vec::new(),
                pull_request_number: Some(1),
                title: String::from("My Title"),
                message: std::collections::BTreeMap::new(),
                bookmarks: Vec::new(),
            });
            let mut child = crate::tree::Tree::new(crate::jj::Revision {
                id: crate::jj::ChangeId::from("change"),
                parent_ids: Vec::new(),
                pull_request_number: Some(3),
                title: String::from("My Third Title"),
                message: std::collections::BTreeMap::new(),
                bookmarks: Vec::new(),
            });
            child.add_child_value(crate::jj::Revision {
                id: crate::jj::ChangeId::from("change"),
                parent_ids: Vec::new(),
                pull_request_number: Some(4),
                title: String::from("My Fourth Title"),
                message: std::collections::BTreeMap::new(),
                bookmarks: Vec::new(),
            });
            tree.add_child(child);
            tree.add_child_value(crate::jj::Revision {
                id: crate::jj::ChangeId::from("change"),
                parent_ids: Vec::new(),
                pull_request_number: Some(2),
                title: String::from("My Other Title"),
                message: std::collections::BTreeMap::new(),
                bookmarks: Vec::new(),
            });
            let config = testing::config::basic();
            let lines = super::super::prepare_revision_comment(&tree, &config);
            let str_lines: Vec<_> = lines.iter().map(|s| s.as_str()).collect();

            assert_eq!(
                str_lines.as_slice(),
                &[
                    "• [My Title](https://github.com/test_owner/test_repo/pull/1)",
                    format!(
                        "{}{}{}",
                        config.drawing.fork,
                        config.drawing.space,
                        "• [My Third Title](https://github.com/test_owner/test_repo/pull/3)"
                    )
                    .as_ref(),
                    format!(
                        "{}{}{}",
                        config.drawing.cont,
                        config.drawing.space,
                        "• [My Fourth Title](https://github.com/test_owner/test_repo/pull/4)"
                    )
                    .as_ref(),
                    "• [My Other Title](https://github.com/test_owner/test_repo/pull/2)"
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

    #[tokio::test]
    async fn test_split_and_reorder() {
        let (_temp_dir, mut jj, bare) = testing::setup::repo_with_origin();
        let repo_path = jj
            .git_repo
            .workdir()
            .expect("Failed to get workdir")
            .to_path_buf();

        // 1. Create a change with 2 affected files
        fs::write(repo_path.join("file1.txt"), "content 1").expect("Failed to write file1");
        fs::write(repo_path.join("file2.txt"), "content 2").expect("Failed to write file2");

        // Commit it
        let _ = std::process::Command::new("jj")
            .args(["commit", "-m", "Original commit"])
            .current_dir(&repo_path)
            .output()
            .expect("Failed to commit");

        let mut gh = crate::github::fakes::GitHub {
            pull_requests: std::collections::BTreeMap::new(),
        };

        // Push it (creates a PR)
        super::push(
            &mut jj,
            &mut gh,
            &testing::config::basic(),
            super::PushOptions::default().with_message(Some("message")),
        )
        .await
        .expect("Initial push failed");

        assert_eq!(gh.pull_requests.len(), 1);
        let original_pr = gh.pull_requests.get(&1).expect("PR 1 should exist");
        assert_eq!(original_pr.title, "Original commit");

        // 2. Split the commit into two.
        // We'll simulate `jj split` by using `jj new`, `jj restore`, and `jj squash`.
        // Current state: trunk -> B (file1, file2, PR 1) -> @ (empty)

        // We want: trunk -> B1 (file1) -> B2 (file2, PR 1)

        // Get the change ID of the commit with files
        let commit_b_change_id = jj
            .revset_to_change_id(&RevSet::current().parent())
            .expect("Failed to get parent change id");

        // Create B1
        let _ = std::process::Command::new("jj")
            .args(["new", "main@origin", "-m", "Parent commit"])
            .current_dir(&repo_path)
            .output()
            .expect("Failed to create B1");

        let _ = std::process::Command::new("jj")
            .args([
                "restore",
                "--from",
                commit_b_change_id.as_ref(),
                "file1.txt",
            ])
            .current_dir(&repo_path)
            .output()
            .expect("Failed to restore file1.txt");

        // Now @ is B1.
        let b1_change_id = jj
            .revset_to_change_id(&RevSet::current())
            .expect("Failed to get B1 change id");

        // Create B2 on top of B1
        let _ = std::process::Command::new("jj")
            .args(["new", b1_change_id.as_ref(), "-m", "Original commit"])
            .current_dir(&repo_path)
            .output()
            .expect("Failed to create B2");

        let _ = std::process::Command::new("jj")
            .args([
                "restore",
                "--from",
                commit_b_change_id.as_ref(),
                "file2.txt",
            ])
            .current_dir(&repo_path)
            .output()
            .expect("Failed to restore file2.txt");

        // Add the PR line to B2 so it "preserves its PR"
        let b2_message =
            format!("Child commit\n\nPull Request: https://github.com/test_owner/test_repo/pull/1");
        let _ = std::process::Command::new("jj")
            .args(["describe", "-m", &b2_message])
            .current_dir(&repo_path)
            .output()
            .expect("Failed to describe B2");

        // Abandon the original commit B
        let _ = std::process::Command::new("jj")
            .args(["abandon", commit_b_change_id.as_ref()])
            .current_dir(&repo_path)
            .output()
            .expect("Failed to abandon original commit");

        // 3. Clean up the parent commit (B1) to no longer track the PR.
        // It already doesn't have the PR line because we gave it a new message.

        // 4. Push again with reordering
        super::push(
            &mut jj,
            &mut gh,
            &testing::config::basic(),
            super::PushOptions::default().with_message(Some("message")),
        )
        .await
        .expect("Second push failed");

        // 5. Assertions:
        // - parent got a new PR (PR 2)
        // - child preserved its PR (PR 1)
        // - child PR got rebased onto the new parent (PR 2's branch)

        assert_eq!(gh.pull_requests.len(), 2, "Should have 2 PRs now");

        let pr1 = gh.pull_requests.get(&1).expect("PR 1 should still exist");
        let pr2 = gh
            .pull_requests
            .get(&2)
            .expect("PR 2 should have been created");

        assert_eq!(pr2.title, "Parent commit");
        // Parent PR should be based on main
        assert_eq!(pr2.base, "main", "Parent PR base should be main");
        // PR 1 currently keeps its original title on GitHub because jj-spr doesn't update titles/bodies for existing PRs yet
        assert_eq!(pr1.title, "Original commit");

        // Assert that pr1 (child) now has pr2's branch as its base
        assert_eq!(pr1.base, pr2.head, "Child PR base should be parent PR head");

        // Also verify the branches on the bare repository
        let pr1_branch = bare
            .find_branch(&pr1.head, git2::BranchType::Local)
            .expect("PR 1 branch should exist on bare");
        let pr2_branch = bare
            .find_branch(&pr2.head, git2::BranchType::Local)
            .expect("PR 2 branch should exist on bare");

        let pr1_oid = pr1_branch
            .get()
            .target()
            .expect("PR 1 branch should have OID");
        let pr2_oid = pr2_branch
            .get()
            .target()
            .expect("PR 2 branch should have OID");

        assert!(
            bare.merge_base(pr1_oid, pr2_oid)
                .expect("Failed to get merge base")
                == pr2_oid,
            "PR 1 should be based on PR 2"
        );
    }
}
