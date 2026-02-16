/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use std::collections::HashSet;

use crate::{error::Result, github::GitHubBranch, utils::slugify};

#[derive(Clone, Debug)]
pub struct Config {
    pub owner: String,
    pub repo: String,
    pub remote_name: String,
    pub master_ref: GitHubBranch,
    pub branch_prefix: String,
}

impl Config {
    pub fn new(
        owner: String,
        repo: String,
        remote_name: String,
        master_branch: String,
        branch_prefix: String,
    ) -> Self {
        let master_ref =
            GitHubBranch::new_from_branch_name(&master_branch, &remote_name, &master_branch);
        Self {
            owner,
            repo,
            remote_name,
            master_ref,
            branch_prefix,
        }
    }

    pub fn pull_request_url(&self, number: u64) -> String {
        format!(
            "https://github.com/{owner}/{repo}/pull/{number}",
            owner = &self.owner,
            repo = &self.repo
        )
    }

    pub fn parse_pull_request_field(&self, text: &str) -> Option<u64> {
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
        if let Some(caps) = m
            && self.owner == caps.get(1).unwrap().as_str()
            && self.repo == caps.get(2).unwrap().as_str()
        {
            return Some(caps.get(3).unwrap().as_str().parse().unwrap());
        }

        None
    }

    pub fn get_new_branch_name(&self, existing_ref_names: &HashSet<String>, title: &str) -> String {
        self.find_unused_branch_name(existing_ref_names, &slugify(title))
    }

    pub fn get_base_branch_name(
        &self,
        existing_ref_names: &HashSet<String>,
        title: &str,
    ) -> String {
        self.find_unused_branch_name(
            existing_ref_names,
            &format!("{}.{}", self.master_ref.branch_name(), &slugify(title)),
        )
    }

    fn find_unused_branch_name(&self, existing_ref_names: &HashSet<String>, slug: &str) -> String {
        let remote_name = &self.remote_name;
        let branch_prefix = &self.branch_prefix;
        let mut branch_name = format!("{branch_prefix}{slug}");
        let mut suffix = 0;

        loop {
            let remote_ref = format!("refs/remotes/{remote_name}/{branch_name}");

            if !existing_ref_names.contains(&remote_ref) {
                return branch_name;
            }

            suffix += 1;
            branch_name = format!("{branch_prefix}{slug}-{suffix}");
        }
    }

    pub fn new_github_branch_from_ref(&self, ghref: &str) -> Result<GitHubBranch> {
        GitHubBranch::new_from_ref(ghref, &self.remote_name, self.master_ref.branch_name())
    }

    pub fn new_github_branch(&self, branch_name: &str) -> GitHubBranch {
        GitHubBranch::new_from_branch_name(
            branch_name,
            &self.remote_name,
            self.master_ref.branch_name(),
        )
    }
}

fn value_from_jj<S: AsRef<str> + Copy>(jj: &crate::jj::Jujutsu, key: S) -> Result<String> {
    jj.config_get(key).or_else(|_| {
        Ok(String::from(
            jj.git_repo.config()?.get_str(key.as_ref())?.trim(),
        ))
    })
}

pub fn remote_from_jj(jj: &crate::jj::Jujutsu) -> Result<String> {
    let trunk = jj
        .config_get("revset-aliases.\"trunk()\"")
        .unwrap_or(String::from(""));

    value_from_jj(jj, "spr.githubRemoteName").or_else(|_| {
        let remotes = jj.git_remote_list()?;
        let remotes: Vec<_> = remotes.lines().collect();

        if remotes.len() > 1 {
            let parts: Vec<_> = trunk.split('@').collect();
            if parts.len() <= 2
                && let Some(remote) = parts.get(1)
            {
                Ok(String::from(*remote))
            } else {
                Err(crate::error::Error::new(
                    "Unexpected trunk() alias. Cannot guess which remote is upstream",
                ))
            }
        } else if let Some(remote) = remotes.first() {
            if let Some(name) = remote.split(' ').next() {
                Ok(String::from(name))
            } else {
                Err(crate::error::Error::new(
                    "Couldn't find name of listed remote",
                ))
            }
        } else {
            Err(crate::error::Error::new(
                "Cannot guess remote. There is none",
            ))
        }
    })
}

