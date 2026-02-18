/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use graphql_client::{GraphQLQuery, Response};
use octocrab::models::IssueState;
use serde::Deserialize;

use crate::{
    error::{Error, Result, ResultExt},
    message::{MessageSection, MessageSectionsMap, build_github_body},
};
use std::collections::BTreeMap;

#[derive(Clone)]
pub struct GitHub {
    config: crate::config::Config,
    graphql_client: reqwest::Client,
}

#[derive(Debug, Clone)]
pub struct PullRequest {
    number: u64,
    state: PullRequestState,
    title: String,
    body: Option<String>,
    sections: MessageSectionsMap,
    base: GitHubBranch,
    head: GitHubBranch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewStatus {
    Requested,
    Approved,
    Rejected,
}

#[derive(serde::Serialize, Default, Debug)]
pub struct PullRequestUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<PullRequestState>,
}

impl PullRequestUpdate {
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.body.is_none() && self.base.is_none() && self.state.is_none()
    }

    pub fn update_message(&mut self, pull_request: &PullRequest, message: &MessageSectionsMap) {
        let title = message.get(&MessageSection::Title);
        if title.is_some() && title != Some(&pull_request.title) {
            self.title = title.cloned();
        }

        let body = build_github_body(message);
        if pull_request.body.as_ref() != Some(&body) {
            self.body = Some(body);
        }
    }
}

impl From<octocrab::models::pulls::PullRequest> for PullRequest {
    fn from(octo_request: octocrab::models::pulls::PullRequest) -> Self {
        PullRequest {
            number: octo_request.number,
            state: octo_request
                .state
                .map(|s| match s {
                    IssueState::Open => PullRequestState::Open,
                    IssueState::Closed => PullRequestState::Closed,
                    _ => PullRequestState::Open,
                })
                .unwrap_or(PullRequestState::Open),

            title: octo_request.title.unwrap_or("".into()),
            body: octo_request.body,
            sections: BTreeMap::new(),
            base: GitHubBranch::new_from_branch_name(
                octo_request.base.ref_field.as_str(),
                "origin",
                "main",
            ),
            head: GitHubBranch::new_from_branch_name(
                octo_request.head.ref_field.as_str(),
                "origin",
                "main",
            ),
        }
    }
}

#[derive(serde::Serialize, Default, Debug)]
pub struct PullRequestRequestReviewers {
    pub reviewers: Vec<String>,
    pub team_reviewers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PullRequestState {
    Open,
    Closed,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct UserWithName {
    pub login: String,
    pub name: Option<String>,
    #[serde(default)]
    pub is_collaborator: bool,
}

#[derive(Debug, Clone)]
pub struct PullRequestMergeability {
    pub base: GitHubBranch,
    pub head_oid: git2::Oid,
    pub mergeable: Option<bool>,
    pub merge_commit: Option<git2::Oid>,
}

type GitObjectID = String;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/gql/schema.docs.graphql",
    query_path = "src/gql/pullrequest_mergeability_query.graphql",
    response_derives = "Debug"
)]
pub struct PullRequestMergeabilityQuery;

impl GitHub {
    pub fn new(config: crate::config::Config, graphql_client: reqwest::Client) -> Self {
        Self {
            config,
            graphql_client,
        }
    }

    pub async fn get_github_user(login: String) -> Result<UserWithName> {
        octocrab::instance()
            .get::<UserWithName, _, _>(format!("/users/{}", login), None::<&()>)
            .await
            .map_err(Error::from)
    }

    pub async fn get_github_team(
        owner: String,
        team: String,
    ) -> Result<octocrab::models::teams::Team> {
        octocrab::instance()
            .teams(owner)
            .get(team)
            .await
            .map_err(Error::from)
    }

    pub async fn get_pull_request_by_head<S>(self, head: S) -> Result<PullRequest>
    where
        S: Into<String>,
    {
        let octo_prs = octocrab::instance()
            .pulls(self.config.owner.clone(), self.config.repo.clone())
            .list()
            .base(head)
            .per_page(10)
            .send()
            .await?;

        if octo_prs.total_count.unwrap_or(0) > 1 {
            return Err(crate::error::Error::new("Found more than one candidate PR"));
        }

        if let Some(pr) = octo_prs.items.into_iter().next() {
            Ok(PullRequest::from(pr))
        } else {
            Err(crate::error::Error::new("Couldn't find a parent PR"))
        }
    }

