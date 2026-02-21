/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use graphql_client::GraphQLQuery;

#[derive(Clone)]
pub struct GitHub {
    config: crate::config::Config,
    crab: octocrab::Octocrab,
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

impl GitHub {
    pub fn new(config: crate::config::Config, crab: octocrab::Octocrab) -> Self {
        Self { config, crab }
    }
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

    fn new_pull_request<H, B, St, Sb>(
        &mut self,
        title: St,
        body: Sb,
        base_ref_name: B,
        head_ref_name: H,
        draft: bool,
    ) -> impl std::future::Future<Output = crate::error::Result<Self::PRAdapter>>
    where
        H: AsRef<str>,
        B: AsRef<str>,
        St: Into<String>,
        Sb: Into<String>;

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

    fn add_reviewers<S, I>(
        &mut self,
        pr: &Self::PRAdapter,
        reviewers: I,
    ) -> impl std::future::Future<Output = crate::error::Result<()>>
    where
        S: Into<String>,
        I: IntoIterator<Item = S>;

    fn add_assignees<S, I>(
        &mut self,
        pr: &Self::PRAdapter,
        assignees: I,
    ) -> impl std::future::Future<Output = crate::error::Result<()>>
    where
        S: Into<String>,
        I: IntoIterator<Item = S>;
}

pub trait GHPullRequest {
    fn head_branch_name(&self) -> &str;
    fn base_branch_name(&self) -> &str;
    fn pr_number(&self) -> u64;
    fn body(&self) -> &str;
    fn title(&self) -> &str;
    fn closed(&self) -> bool;
}

impl GHPullRequest for octocrab::models::pulls::PullRequest {
    fn head_branch_name(&self) -> &str {
        self.head.ref_field.as_str()
    }

    fn base_branch_name(&self) -> &str {
        self.base.ref_field.as_str()
    }

    fn pr_number(&self) -> u64 {
        self.number
    }

    fn title(&self) -> &str {
        self.title.as_ref().map_or("", |s| s.as_ref())
    }

    fn body(&self) -> &str {
        self.body.as_ref().map_or("", |s| s.as_ref())
    }

    fn closed(&self) -> bool {
        self.state
            .as_ref()
            .map_or(false, |s| s == &octocrab::models::IssueState::Closed)
    }
}

impl GitHubAdapter for &mut GitHub {
    type PRAdapter = octocrab::models::pulls::PullRequest;
    async fn pull_request(&mut self, number: u64) -> crate::error::Result<Self::PRAdapter> {
        let octo_pr = self
            .crab
            .pulls(self.config.owner.clone(), self.config.repo.clone())
            .get(number)
            .await?;

        Ok(octo_pr)
    }

    async fn pull_request_by_head<S>(&mut self, head: S) -> crate::error::Result<Self::PRAdapter>
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
            Ok(pr)
        } else {
            Err(crate::error::Error::new(format!(
                "Couldn't find a PR for branch {}",
                head
            )))
        }
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
                    Some(number) => {
                        let octo_pr = gh
                            .crab
                            .pulls(gh.config.owner.clone(), gh.config.repo.clone())
                            .get(number)
                            .await;
                        octo_pr.map(|v| Some(v))
                    }
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

    async fn new_pull_request<H, B, St, Sb>(
        &mut self,
        title: St,
        body: Sb,
        base_ref_name: B,
        head_ref_name: H,
        draft: bool,
    ) -> crate::error::Result<Self::PRAdapter>
    where
        H: AsRef<str>,
        B: AsRef<str>,
        St: Into<String>,
        Sb: Into<String>,
    {
        let octo_pr = self
            .crab
            .pulls(self.config.owner.clone(), self.config.repo.clone())
            .create(title.into(), head_ref_name.as_ref(), base_ref_name.as_ref())
            .body(body.into())
            .draft(Some(draft))
            .send()
            .await?;

        Ok(octo_pr)
    }

