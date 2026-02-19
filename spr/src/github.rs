/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use octocrab::models::IssueState;

use crate::{
    error::Result,
    message::{MessageSection, MessageSectionsMap, build_github_body},
};
use graphql_client::GraphQLQuery;
use std::collections::BTreeMap;

#[derive(Clone)]
pub struct GitHub {
    config: crate::config::Config,
    crab: octocrab::Octocrab,
}

#[derive(Debug, Clone)]
pub struct PullRequest {
    number: u64,
    state: PullRequestState,
    title: String,
    body: Option<String>,
    sections: MessageSectionsMap,
    base: String,
    head: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewStatus {
    Requested,
    Approved,
    Rejected,
}

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/gql/schema.docs.graphql",
    query_path = "src/gql/add_assignees.graphql",
    variables_derives = "Clone, Debug",
    response_derives = "Clone, Debug"
)]
pub struct UserId;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/gql/schema.docs.graphql",
    query_path = "src/gql/request_reviews.graphql",
    variables_derives = "Clone, Debug",
    response_derives = "Clone, Debug"
)]
pub struct RequestReviews;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/gql/schema.docs.graphql",
    query_path = "src/gql/add_assignees.graphql",
    variables_derives = "Clone, Debug",
    response_derives = "Clone, Debug"
)]
pub struct AddAssignees;

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
            base: octo_request.base.ref_field,
            head: octo_request.head.ref_field,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PullRequestState {
    Open,
    Closed,
}

impl GitHub {
    pub fn new(config: crate::config::Config, crab: octocrab::Octocrab) -> Self {
        Self { config, crab }
    }

    pub async fn get_pull_request_by_head<S>(self, head: S) -> Result<PullRequest>
    where
        S: Into<String>,
    {
        let head = head.into();
        let octo_prs = self
            .crab
            .pulls(self.config.owner.clone(), self.config.repo.clone())
            .list()
            .base(head.clone())
            .per_page(10)
            .send()
            .await?;

        if octo_prs.total_count.unwrap_or(0) > 1 {
            return Err(crate::error::Error::new("Found more than one candidate PR"));
        }

        if let Some(pr) = octo_prs.items.into_iter().next() {
            Ok(PullRequest::from(pr))
        } else {
            Err(crate::error::Error::new(format!(
                "Couldn't find a PR for branch {}",
                head
            )))
        }
    }

    pub async fn get_pull_request(self, number: u64) -> Result<PullRequest> {
        let octo_pr = self
            .crab
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
        let octo_pr = self
            .crab
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

        if let Some(assignees) = message.get(&MessageSection::Assignees) {
            let mut assignee_ids = Vec::new();
            for assignee_login in assignees.split(',').map(|s| s.trim()) {
                let variables = user_id::Variables {
                    login: String::from(assignee_login),
                };

                let resp: graphql_client::Response<user_id::ResponseData> =
                    self.crab.graphql(&UserId::build_query(variables)).await?;
                if let Some(errs) = resp.errors
                    && !errs.is_empty()
                {
                    return Err(crate::error::Error::new(format!("{:?}", errs)));
                }

                let id = resp
                    .data
                    .ok_or_else(|| {
                        crate::error::Error::new(format!(
                            "No data on UserID request for {assignee_login}"
                        ))
                    })?
                    .user
                    .ok_or_else(|| {
                        crate::error::Error::new(
                            "No user in data for UserId request for {assignee_login}",
                        )
                    })?
                    .id;
                assignee_ids.push(id);
            }

            let variables = add_assignees::Variables {
                assignees: assignee_ids,
                assignable_id: octo_pr
                    .node_id
                    .as_ref()
                    .expect("PR should come with nodeid")
                    .to_string(),
            };

            let resp: graphql_client::Response<add_assignees::ResponseData> = self
                .crab
                .graphql(&AddAssignees::build_query(variables))
                .await?;
            if let Some(errs) = resp.errors
                && !errs.is_empty()
            {
                return Err(crate::error::Error::new(format!("{:?}", errs)));
            }
        }

        if let Some(reviewers) = message.get(&MessageSection::Reviewers) {
            let variables = request_reviews::Variables {
                pull_request_id: octo_pr
                    .node_id
                    .as_ref()
                    .expect("PR should come with nodeid")
                    .to_string(),
                users: Some(
                    reviewers
                        .split(',')
                        .map(|s| String::from(s.trim()))
                        .collect(),
                ),
            };

            let resp: graphql_client::Response<request_reviews::ResponseData> = self
                .crab
                .graphql(&RequestReviews::build_query(variables))
                .await?;
            if let Some(errs) = resp.errors
                && !errs.is_empty()
            {
                return Err(crate::error::Error::new(format!("{:?}", errs)));
            }
        }

        Ok(PullRequest::from(octo_pr))
    }

    pub async fn update_pull_request(&self, number: u64, updates: PullRequestUpdate) -> Result<()> {
        self.crab
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