    pub async fn get_pull_request(self, number: u64) -> Result<PullRequest> {
        let octo_pr = octocrab::instance()
            .pulls(self.config.owner.clone(), self.config.repo.clone())
            .get(number)
            .await?;

        Ok(PullRequest::from(octo_pr))
    }

    pub async fn create_pull_request(
        &self,
        message: &MessageSectionsMap,
        base_ref_name: String,
        head_ref_name: String,
        draft: bool,
    ) -> Result<PullRequest> {
        let octo_pr = octocrab::instance()
            .pulls(self.config.owner.clone(), self.config.repo.clone())
            .create(
                message
                    .get(&MessageSection::Title)
                    .unwrap_or(&String::new()),
                head_ref_name,
                base_ref_name,
            )
            .body(build_github_body(message))
            .draft(Some(draft))
            .send()
            .await?;

        Ok(PullRequest::from(octo_pr))
    }

    pub async fn update_pull_request(&self, number: u64, updates: PullRequestUpdate) -> Result<()> {
        octocrab::instance()
            .patch::<octocrab::models::pulls::PullRequest, _, _>(
                format!(
                    "/repos/{}/{}/pulls/{}",
                    self.config.owner, self.config.repo, number
                ),
                Some(&updates),
            )
            .await?;

        Ok(())
    }

    pub async fn request_reviewers(
        &self,
        number: u64,
        reviewers: PullRequestRequestReviewers,
    ) -> Result<()> {
        #[derive(Deserialize)]
        struct Ignore {}
        let _: Ignore = octocrab::instance()
            .post(
                format!(
                    "/repos/{}/{}/pulls/{}/requested_reviewers",
                    self.config.owner, self.config.repo, number
                ),
                Some(&reviewers),
            )
            .await?;

        Ok(())
    }

    pub async fn get_pull_request_mergeability(
        &self,
        number: u64,
    ) -> Result<PullRequestMergeability> {
        let variables = pull_request_mergeability_query::Variables {
            name: self.config.repo.clone(),
            owner: self.config.owner.clone(),
            number: number as i64,
        };
        let request_body = PullRequestMergeabilityQuery::build_query(variables);
        let res = self
            .graphql_client
            .post("https://api.github.com/graphql")
            .json(&request_body)
            .send()
            .await?;
        let response_body: Response<pull_request_mergeability_query::ResponseData> =
            res.json().await?;

        if let Some(errors) = response_body.errors {
            let error = Err(Error::new(format!(
                "querying PR #{number} mergeability failed"
            )));
            return errors
                .into_iter()
                .fold(error, |err, e| err.context(e.to_string()));
        }

        let pr = response_body
            .data
            .ok_or_else(|| Error::new("failed to fetch PR"))?
            .repository
            .ok_or_else(|| Error::new("failed to find repository"))?
            .pull_request
            .ok_or_else(|| Error::new("failed to find PR"))?;

        Ok::<_, Error>(PullRequestMergeability {
            base: self.config.new_github_branch_from_ref(&pr.base_ref_name)?,
            head_oid: git2::Oid::from_str(&pr.head_ref_oid)?,
            mergeable: match pr.mergeable {
                pull_request_mergeability_query::MergeableState::CONFLICTING => Some(false),
                pull_request_mergeability_query::MergeableState::MERGEABLE => Some(true),
                pull_request_mergeability_query::MergeableState::UNKNOWN => None,
                _ => None,
            },
            merge_commit: pr
                .merge_commit
                .and_then(|sha| git2::Oid::from_str(&sha.oid).ok()),
        })
    }
}

#[derive(Debug, Clone)]
pub struct GitHubBranch {
    ref_on_github: String,
    ref_local: String,
    is_master_branch: bool,
}