    async fn add_reviewers<S, I>(
        &mut self,
        pr: &Self::PRAdapter,
        reviewers: I,
    ) -> crate::error::Result<()>
    where
        S: Into<String>,
        I: IntoIterator<Item = S>,
    {
        let reviewers = reviewers.into_iter().map(|s| s.into());
        let variables = request_reviews::Variables {
            pull_request_id: pr
                .node_id
                .as_ref()
                .expect("Every PR should have a node id")
                .to_string(),
            users: Some(reviewers.collect()),
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

        Ok(())
    }

    async fn add_assignees<S, I>(
        &mut self,
        pr: &Self::PRAdapter,
        assignees: I,
    ) -> crate::error::Result<()>
    where
        S: Into<String>,
        I: IntoIterator<Item = S>,
    {
        let mut assignee_ids = Vec::new();
        for assignee_login in assignees.into_iter().map(|s| s.into()) {
            let variables = user_id::Variables {
                login: assignee_login.clone(),
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
            assignable_id: pr
                .node_id
                .as_ref()
                .expect("Every PR should have a node id")
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

        Ok(())
    }
}

#[cfg(test)]
pub mod fakes {
    #[derive(Debug, Clone)]
    pub struct PullRequest {
        pub base: String,
        pub head: String,
        pub number: u64,
        pub title: String,
        pub body: String,
        pub reviewers: Vec<String>,
        pub assignees: Vec<String>,
    }

    impl PullRequest {
        pub fn new<Ba, H, T, Bo>(base: Ba, head: H, number: u64, title: T, body: Bo) -> Self
        where
            Ba: Into<String>,
            H: Into<String>,
            T: Into<String>,
            Bo: Into<String>,
        {
            PullRequest {
                base: base.into(),
                head: head.into(),
                number,
                title: title.into(),
                body: body.into(),
                reviewers: Vec::new(),
                assignees: Vec::new(),
            }
        }
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

        fn body(&self) -> &str {
            self.body.as_str()
        }

        fn title(&self) -> &str {
            self.title.as_str()
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

        async fn new_pull_request<H, B, St, Sb>(
            &mut self,
            title: St,
            body: Sb,
            base_ref_name: B,
            head_ref_name: H,
            _draft: bool,
        ) -> crate::error::Result<Self::PRAdapter>
        where
            H: AsRef<str>,
            B: AsRef<str>,
            St: Into<String>,
            Sb: Into<String>,
        {
            let max = self
                .pull_requests
                .iter()
                .map(|(k, _)| *k)
                .max()
                .unwrap_or(0);
            let pr = Self::PRAdapter::new(
                base_ref_name.as_ref(),
                head_ref_name.as_ref(),
                max + 1,
                title,
                body,
            );

            self.pull_requests.insert(pr.number, pr.clone());

            Ok(pr)
        }

        async fn add_reviewers<S, I>(
            &mut self,
            pr: &Self::PRAdapter,
            reviewers: I,
        ) -> crate::error::Result<()>
        where
            S: Into<String>,
            I: IntoIterator<Item = S>,
        {
            if let Some(pr) = self.pull_requests.get_mut(&pr.number) {
                pr.reviewers.extend(reviewers.into_iter().map(|s| s.into()));
            }
            Ok(())
        }

        async fn add_assignees<S, I>(
            &mut self,
            pr: &Self::PRAdapter,
            assignees: I,
        ) -> crate::error::Result<()>
        where
            S: Into<String>,
            I: IntoIterator<Item = S>,
        {
            if let Some(pr) = self.pull_requests.get_mut(&pr.number) {
                pr.assignees.extend(assignees.into_iter().map(|s| s.into()));
            }
            Ok(())
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

        async fn new_pull_request<H, B, St, Sb>(
            &mut self,
            title: St,
            body: Sb,
            base_ref_name: B,
            head_ref_name: H,
            _draft: bool,
        ) -> crate::error::Result<Self::PRAdapter>
        where
            H: AsRef<str>,
            B: AsRef<str>,
            St: Into<String>,
            Sb: Into<String>,
        {
            let max = self
                .pull_requests
                .iter()
                .map(|(k, _)| *k)
                .max()
                .unwrap_or(0);
            let pr = Self::PRAdapter::new(
                base_ref_name.as_ref(),
                head_ref_name.as_ref(),
                max + 1,
                title,
                body,
            );

            self.pull_requests.insert(pr.number, pr.clone());

            Ok(pr)
        }

        async fn add_reviewers<S, I>(
            &mut self,
            pr: &Self::PRAdapter,
            reviewers: I,
        ) -> crate::error::Result<()>
        where
            S: Into<String>,
            I: IntoIterator<Item = S>,
        {
            if let Some(pr) = self.pull_requests.get_mut(&pr.number) {
                pr.reviewers.extend(reviewers.into_iter().map(|s| s.into()));
            }
            Ok(())
        }

        async fn add_assignees<S, I>(
            &mut self,
            pr: &Self::PRAdapter,
            assignees: I,
        ) -> crate::error::Result<()>
        where
            S: Into<String>,
            I: IntoIterator<Item = S>,
        {
            if let Some(pr) = self.pull_requests.get_mut(&pr.number) {
                pr.assignees.extend(assignees.into_iter().map(|s| s.into()));
            }
            Ok(())
        }
    }
}