pub fn repo_and_owner_from_jj(
    jj: &crate::jj::Jujutsu,
    remote_name: &str,
) -> Result<(String, String)> {
    let remote_info = jj
        .git_remote_list()?
        .lines()
        .find(|line| line.starts_with(&remote_name))
        .and_then(|s| s.split(' ').last().map(|s| String::from(s)))
        .unwrap_or(String::from(""));

    let repo_with_owner = value_from_jj(jj, "spr.githubRepository").or_else(|_| {
        let no_suffix = remote_info
            .strip_suffix(".git")
            .unwrap_or(remote_info.as_str());
        if let Some(index) = no_suffix.find("github.com") {
            if let Some((_, path)) = no_suffix.split_at_checked(index + "github.com".len() + 1) {
                Ok(String::from(path))
            } else {
                Err(crate::error::Error::new(format!(
                    "Couldn't split along 'github.com' in {}",
                    remote_info
                )))
            }
        } else {
            Err(crate::error::Error::new(format!(
                "Couldn't find 'github.com' in {}",
                remote_info
            )))
        }
    })?;
    let components: Vec<_> = std::path::Path::new(repo_with_owner.as_str())
        .components()
        .collect();
    match (
        components.get(0).and_then(|c| c.as_os_str().to_str()),
        components.get(1).and_then(|c| c.as_os_str().to_str()),
    ) {
        (Some(owner), Some(repo)) => Ok((repo.into(), owner.into())),
        _ => Err(crate::error::Error::new(
            "Unexpected string for owner and repo...",
        )),
    }
}

pub fn default_branch_from_jj(jj: &crate::jj::Jujutsu) -> Result<String> {
    let trunk = jj
        .config_get("revset-aliases.\"trunk()\"")
        .unwrap_or(String::from(""));

    value_from_jj(jj, "spr.githubMasterBranch").or_else(|_| {
        let parts: Vec<_> = trunk.split('@').collect();
        if parts.len() <= 2
            && let Some(branch) = parts.get(0)
        {
            Ok(String::from(*branch))
        } else {
            Err(crate::error::Error::new(
                "Unexpected trunk() alias. Cannot guess which branch is upstream",
            ))
        }
    })
}

pub async fn from_jj<F: AsyncFnOnce() -> Result<String>>(
    jj: &crate::jj::Jujutsu,
    user: F,
) -> Result<Config> {
    let remote_name = remote_from_jj(jj)?;
    let branch_prefix = match value_from_jj(jj, "spr.branchPrefix") {
        Ok(val) => Ok(val),
        Err(_) => user().await.map(|u| format!("spr/{}/", u)),
    }?;
    let master_branch = default_branch_from_jj(jj)?;
    let (repo, owner) = repo_and_owner_from_jj(jj, remote_name.as_ref())?;

    Ok(Config::new(
        owner,
        repo,
        remote_name,
        master_branch,
        branch_prefix,
    ))
}

pub enum AuthTokenSource {
    Config(String),
    GitHubCLI(String),
}

impl AuthTokenSource {
    pub fn token(&self) -> &String {
        match self {
            AuthTokenSource::Config(token) | AuthTokenSource::GitHubCLI(token) => token,
        }
    }
}

pub fn get_auth_token(git_config: &git2::Config) -> Option<String> {
    get_auth_token_with_source(git_config).map(|v| v.token().to_owned())
}

pub fn get_auth_token_with_source(git_config: &git2::Config) -> Option<AuthTokenSource> {
    // Prefer the configured token if it exists
    if let Some(token) = get_config_value("spr.githubAuthToken", git_config) {
        return Some(AuthTokenSource::Config(token));
    }

    // Try to get a token from the gh CLI
    let output = std::process::Command::new("gh")
        .args(["auth", "token"])
        .stdout(std::process::Stdio::piped())
        .output()
        .ok()?;

    if output.status.success() {
        Some(AuthTokenSource::GitHubCLI(
            String::from_utf8(output.stdout).ok()?.trim().to_owned(),
        ))
    } else {
        None
    }
}

// Helper function to get config value from jj first, then git
pub fn get_config_value(key: &str, git_config: &git2::Config) -> Option<String> {
    // Try jj config first
    if let Ok(output) = std::process::Command::new("jj")
        .args(["config", "get", key])
        .output()
        && output.status.success()
        && let Ok(value) = String::from_utf8(output.stdout)
    {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    // Fall back to git config
    git_config.get_string(key).ok()
}

/// Helper function to set config value in jj (repo-level)
pub fn set_jj_config(key: &str, value: &str, repo_path: &std::path::Path) -> Result<()> {
    let output = std::process::Command::new("jj")
        .args(["config", "set", "--repo", key, value])
        .current_dir(repo_path)
        .output()
        .map_err(|e| crate::error::Error::new(format!("Failed to execute jj config set: {}", e)))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(crate::error::Error::new(format!(
            "jj config set failed for key '{}': {}",
            key, stderr
        )))
    }
}