impl GitHubBranch {
    pub fn new_from_ref(ghref: &str, remote_name: &str, master_branch_name: &str) -> Result<Self> {
        let ref_on_github = if ghref.starts_with("refs/heads/") {
            ghref.to_string()
        } else if ghref.starts_with("refs/") {
            return Err(Error::new(format!(
                "Ref '{ghref}' does not refer to a branch"
            )));
        } else {
            format!("refs/heads/{ghref}")
        };

        // The branch name is `ref_on_github` with the `refs/heads/` prefix
        // (length 11) removed
        let branch_name = &ref_on_github[11..];
        let ref_local = format!("refs/remotes/{remote_name}/{branch_name}");
        let is_master_branch = branch_name == master_branch_name;

        Ok(Self {
            ref_on_github,
            ref_local,
            is_master_branch,
        })
    }

    pub fn new_from_branch_name(
        branch_name: &str,
        remote_name: &str,
        master_branch_name: &str,
    ) -> Self {
        Self {
            ref_on_github: format!("refs/heads/{branch_name}"),
            ref_local: format!("refs/remotes/{remote_name}/{branch_name}"),
            is_master_branch: branch_name == master_branch_name,
        }
    }

    pub fn on_github(&self) -> &str {
        &self.ref_on_github
    }

    pub fn local(&self) -> &str {
        &self.ref_local
    }

    pub fn is_master_branch(&self) -> bool {
        self.is_master_branch
    }

    pub fn branch_name(&self) -> &str {
        // The branch name is `ref_on_github` with the `refs/heads/` prefix
        // (length 11) removed
        &self.ref_on_github[11..]
    }
}

pub trait GHPullRequest {
    fn head_branch_name(&self) -> &str;
    fn base_branch_name(&self) -> &str;
    fn pr_number(&self) -> u64;
    fn sections(&self) -> &MessageSectionsMap;
    fn closed(&self) -> bool;
}

pub trait GitHubAdapter {
    type PRAdapter: Send;

    fn pull_request(
        &mut self,
        number: u64,
    ) -> impl std::future::Future<Output = crate::error::Result<Self::PRAdapter>>;

    fn pull_request_by_head<S>(
        &mut self,
        head: S,
    ) -> impl std::future::Future<Output = crate::error::Result<Self::PRAdapter>>
    where
        S: Into<String>;

    fn new_pull_request<H, B>(
        &mut self,
        message: &MessageSectionsMap,
        base_ref_name: B,
        head_ref_name: H,
        draft: bool,
    ) -> impl std::future::Future<Output = crate::error::Result<Self::PRAdapter>>
    where
        H: AsRef<str>,
        B: AsRef<str>;

    fn pull_requests<I>(
        &mut self,
        numbers: I,
    ) -> impl std::future::Future<Output = crate::error::Result<Vec<Option<Self::PRAdapter>>>>
    where
        I: IntoIterator<Item = Option<u64>>,
    {
        async {
            let mut ret = Vec::new();

            for number in numbers.into_iter() {
                ret.push(match number {
                    Some(number) => Some(self.pull_request(number).await?),
                    None => None,
                });
            }
            Ok(ret)
        }
    }
}

impl GHPullRequest for PullRequest {
    fn head_branch_name(&self) -> &str {
        self.head.branch_name()
    }

    fn base_branch_name(&self) -> &str {
        self.base.branch_name()
    }

    fn pr_number(&self) -> u64 {
        self.number
    }

    fn sections(&self) -> &MessageSectionsMap {
        &self.sections
    }

    fn closed(&self) -> bool {
        self.state == PullRequestState::Closed
    }
}

impl GitHubAdapter for &mut GitHub {
    type PRAdapter = PullRequest;
    async fn pull_request(&mut self, number: u64) -> crate::error::Result<Self::PRAdapter> {
        self.clone().get_pull_request(number).await
    }

    async fn pull_request_by_head<S>(&mut self, head: S) -> crate::error::Result<Self::PRAdapter>
    where
        S: Into<String>,
    {
        self.clone().get_pull_request_by_head(head).await
    }

    async fn pull_requests<I>(
        &mut self,
        numbers: I,
    ) -> crate::error::Result<Vec<Option<Self::PRAdapter>>>
    where
        I: IntoIterator<Item = Option<u64>>,
    {
        let pull_requests = numbers.into_iter().map(|number| {
            let gh = self.clone();
            tokio::spawn(async move {
                match number {
                    Some(number) => gh.get_pull_request(number).await.map(|v| Some(v)),
                    None => Ok(None),
                }
            })
        });

        let mut ret = Vec::new();
        for pr in pull_requests {
            ret.push(pr.await??);
        }

        Ok(ret)
    }

