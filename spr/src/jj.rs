/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use std::{
    ffi::OsStr,
    fmt::Display,
    os::unix::ffi::OsStrExt,
    path::PathBuf,
    process::{Command, Stdio},
};

use crate::{
    config::Config,
    error::{Error, Result, ResultExt},
    message::{MessageSection, MessageSectionsMap, build_commit_message, parse_message},
};
use git2::Oid;
use serde::Deserialize;

//r#""{\"parents\": " ++ json(parents.map(|c| c.change_id())) ++ ", \"bookmarks\": " ++ json(bookmarks.map(|b| b.name())) ++ ", \"description\": " ++ json(description) ++ ", \"change_id\": " ++ json(change_id) ++ "}""#,
static REVISION_TEMPLATE: &'static str = r#""{\"parents\": [" ++ parents.map(|c| json(c.change_id())).join(",") ++ "], \"bookmarks\": [" ++ bookmarks.map(|b| json(b.name())).join(",") ++ "], \"description\": " ++ json(description) ++ ", \"change_id\": " ++ json(change_id) ++ " }\n""#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeId {
    id: String,
}

impl AsRef<str> for ChangeId {
    fn as_ref(&self) -> &str {
        return self.id.as_ref();
    }
}

impl<S: Into<String>> From<S> for ChangeId {
    fn from(id: S) -> Self {
        ChangeId { id: id.into() }
    }
}
#[derive(Debug, Clone)]
pub struct Revision {
    pub id: ChangeId,
    pub parent_ids: Vec<ChangeId>,
    pub pull_request_number: Option<u64>,
    pub title: String,
    pub message: MessageSectionsMap,
    pub bookmarks: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawRevision {
    change_id: String,
    parents: Vec<String>,
    bookmarks: Vec<String>,
    description: String,
}

fn parse_pull_request_field(text: &str) -> Option<u64> {
    if text.is_empty() {
        return None;
    }

    let regex = lazy_regex::regex!(r#"^\s*#?\s*(\d+)\s*$"#);
    let m = regex.captures(text);
    if let Some(caps) = m {
        return Some(caps.get(1).unwrap().as_str().parse().unwrap());
    }

    let regex = lazy_regex::regex!(
        r#"^\s*https?://github.com/([\w\-\.]+)/([\w\-\.]+)/pull/(\d+)([/?#].*)?\s*$"#
    );
    let m = regex.captures(text);
    if let Some(caps) = m {
        return Some(caps.get(3).unwrap().as_str().parse().unwrap());
    }

    None
}

impl From<RawRevision> for Revision {
    fn from(value: RawRevision) -> Self {
        let message = parse_message(value.description.as_ref(), MessageSection::Title);
        let pull_request_number = message
            .get(&MessageSection::PullRequest)
            .and_then(|url| parse_pull_request_field(url));
        let title = String::from(
            message
                .get(&MessageSection::Title)
                .map(|t| &t[..])
                .unwrap_or(""),
        );

        Revision {
            id: ChangeId {
                id: value.change_id,
            },
            bookmarks: value.bookmarks,
            parent_ids: value
                .parents
                .into_iter()
                .map(|p| ChangeId { id: p })
                .collect(),

            pull_request_number,
            title,
            message,
        }
    }
}

impl AsRef<Revision> for Revision {
    fn as_ref(&self) -> &Revision {
        self
    }
}

#[derive(Debug, Clone)]
pub struct PreparedCommit {
    pub oid: Oid,
    pub short_id: String,
    pub parent_oid: Oid,
    pub message: MessageSectionsMap,
    pub pull_request_number: Option<u64>,
    pub message_changed: bool,
}

pub struct Jujutsu {
    repo_path: PathBuf,
    jj_bin: PathBuf,
    pub git_repo: git2::Repository,
}

#[derive(Debug, Clone)]
pub struct RevSet(String);

impl RevSet {
    // Constants
    pub fn immutable() -> Self {
        RevSet("immutable()".into())
    }

    pub fn current() -> Self {
        RevSet("@".into())
    }

    pub fn root() -> Self {
        RevSet("root()".into())
    }

    pub fn conflicts() -> Self {
        RevSet("conflicts()".into())
    }

    pub fn divergent() -> Self {
        RevSet("none()".into())
    }

    pub fn merges() -> Self {
        RevSet("merges()".into())
    }

    pub fn mutable() -> Self {
        RevSet("mutable()".into())
    }

    // From known
    /// This is only intended to be used for user input
    pub fn from_arg<S: Into<String>>(s: S) -> Self {
        RevSet(s.into())
    }

    pub fn from_local_branch(b: git2::Branch) -> Result<Self> {
        let name = b.name()?;
        if let Some(name) = name {
            Ok(RevSet(format!("bookmarks({})", name)))
        } else {
            Err(Error::new("Got branch with no name"))
        }
    }

    pub fn from_remote_branch<S: Display>(b: &git2::Branch, r: S) -> Result<Self> {
        let name = b.name()?;
        if let Some(name) = name {
            if let Some(name) = name.strip_prefix(&format!("{}/", r)) {
                Ok(RevSet(format!("remote_bookmarks({}, {})", name, r)))
            } else {
                Err(Error::new(format!(
                    "Branch {} is not on provided remote",
                    name
                )))
            }
        } else {
            Err(Error::new("Got branch with no name"))
        }
    }

    pub fn description<S: AsRef<str>>(d: S) -> Self {
        RevSet(format!("description({})", d.as_ref()))
    }

    // binary
    pub fn and(&self, o: &Self) -> Self {
        RevSet(format!("({}) & ({})", self.0, o.0))
    }

    pub fn or(&self, o: &Self) -> Self {
        RevSet(format!("({}) | ({})", self.0, o.0))
    }

    pub fn without(&self, o: &Self) -> Self {
        RevSet(format!("({}) ~ ({})", self.0, o.0))
    }

    pub fn to(&self, o: &Self) -> Self {
        o.ancestors().without(&self.ancestors())
    }

    pub fn fork_point(&self, o: &Self) -> Self {
        RevSet(format!("fork_point(({}) | ({}))", self.0, o.0))
    }

    // Unary
    pub fn ancestors(&self) -> Self {
        RevSet(format!("::({})", self.0))
    }

    pub fn parent(&self) -> Self {
        RevSet(format!("({})-", self.0))
    }

    pub fn heads(&self) -> Self {
        RevSet(format!("heads({})", self.0))
    }

    // Restrictions
    pub fn exactly(&self, count: u64) -> Self {
        RevSet(format!("exactly({}, {count})", self.0))
    }

    pub fn unique(&self) -> Self {
        self.exactly(1)
    }
}

impl AsRef<str> for RevSet {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl From<&ChangeId> for RevSet {
    fn from(id: &ChangeId) -> Self {
        RevSet(format!("change_id({})", id.id))
    }
}

impl From<&git2::Commit<'_>> for RevSet {
    fn from(c: &git2::Commit) -> Self {
        RevSet(format!("commit_id({})", c.id().to_string()))
    }
}

impl Jujutsu {
    pub fn new<P>(path: P) -> Result<Self>
    where
        P: AsRef<std::path::Path>,
    {
        let jj_bin = get_jj_bin();

        let git_repo = {
            let git_root = Command::new(&jj_bin)
                .current_dir(&path)
                .args(["git", "root"])
                .output()?;
            if !git_root.status.success() {
                return Err(crate::error::Error::new("Couldn't find jj git root"));
            }
            let git_root = std::path::PathBuf::from(OsStr::from_bytes(
                git_root.stdout.as_slice().trim_ascii(),
            ));
            git2::Repository::open(git_root)
        }?;

        let repo_path = Command::new(&jj_bin)
            .current_dir(&path)
            .args(["workspace", "root"])
            .output()?;
        if !repo_path.status.success() {
            return Err(crate::error::Error::new("Couldn't find jj workspace root"));
        }
        let repo_path =
            std::path::PathBuf::from(OsStr::from_bytes(repo_path.stdout.as_slice().trim_ascii()));

        Ok(Self {
            repo_path,
            jj_bin,
            git_repo,
        })
    }

    pub fn get_prepared_commit_for_revision(
        &self,
        config: &Config,
        revision: &str,
    ) -> Result<PreparedCommit> {
        let commit_oid = self.resolve_revision_to_commit_id(revision)?;
        self.prepare_commit(config, commit_oid)
    }

    pub fn read_revision_range(&self, range: &RevSet) -> Result<Vec<Revision>> {
        let output = self.run_ro_captured_with_args([
            "log",
            "--no-graph",
            "--reversed",
            "-r",
            range.as_ref(),
            "--template",
            REVISION_TEMPLATE,
        ])?;

        let mut ret = Vec::new();
        for line in output.lines() {
            let raw: RawRevision = serde_json::from_str(line.trim())
                .context(String::from("Decode revision in range"))?;
            ret.push(Revision::from(raw))
        }

        Ok(ret)
    }

    pub fn read_revision(&self, id: ChangeId) -> Result<Revision> {
        if let Some(r) = self
            .read_revision_range(&RevSet::from(&id).unique())?
            .into_iter()
            .next()
        {
            Ok(r)
        } else {
            Err(crate::error::Error::new(format!(
                "Could not find unique revision for {id:?}"
            )))
        }
    }

    pub fn update_revision_message(&mut self, rev: &Revision) -> Result<()> {
        let new_message = build_commit_message(&rev.message);

        self.run_captured_with_args(["describe", "-r", rev.id.as_ref(), "-m", &new_message])
            .map(|_| {})
    }

    pub fn get_prepared_commits_from_to(
        &self,
        config: &Config,
        from_revision: &str,
        to_revision: &str,
        is_inclusive: bool,
    ) -> Result<Vec<PreparedCommit>> {
        // Get commit range using jj
        let operator = if is_inclusive { "::" } else { ".." };
        let output = self.run_ro_captured_with_args([
            "log",
            "--no-graph",
            "-r",
            &format!("{}{}{}", from_revision, operator, to_revision),
            "--template",
            "commit_id ++ \"\\n\"",
        ])?;

        let mut commits = Vec::new();
        for line in output.lines() {
            let line = line.trim();
            if !line.is_empty() {
                let commit_oid = Oid::from_str(line).map_err(|e| {
                    Error::new(format!("Failed to parse commit ID '{}': {}", line, e))
                })?;
                commits.push(self.prepare_commit(config, commit_oid)?);
            }
        }

        commits.reverse();

        Ok(commits)
    }

    pub fn check_no_uncommitted_changes(&mut self) -> Result<()> {
        let output = self.run_captured_with_args(["status"])?;

        // Check if there are any changes
        // Jujutsu reports "The working copy has no changes" when clean
        if output.trim().is_empty()
            || output.contains("No changes.")
            || output.contains("The working copy has no changes")
        {
            Ok(())
        } else {
            Err(Error::new(format!(
                "You have uncommitted changes:\n{}",
                output
            )))
        }
    }

    pub fn get_all_ref_names(&self) -> Result<std::collections::HashSet<String>> {
        // Use git for ref names since jj doesn't expose them directly
        let refs = self.git_repo.references()?;
        let mut ref_names = std::collections::HashSet::new();

        for reference in refs {
            let reference = reference?;
            if let Some(name) = reference.name() {
                ref_names.insert(name.to_string());
            }
        }

        Ok(ref_names)
    }

    pub fn resolve_reference(&self, ref_name: &str) -> Result<Oid> {
        let reference = self.git_repo.find_reference(ref_name)?;
        reference
            .target()
            .ok_or_else(|| Error::new(format!("Reference {} has no target", ref_name)))
    }

    pub fn get_tree_oid_for_commit(&self, commit_oid: Oid) -> Result<Oid> {
        let commit = self.git_repo.find_commit(commit_oid)?;
        Ok(commit.tree()?.id())
    }

    pub fn create_derived_commit(
        &mut self,
        original_commit_oid: Oid,
        message: &str,
        parent_oids: &[Oid],
    ) -> Result<ChangeId> {
        let original_commit = RevSet::from(&self.git_repo.find_commit(original_commit_oid)?);

        let parents: std::result::Result<Vec<RevSet>, _> = parent_oids
            .iter()
            .map(|r| {
                self.git_repo
                    .find_commit(r.clone())
                    .context(format!("get commit for {r}"))
                    .map(|c| RevSet::from(&c))
            })
            .collect();
        self.new_revision(parents?, Some(message), true)
            .context(String::from("Create new for derived commit"))?;

        let change = self
            .revset_to_change_id(&RevSet::from_arg("at_operation(@-, ..).."))
            .context(String::from("Read change_id from last operation"))?;

        self.restore(
            None as Option<&str>,
            Some(original_commit),
            Some(RevSet::from(&change)),
        )
        .context(String::from("Execute restore for derived commit"))?;
        Ok(change)
    }

    pub fn cherrypick(&self, commit_oid: Oid, onto_oid: Oid) -> Result<git2::Index> {
        let commit = self.git_repo.find_commit(commit_oid)?;
        let onto_commit = self.git_repo.find_commit(onto_oid)?;

        let index = self.git_repo.cherrypick_commit(
            &commit,
            &onto_commit,
            0,
            Some(&git2::MergeOptions::new()),
        )?;
        Ok(index)
    }

    pub fn write_index(&self, mut index: git2::Index) -> Result<Oid> {
        Ok(index.write_tree_to(&self.git_repo)?)
    }

    pub fn rewrite_commit_messages(&mut self, commits: &mut [PreparedCommit]) -> Result<()> {
        if commits.is_empty() {
            return Ok(());
        }

        // Use jj describe to update commit messages, but only for commits that actually changed
        for prepared_commit in commits.iter_mut() {
            // Only update commits whose messages were actually modified
            if !prepared_commit.message_changed {
                continue;
            }

            let new_message = build_commit_message(&prepared_commit.message);

            // Get the change ID for this commit
            let change_id = self.get_change_id_for_commit(prepared_commit.oid)?;

            // Update the commit message using jj describe
            let _ =
                self.run_captured_with_args(["describe", "-r", &change_id, "-m", &new_message])?;

            // Reset the flag after successful update
            prepared_commit.message_changed = false;
        }

        Ok(())
    }

    fn prepare_commit(&self, config: &Config, commit_oid: Oid) -> Result<PreparedCommit> {
        let commit = self.git_repo.find_commit(commit_oid)?;
        let short_id = format!("{:.7}", commit_oid);

        let parent_oid = if commit.parents().count() > 0 {
            commit.parent(0)?.id()
        } else {
            // For initial commit, use a null OID or the commit itself
            commit_oid
        };

        let message_text = commit.message().unwrap_or("").to_string();
        let message = parse_message(&message_text, MessageSection::Title);

        let pull_request_number = message
            .get(&MessageSection::PullRequest)
            .and_then(|url| config.parse_pull_request_field(url));

        Ok(PreparedCommit {
            oid: commit_oid,
            short_id,
            parent_oid,
            message,
            pull_request_number,
            message_changed: false,
        })
    }

    pub fn resolve_revision_to_commit_id(&self, revision: &str) -> Result<Oid> {
        let output = self.run_ro_captured_with_args([
            "log",
            "--no-graph",
            "-r",
            revision,
            "--template",
            "commit_id",
        ])?;

        let commit_id_str = output.trim();
        Oid::from_str(commit_id_str).map_err(|e| {
            Error::new(format!(
                "Failed to parse commit ID '{}' from jj output: {}",
                commit_id_str, e
            ))
        })
    }

    pub fn squash(&mut self) -> Result<()> {
        let _ = self.run_captured_with_args(["squash", "--use-destination-message"])?;

        Ok(())
    }

    pub fn squash_from_into(&mut self, from: &RevSet, to: &RevSet) -> Result<()> {
        let _ = self.run_captured_with_args([
            "squash",
            "--use-destination-message",
            "--from",
            from.as_ref(),
            "--to",
            to.as_ref(),
        ])?;

        Ok(())
    }

    pub fn commit<M: AsRef<str>>(&mut self, message: M) -> Result<()> {
        let _ = self.run_captured_with_args(["commit", "--message", message.as_ref()])?;

        Ok(())
    }

    pub fn bookmark_create<S: AsRef<str>>(
        &mut self,
        name: S,
        revision: Option<&str>,
    ) -> Result<()> {
        let mut args = vec!["bookmark", "create", name.as_ref()];
        if let Some(rev) = revision {
            args.extend(["-r", rev]);
        }
        let _ = self.run_captured_with_args(args)?;
        Ok(())
    }

    pub fn revset_to_change_ids(&self, revset: &RevSet) -> Result<Vec<ChangeId>> {
        // Get commit range using jj
        let output = self.run_ro_captured_with_args([
            "log",
            "--no-graph",
            "--reversed",
            "-r",
            revset.as_ref(),
            "--template",
            "change_id ++ \"\\n\"",
        ])?;

        Ok(output.lines().map(|l| ChangeId::from(l.trim())).collect())
    }

    pub fn revset_to_change_id(&self, revset: &RevSet) -> Result<ChangeId> {
        let ids = self.revset_to_change_ids(&revset.unique())?;
        if let Some(id) = ids.first() {
            Ok(id.clone())
        } else {
            Err(Error::new(format!(
                "Revset {:?} returned no revision",
                revset.unique()
            )))
        }
    }

    pub fn squash_copy(&mut self, revision: &RevSet, onto: ChangeId) -> Result<()> {
        let _ = self.run_captured_with_args([
            "duplicate",
            revision.as_ref(),
            "--destination",
            onto.id.as_str(),
            "--config",
            format!(
                "templates.duplicate_description='''\"jj-spr-duplicate-for-{}\"'''",
                onto.id
            )
            .as_str(),
        ])?;

        let _ = self.run_captured_with_args([
            "squash",
            "--into",
            onto.id.as_str(),
            "--from",
            format!(
                "description(substring:\"jj-spr-duplicate-for-{}\")",
                onto.id
            )
            .as_str(),
            "--use-destination-message",
        ])?;

        Ok(())
    }

    fn get_change_id_for_commit(&self, commit_oid: Oid) -> Result<String> {
        // Get the change ID for a given commit OID
        let output = self.run_ro_captured_with_args([
            "log",
            "--no-graph",
            "-r",
            &commit_oid.to_string(),
            "--template",
            "change_id",
        ])?;

        Ok(output.trim().to_string())
    }

    fn run_captured_command<I, S>(&self, mut command: Command, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        command.args(args);
        command.current_dir(&self.repo_path);
        command.stdout(Stdio::piped());

        let child = command.spawn().context("jj failed to spawn".to_string())?;
        let output = child
            .wait_with_output()
            .context("failed to wait for jj to exit".to_string())?;

        if output.status.success() {
            let output = String::from_utf8(output.stdout)
                .context("jujutsu output was not valid UTF-8".to_string())?;
            Ok(output)
        } else {
            Err(Error::new(format!(
                "jujutsu exited with code {}, stderr:\n{}",
                output
                    .status
                    .code()
                    .map_or_else(|| "(unknown)".to_string(), |c| c.to_string()),
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }

    fn run_ro_captured_with_args<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = Command::new(&self.jj_bin);
        command.args(["--no-pager", "--quiet", "--ignore-working-copy"]);
        self.run_captured_command(command, args)
    }

    fn run_captured_with_args<I, S>(&mut self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = Command::new(&self.jj_bin);
        command.args(["--no-pager", "--quiet"]);
        self.run_captured_command(command, args)
    }

    pub fn abandon(&mut self, revset: &RevSet) -> Result<()> {
        self.run_captured_with_args(["abandon", revset.as_ref()])
            .map(|_| {})
    }

    pub fn rebase(&mut self, revset: &RevSet, target: &RevSet) -> Result<()> {
        self.run_captured_with_args([
            "rebase",
            "--source",
            revset.as_ref(),
            "--destination",
            target.as_ref(),
        ])
        .map(|_| {})
    }

    pub fn rebase_branch(&mut self, revset: &RevSet, target: ChangeId) -> Result<()> {
        self.run_captured_with_args([
            "rebase",
            "--branch",
            revset.as_ref(),
            "--destination",
            target.as_ref(),
        ])
        .map(|_| {})
    }

    pub fn run_git_fetch(&mut self) -> Result<()> {
        self.run_captured_with_args(["git", "fetch"]).map(|_| {})
    }

    pub fn new_revision<M: AsRef<str>, I>(
        &mut self,
        parents: I,
        message: Option<M>,
        no_edit: bool,
    ) -> Result<()>
    where
        I: IntoIterator<Item = RevSet>,
    {
        let parents: Vec<_> = parents.into_iter().collect();
        let mut args = vec!["new"];
        args.extend(parents.iter().map(|r| r.as_ref()));

        if let Some(ref m) = message {
            args.extend(["-m", m.as_ref()]);
        }
        if no_edit {
            args.push("--no-edit")
        }

        self.run_captured_with_args(args).map(|_| {})
    }

    pub fn restore<Fr: AsRef<str>, T: AsRef<str>, Fi: AsRef<str>>(
        &mut self,
        files: Option<Fi>,
        from: Option<Fr>,
        to: Option<T>,
    ) -> Result<()> {
        let mut args = vec!["restore"];
        if let Some(ref from) = from {
            args.extend(["--from", from.as_ref()]);
        }
        if let Some(ref to) = to {
            args.extend(["--into", to.as_ref()]);
        }
        if let Some(ref files) = files {
            args.push(files.as_ref());
        }

        self.run_captured_with_args(args).map(|_| {})
    }

    pub fn git_remote_list(&self) -> Result<String> {
        self.run_ro_captured_with_args(["git", "remote", "list"])
    }

    pub fn git_remote_remove<S>(&mut self, remote: S) -> Result<()>
    where
        S: AsRef<str>,
    {
        self.run_captured_with_args(["git", "remote", "remove", remote.as_ref()])
            .map(|_| {})
    }

    pub fn git_remote_add<S, Su>(&mut self, remote: S, url: Su) -> Result<()>
    where
        S: AsRef<str>,
        Su: AsRef<str>,
    {
        self.run_captured_with_args(["git", "remote", "add", remote.as_ref(), url.as_ref()])
            .map(|_| {})
    }

    pub fn config_set<K, V>(&mut self, key: K, value: V, user: bool) -> Result<()>
    where
        K: AsRef<str>,
        V: AsRef<str>,
    {
        self.run_captured_with_args([
            "config",
            "set",
            if user { "--user" } else { "--repo" },
            key.as_ref(),
            value.as_ref(),
        ])
        .map(|_| {})
    }

    pub fn config_get<S: AsRef<str>>(&self, key: S) -> Result<String> {
        let mut command = Command::new(&self.jj_bin);
        command.args(["--no-pager", "--quiet", "--ignore-working-copy"]);
        command.stderr(Stdio::null());
        self.run_captured_command(command, ["config", "get", key.as_ref()])
            .map(|v| v.trim().into())
    }

    pub fn update(&mut self) -> Result<()> {
        self.run_captured_with_args(["workspace", "update-stale"])
            .map(|_| {})
    }

    pub fn is_empty(&self, change: &ChangeId) -> Result<bool> {
        let output = self.run_ro_captured_with_args([
            "log",
            "--no-graph",
            "-r",
            RevSet::from(change).unique().as_ref(),
            "--template",
            "empty",
        ])?;

        Ok(output.trim() == "true")
    }

    pub fn fix(&mut self, source: &RevSet) -> Result<()> {
        self.run_captured_with_args(["fix", "--source", source.as_ref()])
            .map(|_| {})
    }
}

fn get_jj_bin() -> PathBuf {
    std::env::var_os("JJ").map_or_else(|| "jj".into(), |v| v.into())
}

#[cfg(test)]
mod tests {
    use crate::testing;

    use super::*;
    use std::{fs, path::Path};

    fn create_jujutsu_commit(repo_path: &Path, message: &str, file_content: &str) -> String {
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

    fn create_jujutsu_commit_from(
        repo_path: &Path,
        message: &str,
        file_content: &str,
        parents: &[&str],
    ) -> String {
        let mut args = Vec::new();
        args.push("new");
        args.extend_from_slice(parents);
        // Create new revision to commit later
        let output = std::process::Command::new("jj")
            .args(args)
            .current_dir(repo_path)
            .output()
            .expect("Failed to run jj new");

        if !output.status.success() {
            panic!(
                "Failed to prepare revision: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

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

    #[test]
    fn test_jujutsu_creation() {
        let (_temp_dr, jj, _) = testing::setup::repo_with_origin();
        assert!(jj.repo_path.exists());
        assert!(jj.repo_path.join(".jj").exists());
    }

    #[test]
    fn test_revision_reading() {
        let (_temp_dr, jj, _) = testing::setup::repo_with_origin();

        // Create some commits
        let _commit1 = create_jujutsu_commit(&jj.repo_path, "First commit", "content1");
        let commit2 = create_jujutsu_commit(&jj.repo_path, "Second commit", "content2");
        let commit3 = create_jujutsu_commit(&jj.repo_path, "Third commit", "content3");

        let commit4 = create_jujutsu_commit_from(
            &jj.repo_path,
            "Fourth commit",
            "content4",
            &[commit2.as_str(), commit3.as_str()],
        );

        let c1 = jj.read_revision(ChangeId::from(commit2));
        assert!(c1.is_ok(), "Failed to read @ revision: {:?}", c1.err());
        assert!(
            c1.unwrap().parent_ids.len() == 1,
            "Got more than one parent of c1",
        );

        let c2 = jj.read_revision(ChangeId::from(commit4));
        assert!(c2.is_ok(), "Failed to read @ revision: {:?}", c2.err());
        assert!(
            c2.unwrap().parent_ids.len() == 2,
            "Got parent count != 2 from c2",
        );
    }

    #[test]
    fn test_revision_resolution() {
        let (_temp_dr, jj, _) = testing::setup::repo_with_origin();
        let config = testing::config::basic();

        // Create some commits
        let _commit1 = create_jujutsu_commit(&jj.repo_path, "First commit", "content1");
        let _commit2 = create_jujutsu_commit(&jj.repo_path, "Second commit", "content2");

        // Test resolving current revision (@)
        let result = jj.get_prepared_commit_for_revision(&config, "@");
        assert!(
            result.is_ok(),
            "Failed to resolve @ revision: {:?}",
            result.err()
        );

        // Test resolving previous revision (@-)
        let result = jj.get_prepared_commit_for_revision(&config, "@-");
        assert!(
            result.is_ok(),
            "Failed to resolve @- revision: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_commit_range() {
        let (_temp_dr, jj, _) = testing::setup::repo_with_origin();
        let config = testing::config::basic();

        // Create multiple commits
        let _commit1 = create_jujutsu_commit(&jj.repo_path, "First commit", "content1");
        let _commit2 = create_jujutsu_commit(&jj.repo_path, "Second commit", "content2");
        let _commit3 = create_jujutsu_commit(&jj.repo_path, "Third commit", "content3");

        // Test getting commit range
        let result = jj.get_prepared_commits_from_to(&config, "@----", "@-", false);
        assert!(
            result.is_ok(),
            "Failed to get commit range: {:?}",
            result.err()
        );

        if let Ok(commits) = result {
            // Should get 3 commits in the range
            assert_eq!(commits.len(), 3, "Should get exactly 3 commits in range");

            // Commits must be in bottom-to-top order (oldest to newest).
            let first_commit_title = commits[0]
                .message
                .get(&MessageSection::Title)
                .expect("First commit should have a title");
            let last_commit_title = commits[2]
                .message
                .get(&MessageSection::Title)
                .expect("Last commit should have a title");

            assert!(
                first_commit_title.contains("First commit"),
                "First element should be the oldest commit 'First commit', got: {}",
                first_commit_title
            );
            assert!(
                last_commit_title.contains("Third commit"),
                "Last element should be the newest commit 'Third commit', got: {}",
                last_commit_title
            );
        }
    }

    #[test]
    fn test_status_check() {
        let (_temp_dr, mut jj, _) = testing::setup::repo_with_origin();

        // Should pass since new repo has no changes
        let result = jj.check_no_uncommitted_changes();
        assert!(
            result.is_ok(),
            "Status check should pass for clean repo: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_derived_commit_has_different_timestamp() {
        let (_temp_dr, mut jj, _) = testing::setup::repo_with_origin();

        // Create a commit with some content
        let _commit1 = create_jujutsu_commit(&jj.repo_path, "Original commit", "original content");

        // Get the original commit
        let original_commit_oid = jj
            .resolve_revision_to_commit_id("@-")
            .expect("Failed to resolve @- revision");

        // Sleep briefly to ensure timestamp difference
        std::thread::sleep(std::time::Duration::from_secs(1));

        // Create a derived commit
        let parent_oids = {
            let original_commit = jj
                .git_repo
                .find_commit(original_commit_oid)
                .expect("Failed to find original commit");

            if original_commit.parents().count() > 0 {
                vec![
                    original_commit
                        .parent(0)
                        .expect("Failed to get parent")
                        .id(),
                ]
            } else {
                vec![]
            }
        };

        let change = RevSet::from(
            &jj.create_derived_commit(original_commit_oid, "Derived commit message", &parent_oids)
                .expect("Failed to create derived commit"),
        );
        let derived_commit_oid = jj
            .resolve_revision_to_commit_id(change.as_ref())
            .expect("Faield to find commit for derived change");

        // Get the derived commit
        let derived_commit = jj
            .git_repo
            .find_commit(derived_commit_oid)
            .expect("Failed to find derived commit");

        let original_commit = jj
            .git_repo
            .find_commit(original_commit_oid)
            .expect("Failed to find original commit");
        // Verify that derived timestamps are newer than original
        let original_author_time = original_commit.author().when();
        let derived_author_time = derived_commit.author().when();
        let original_committer_time = original_commit.committer().when();
        let derived_committer_time = derived_commit.committer().when();

        assert!(
            derived_author_time.seconds() > original_author_time.seconds(),
            "Derived commit author timestamp should be newer than original"
        );

        assert!(
            derived_committer_time.seconds() > original_committer_time.seconds(),
            "Derived commit committer timestamp should be newer than original"
        );
    }

    #[test]
    fn jj_from_workspace() {
        let (temp_dr, jj, _) = testing::setup::repo_with_origin();
        let workspace_path = temp_dr.path().join("workspace");

        std::process::Command::new("jj")
            .current_dir(jj.repo_path)
            .args([
                "workspace",
                "add",
                String::from_utf8_lossy(workspace_path.as_os_str().as_encoded_bytes()).as_ref(),
            ])
            .status()
            .expect("Should be able to create workspace");
        super::Jujutsu::new(workspace_path).expect("Expect to be able to create JJ from workspace");
    }
}