#[cfg(test)]
mod tests {
    // Note this useful idiom: importing names from outer (for mod tests) scope.
    use super::*;

    fn config_factory() -> Config {
        crate::config::Config::new(
            "acme".into(),
            "codez".into(),
            "origin".into(),
            "master".into(),
            "spr/foo/".into(),
        )
    }

    #[test]
    fn test_set_jj_config_success() {
        // Create a temporary jj repo for testing
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let path = temp_dir.path();

        // Initialize git repo first
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(path)
            .output()
            .expect("Failed to init git repo");

        // Initialize jj repo (colocated)
        let jj_init = std::process::Command::new("jj")
            .args(["git", "init", "--colocate"])
            .current_dir(path)
            .output()
            .expect("Failed to init jj repo");

        if !jj_init.status.success() {
            // Skip test if jj is not available
            return;
        }

        // Test setting a config value
        let result = set_jj_config("spr.githubRepository", "test/repo", path);
        assert!(result.is_ok(), "Should successfully set config");

        // Verify the config was set
        let output = std::process::Command::new("jj")
            .args(["config", "get", "spr.githubRepository"])
            .current_dir(path)
            .output()
            .expect("Failed to get config");

        assert!(output.status.success());
        let value = String::from_utf8(output.stdout).unwrap();
        assert_eq!(value.trim(), "test/repo");
    }

    #[test]
    fn test_set_jj_config_multiple_values() {
        // Create a temporary jj repo for testing
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let path = temp_dir.path();

        // Initialize git repo first
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(path)
            .output()
            .expect("Failed to init git repo");

        // Initialize jj repo (colocated)
        let jj_init = std::process::Command::new("jj")
            .args(["git", "init", "--colocate"])
            .current_dir(path)
            .output()
            .expect("Failed to init jj repo");

        if !jj_init.status.success() {
            // Skip test if jj is not available
            return;
        }

        // Set multiple config values
        assert!(set_jj_config("spr.githubRepository", "owner/repo", path).is_ok());
        assert!(set_jj_config("spr.branchPrefix", "spr/test/", path).is_ok());
        assert!(set_jj_config("spr.requireApproval", "false", path).is_ok());

        // Verify all configs were set correctly
        let output = std::process::Command::new("jj")
            .args(["config", "get", "spr.githubRepository"])
            .current_dir(path)
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            "owner/repo"
        );