    async fn new_pull_request<H, B>(
        &mut self,
        message: &MessageSectionsMap,
        base_ref_name: B,
        head_ref_name: H,
        draft: bool,
    ) -> crate::error::Result<Self::PRAdapter>
    where
        H: AsRef<str>,
        B: AsRef<str>,
    {
        self.create_pull_request(
            message,
            String::from(base_ref_name.as_ref()),
            String::from(head_ref_name.as_ref()),
            draft,
        )
        .await
    }
}

#[cfg(test)]
pub mod fakes {
    use crate::message::{MessageSection, MessageSectionsMap};

    #[derive(Debug, Clone)]
    pub struct PullRequest {
        pub base: String,
        pub head: String,
        pub number: u64,
        pub sections: MessageSectionsMap,
    }

    impl super::GHPullRequest for PullRequest {
        fn head_branch_name(&self) -> &str {
            self.head.as_ref()
        }

        fn base_branch_name(&self) -> &str {
            self.base.as_ref()
        }

        fn pr_number(&self) -> u64 {
            self.number
        }

        fn sections(&self) -> &MessageSectionsMap {
            &self.sections
        }

        fn closed(&self) -> bool {
            false
        }
    }

    #[derive(Debug, Clone)]
    pub struct GitHub {
        pub pull_requests: std::collections::BTreeMap<u64, PullRequest>,
    }

    impl super::GitHubAdapter for GitHub {
        type PRAdapter = PullRequest;
        async fn pull_request(&mut self, number: u64) -> crate::error::Result<Self::PRAdapter> {
            self.pull_requests
                .get(&number)
                .map_or(Err(crate::error::Error::new("No such PR")), |pr| {
                    Ok(pr.clone())
                })
        }

        async fn pull_request_by_head<S>(
            &mut self,
            head: S,
        ) -> crate::error::Result<Self::PRAdapter>
        where
            S: Into<String>,
        {
            let head = head.into();
            self.pull_requests
                .iter()
                .find(|(_, pr)| pr.head == head)
                .map_or(Err(crate::error::Error::new("No such PR")), |(_, pr)| {
                    Ok(pr.clone())
                })
        }

        async fn new_pull_request<H, B>(
            &mut self,
            message: &MessageSectionsMap,
            base_ref_name: B,
            head_ref_name: H,
            _: bool,
        ) -> crate::error::Result<Self::PRAdapter>
        where
            H: AsRef<str>,
            B: AsRef<str>,
        {
            let max = self
                .pull_requests
                .iter()
                .map(|(k, _)| *k)
                .max()
                .unwrap_or(0);
            let pr = Self::PRAdapter {
                number: max + 1,
                base: String::from(base_ref_name.as_ref()),
                head: String::from(head_ref_name.as_ref()),
                sections: message.clone(),
            };

            self.pull_requests.insert(pr.number, pr.clone());

            Ok(pr)
        }
    }

    impl super::GitHubAdapter for &mut GitHub {
        type PRAdapter = PullRequest;
        async fn pull_request(&mut self, number: u64) -> crate::error::Result<Self::PRAdapter> {
            self.pull_requests
                .get(&number)
                .map_or(Err(crate::error::Error::new("No such PR")), |pr| {
                    Ok(pr.clone())
                })
        }

        async fn pull_request_by_head<S>(
            &mut self,
            head: S,
        ) -> crate::error::Result<Self::PRAdapter>
        where
            S: Into<String>,
        {
            let head = head.into();
            self.pull_requests
                .iter()
                .find(|(_, pr)| pr.head == head)
                .map_or(Err(crate::error::Error::new("No such PR")), |(_, pr)| {
                    Ok(pr.clone())
                })
        }

