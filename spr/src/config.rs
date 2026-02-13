/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use std::collections::HashSet;

use reqwest::Url;

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

pub fn from_jj<F: FnOnce() -> Result<String>>(jj: &crate::jj::Jujutsu, user: F) -> Result<Config> {
    let trunk = jj
        .config_get("revset-aliases.\"trunk()\"")
        .unwrap_or(String::from(""));
    let remote_name = value_from_jj(jj, "spr.githubRemoteName").or_else(|_| {
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
            Ok(String::from(*remote))
        } else {
            Err(crate::error::Error::new(
                "Cannot guess remote. There is none",
            ))
        }
    })?;
    let master_branch = value_from_jj(jj, "spr.githubMasterBranch").or_else(|_| {
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
    })?;
    let branch_prefix =
        value_from_jj(jj, "spr.branchPrefix").or_else(|_| user().map(|u| format!("spr/{}", u)))?;
    let remote_info = jj
        .git_remote_list()?
        .lines()
        .find(|line| line.starts_with(&remote_name))
        .and_then(|s| s.split(' ').last().map(|s| String::from(s)))
        .unwrap_or(String::from(""));

    let repo_with_owner = value_from_jj(jj, "spr.githubRepository").or_else(|_| {
        Url::parse(remote_info.as_str()).map(|url| {
            let path = url.path();
            String::from(path.strip_suffix(".git").unwrap_or(path))
        })
    })?;
    let components: Vec<_> = std::path::Path::new(repo_with_owner.as_str())
        .components()
        .collect();
    let (owner, repo) = match (
        components.get(0).and_then(|c| c.as_os_str().to_str()),
        components.get(1).and_then(|c| c.as_os_str().to_str()),
    ) {
        (Some(owner), Some(repo)) => Ok((owner.into(), repo.into())),
        _ => Err(crate::error::Error::new(
            "Unexpected string for owner and repo...",
        )),
    }?;

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
}