        let output = std::process::Command::new("jj")
            .args(["config", "get", "spr.branchPrefix"])
            .current_dir(path)
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            "spr/test/"
        );

        let output = std::process::Command::new("jj")
            .args(["config", "get", "spr.requireApproval"])
            .current_dir(path)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "false");
    }

    #[test]
    fn test_set_jj_config_invalid_repo() {
        // Try to set config in a non-existent directory
        let result = set_jj_config(
            "spr.test",
            "value",
            std::path::Path::new("/nonexistent/path"),
        );
        assert!(result.is_err(), "Should fail for invalid repo path");
    }

    #[test]
    fn test_pull_request_url() {
        let gh = config_factory();

        assert_eq!(
            &gh.pull_request_url(123),
            "https://github.com/acme/codez/pull/123"
        );
    }

    #[test]
    fn test_parse_pull_request_field_empty() {
        let gh = config_factory();

        assert_eq!(gh.parse_pull_request_field(""), None);
        assert_eq!(gh.parse_pull_request_field("   "), None);
        assert_eq!(gh.parse_pull_request_field("\n"), None);
    }

    #[test]
    fn test_parse_pull_request_field_number() {
        let gh = config_factory();

        assert_eq!(gh.parse_pull_request_field("123"), Some(123));
        assert_eq!(gh.parse_pull_request_field("   123 "), Some(123));
        assert_eq!(gh.parse_pull_request_field("#123"), Some(123));
        assert_eq!(gh.parse_pull_request_field(" # 123"), Some(123));
    }

    #[test]
    fn test_parse_pull_request_field_url() {
        let gh = config_factory();

        assert_eq!(
            gh.parse_pull_request_field("https://github.com/acme/codez/pull/123"),
            Some(123)
        );
        assert_eq!(
            gh.parse_pull_request_field("  https://github.com/acme/codez/pull/123  "),
            Some(123)
        );
        assert_eq!(
            gh.parse_pull_request_field("https://github.com/acme/codez/pull/123/"),
            Some(123)
        );
        assert_eq!(
            gh.parse_pull_request_field("https://github.com/acme/codez/pull/123?x=a"),
            Some(123)
        );
        assert_eq!(
            gh.parse_pull_request_field("https://github.com/acme/codez/pull/123/foo"),
            Some(123)
        );
        assert_eq!(
            gh.parse_pull_request_field("https://github.com/acme/codez/pull/123#abc"),
            Some(123)
        );
    }

    mod from_jj {
        use crate::testing;

        #[tokio::test]
        async fn basic() {
            let (_tmpdir, mut jj, _) = testing::setup::repo_with_origin();

            jj.git_remote_remove("origin")
                .expect("Failed to remove origin");
            jj.git_remote_add("origin", "git@github.com/Ongy/jj-spr.git")
                .expect("Failed to add origin");
            jj.config_set("revset-aliases.\"trunk()\"", "main@origin", false)
                .expect("Failed to set trunk alias.");

            let config = super::from_jj(&jj, async || Ok(String::from("user")))
                .await
                .expect("Failed to guess config from jj");

            assert_eq!(config.owner, "Ongy", "Failed to guess owner of repo");
            assert_eq!(config.repo, "jj-spr", "Failed to guess repo name");
            assert_eq!(config.remote_name, "origin", "Failed to guess remote name");
            assert_eq!(
                config.branch_prefix, "spr/user/",
                "Failed to build default branch prefix"
            );
            assert_eq!(
                config.master_ref.branch_name(),
                "main",
                "Failed to guess default target branch"
            );
        }

        #[tokio::test]
        async fn multi_remote() {
            let (_tmpdir, mut jj, _) = testing::setup::repo_with_origin();

            jj.git_remote_remove("origin")
                .expect("Failed to remove origin");
            jj.git_remote_add("origin", "git@github.com/Ongy/jj-spr.git")
                .expect("Failed to add origin");
            jj.git_remote_add("mine", "git@github.com/user/jj-spr.git")
                .expect("Failed to add mine remote");
            jj.config_set("revset-aliases.\"trunk()\"", "dev@mine", false)
                .expect("Failed to set trunk alias.");

            let config = super::from_jj(&jj, async || Ok(String::from("user")))
                .await
                .expect("Failed to guess config from jj");

            assert_eq!(config.owner, "user", "Failed to guess owner of repo");
            assert_eq!(config.repo, "jj-spr", "Failed to guess repo name");
            assert_eq!(config.remote_name, "mine", "Failed to guess remote name");
            assert_eq!(
                config.branch_prefix, "spr/user/",
                "Failed to build default branch prefix"
            );
            assert_eq!(
                config.master_ref.branch_name(),
                "dev",
                "Failed to guess default target branch"
            );
        }

        #[tokio::test]
        async fn prefers_config() {
            let (_tmpdir, mut jj, _) = testing::setup::repo_with_origin();

            jj.git_remote_remove("origin")
                .expect("Failed to remove origin");
            jj.git_remote_add("origin", "git@github.com/Ongy/jj-spr.git")
                .expect("Failed to add origin");
            jj.git_remote_add("my-remote", "git@github.com/Ongy/jj-spr.git")
                .expect("Failed to add origin");
            jj.config_set("revset-aliases.\"trunk()\"", "main@origin", false)
                .expect("Failed to set trunk alias.");

            jj.config_set("spr.branchPrefix", "my-prefix", false)
                .expect("Failed to set branch prefix config");
            jj.config_set("spr.githubRemoteName", "my-remote", false)
                .expect("Failed to set remote config");
            jj.config_set("spr.githubMasterBranch", "branch", false)
                .expect("Failed to set target branch config");

            let config = super::from_jj(&jj, async || {
                Err(crate::error::Error::new(
                    "Shouldn't be called when the branch prefix isn't constructed",
                ))
            })
            .await
            .expect("Failed to guess config from jj");

            assert_eq!(config.owner, "Ongy", "Failed to guess owner of repo");
            assert_eq!(config.repo, "jj-spr", "Failed to guess repo name");
            assert_eq!(
                config.remote_name, "my-remote",
                "Failed to read remote from config"
            );
            assert_eq!(
                config.branch_prefix, "my-prefix",
                "Failed to read branch prefix from config"
            );
            assert_eq!(
                config.master_ref.branch_name(),
                "branch",
                "Failed to read target branch from config"
            );
        }
    }
}