        async fn new_pull_request<H, B>(
            &mut self,
            message: &MessageSectionsMap,
            base_ref_name: B,
            head_ref_name: H,
            _: bool,
        ) -> crate::error::Result<Self::PRAdapter>
        where
            H: AsRef<str>,
            B: AsRef<str>,
        {
            let max = self
                .pull_requests
                .iter()
                .map(|(k, _)| *k)
                .max()
                .unwrap_or(0);
            let pr = Self::PRAdapter {
                number: max + 1,
                base: String::from(base_ref_name.as_ref()),
                head: String::from(head_ref_name.as_ref()),
                sections: message
                    .clone()
                    .into_iter()
                    .filter(|(k, _)| {
                        k != &MessageSection::LastCommit && k != &MessageSection::PullRequest
                    })
                    .collect(),
            };

            self.pull_requests.insert(pr.number, pr.clone());

            Ok(pr)
        }
    }
}

#[cfg(test)]
mod tests {
    // Note this useful idiom: importing names from outer (for mod tests) scope.
    use super::*;

    #[test]
    fn test_new_from_ref_with_branch_name() {
        let r = GitHubBranch::new_from_ref("foo", "github-remote", "masterbranch").unwrap();
        assert_eq!(r.on_github(), "refs/heads/foo");
        assert_eq!(r.local(), "refs/remotes/github-remote/foo");
        assert_eq!(r.branch_name(), "foo");
        assert!(!r.is_master_branch());
    }

    #[test]
    fn test_new_from_ref_with_master_branch_name() {
        let r =
            GitHubBranch::new_from_ref("masterbranch", "github-remote", "masterbranch").unwrap();
        assert_eq!(r.on_github(), "refs/heads/masterbranch");
        assert_eq!(r.local(), "refs/remotes/github-remote/masterbranch");
        assert_eq!(r.branch_name(), "masterbranch");
        assert!(r.is_master_branch());
    }

    #[test]
    fn test_new_from_ref_with_ref_name() {
        let r =
            GitHubBranch::new_from_ref("refs/heads/foo", "github-remote", "masterbranch").unwrap();
        assert_eq!(r.on_github(), "refs/heads/foo");
        assert_eq!(r.local(), "refs/remotes/github-remote/foo");
        assert_eq!(r.branch_name(), "foo");
        assert!(!r.is_master_branch());
    }

    #[test]
    fn test_new_from_ref_with_master_ref_name() {
        let r =
            GitHubBranch::new_from_ref("refs/heads/masterbranch", "github-remote", "masterbranch")
                .unwrap();
        assert_eq!(r.on_github(), "refs/heads/masterbranch");
        assert_eq!(r.local(), "refs/remotes/github-remote/masterbranch");
        assert_eq!(r.branch_name(), "masterbranch");
        assert!(r.is_master_branch());
    }

    #[test]
    fn test_new_from_branch_name() {
        let r = GitHubBranch::new_from_branch_name("foo", "github-remote", "masterbranch");
        assert_eq!(r.on_github(), "refs/heads/foo");
        assert_eq!(r.local(), "refs/remotes/github-remote/foo");
        assert_eq!(r.branch_name(), "foo");
        assert!(!r.is_master_branch());
    }

    #[test]
    fn test_new_from_master_branch_name() {
        let r = GitHubBranch::new_from_branch_name("masterbranch", "github-remote", "masterbranch");
        assert_eq!(r.on_github(), "refs/heads/masterbranch");
        assert_eq!(r.local(), "refs/remotes/github-remote/masterbranch");
        assert_eq!(r.branch_name(), "masterbranch");
        assert!(r.is_master_branch());
    }

    #[test]
    fn test_new_from_ref_with_edge_case_ref_name() {
        let r = GitHubBranch::new_from_ref(
            "refs/heads/refs/heads/foo",
            "github-remote",
            "masterbranch",
        )
        .unwrap();
        assert_eq!(r.on_github(), "refs/heads/refs/heads/foo");
        assert_eq!(r.local(), "refs/remotes/github-remote/refs/heads/foo");
        assert_eq!(r.branch_name(), "refs/heads/foo");
        assert!(!r.is_master_branch());
    }

    #[test]
    fn test_new_from_edge_case_branch_name() {
        let r =
            GitHubBranch::new_from_branch_name("refs/heads/foo", "github-remote", "masterbranch");
        assert_eq!(r.on_github(), "refs/heads/refs/heads/foo");
        assert_eq!(r.local(), "refs/remotes/github-remote/refs/heads/foo");
        assert_eq!(r.branch_name(), "refs/heads/foo");
        assert!(!r.is_master_branch());
    }
}
