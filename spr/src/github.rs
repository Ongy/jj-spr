/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use graphql_client::GraphQLQuery;

static COMMENT_MARKER: &str = "\n<!--creatd by jj-spr-->";

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

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/gql/schema.docs.graphql",
    query_path = "src/gql/update_issuecomment.graphql",
    variables_derives = "Clone, Debug",
    response_derives = "Clone, Debug"
)]
pub struct UpdateIssueComment;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/gql/schema.docs.graphql",
    query_path = "src/gql/update_issuecomment.graphql",
    variables_derives = "Clone, Debug",
    response_derives = "Clone, Debug"
)]
pub struct AddComment;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/gql/schema.docs.graphql",
    query_path = "src/gql/update_issuecomment.graphql",
    variables_derives = "Clone, Debug",
    response_derives = "Clone, Debug"
)]
pub struct OldComments;

impl GitHub {
    pub fn new(config: crate::config::Config, crab: octocrab::Octocrab) -> Self {
        Self { config, crab }
    }
}

pub trait GithubPRComment {
    fn editable(&self) -> bool;
    fn body(&self) -> &str;
    fn id(&self) -> &str;
}

pub trait GitHubAdapter {
    type PRAdapter: Send;
    type PRComment: GithubPRComment;

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

    fn list_comments(
        &self,
        number: u64,
    ) -> impl std::future::Future<Output = crate::error::Result<Vec<Self::PRComment>>>;

    fn post_comment<C>(
        &mut self,
        pr: u64,
        content: C,
    ) -> impl std::future::Future<Output = crate::error::Result<()>>
    where
        C: Into<String>;

    fn update_issue_comment<S, C>(
        &mut self,
        issue_comment: S,
        content: C,
    ) -> impl std::future::Future<Output = crate::error::Result<()>>
    where
        S: Into<String>,
        C: Into<String>;

    fn update_pr_comment<S>(
        &mut self,
        number: u64,
        content: S,
    ) -> impl std::future::Future<Output = crate::error::Result<()>>
    where
        S: Into<String>,
    {
        async move {
            let comments = self.list_comments(number).await?;
            let content = format!("{}{}", content.into(), COMMENT_MARKER);

            if let Some(old) = comments
                .into_iter()
                .find(|c| c.editable() && c.body().strip_suffix(COMMENT_MARKER).is_some())
            {
                if old.body() == content {
                    return Ok(());
                }

                return self.update_issue_comment(old.id(), content).await;
            }

            return self.post_comment(number, content).await;
        }
    }
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

impl GithubPRComment for old_comments::OldCommentsRepositoryPullRequestCommentsNodes {
    fn editable(&self) -> bool {
        self.viewer_can_update
    }

    fn body(&self) -> &str {
        self.body.as_ref()
    }

    fn id(&self) -> &str {
        self.id.as_ref()
    }
}

impl GitHubAdapter for &mut GitHub {
    type PRAdapter = octocrab::models::pulls::PullRequest;
    type PRComment = old_comments::OldCommentsRepositoryPullRequestCommentsNodes;

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
                .ok_or_else(|| {
                    crate::error::Error::new(format!("PR {} does not have an node id.", pr.url))
                })?
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
                .ok_or_else(|| {
                    crate::error::Error::new(format!("PR {} does not have an node id.", pr.url))
                })?
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

    async fn update_issue_comment<S, C>(
        &mut self,
        issue_comment: S,
        content: C,
    ) -> crate::error::Result<()>
    where
        S: Into<String>,
        C: Into<String>,
    {
        let variables = update_issue_comment::Variables {
            comment_id: issue_comment.into(),
            body: content.into(),
        };

        let resp: graphql_client::Response<update_issue_comment::ResponseData> = self
            .crab
            .graphql(&UpdateIssueComment::build_query(variables))
            .await?;
        if let Some(errs) = resp.errors
            && !errs.is_empty()
        {
            return Err(crate::error::Error::new(format!("{:?}", errs)));
        }
        return Ok(());
    }

    async fn post_comment<C>(&mut self, number: u64, content: C) -> crate::error::Result<()>
    where
        C: Into<String>,
    {
        let octo_pr = self
            .crab
            .pulls(self.config.owner.clone(), self.config.repo.clone())
            .get(number)
            .await?;
        let variables = add_comment::Variables {
            pull_request_id: octo_pr.node_id.ok_or_else(|| {
                crate::error::Error::new(format!(
                    "PR {} does not have a node-id? Not sure how to handle that.",
                    octo_pr.url,
                ))
            })?,
            body: content.into(),
        };

        let resp: graphql_client::Response<add_comment::ResponseData> = self
            .crab
            .graphql(&AddComment::build_query(variables))
            .await?;
        if let Some(errs) = resp.errors
            && !errs.is_empty()
        {
            return Err(crate::error::Error::new(format!("{:?}", errs)));
        }
        Ok(())
    }

    async fn list_comments(&self, number: u64) -> crate::error::Result<Vec<Self::PRComment>> {
        let variables = old_comments::Variables {
            owner: self.config.owner.clone(),
            name: self.config.repo.clone(),
            number: number as i64,
        };

        let resp: graphql_client::Response<old_comments::ResponseData> = self
            .crab
            .graphql(&OldComments::build_query(variables))
            .await?;
        if let Some(errs) = resp.errors
            && !errs.is_empty()
        {
            return Err(crate::error::Error::new(format!("{:?}", errs)));
        }

        let comments = resp
            .data
            .ok_or_else(|| {
                crate::error::Error::new(format!("No data on OldComments request for {number}"))
            })?
            .repository
            .ok_or_else(|| {
                crate::error::Error::new(format!(
                    "No repository on OldComments request for {number}"
                ))
            })?
            .pull_request
            .ok_or_else(|| {
                crate::error::Error::new(format!(
                    "No pullRequest on OldComments request for {number}"
                ))
            })?
            .comments
            .nodes
            .ok_or_else(|| {
                crate::error::Error::new(format!("No comments on OldComments request for {number}"))
            })?
            .into_iter()
            .filter_map(|c| c)
            .collect();

        Ok(comments)
    }
}

#[cfg(test)]
pub mod fakes {
    #[derive(Debug, Clone)]
    pub struct PullRequestComment {
        pub content: String,
        pub id: String,
        pub editable: bool,
    }

    #[derive(Debug, Clone)]
    pub struct PullRequest {
        pub base: String,
        pub head: String,
        pub number: u64,
        pub title: String,
        pub body: String,
        pub reviewers: Vec<String>,
        pub assignees: Vec<String>,
        pub comments: Vec<PullRequestComment>,
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
                comments: Vec::new(),
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

    impl super::GithubPRComment for PullRequestComment {
        fn editable(&self) -> bool {
            self.editable
        }

        fn body(&self) -> &str {
            self.content.as_ref()
        }

        fn id(&self) -> &str {
            self.id.as_ref()
        }
    }

    impl super::GitHubAdapter for GitHub {
        type PRAdapter = PullRequest;
        type PRComment = PullRequestComment;

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

        async fn list_comments(&self, number: u64) -> crate::error::Result<Vec<Self::PRComment>> {
            Ok(self
                .pull_requests
                .get(&number)
                .ok_or_else(|| crate::error::Error::new("No such PR"))?
                .comments
                .clone())
        }

        async fn post_comment<C>(&mut self, number: u64, content: C) -> crate::error::Result<()>
        where
            C: Into<String>,
        {
            let pr = self
                .pull_requests
                .get_mut(&number)
                .ok_or_else(|| crate::error::Error::new("No such PR"))?;

            pr.comments.push(PullRequestComment {
                content: content.into(),
                id: format!("{}-{}", number, pr.comments.len()),
                editable: true,
            });

            Ok(())
        }

        async fn update_issue_comment<S, C>(
            &mut self,
            issue: S,
            content: C,
        ) -> crate::error::Result<()>
        where
            S: Into<String>,
            C: Into<String>,
        {
            let id = issue.into();
            for (_, pr) in self.pull_requests.iter_mut() {
                for comment in pr.comments.iter_mut() {
                    if comment.id == id {
                        comment.content = content.into();
                        return Ok(());
                    }
                }
            }

            Err(crate::error::Error::new("Couldn't find the comment"))
        }
    }

    impl super::GitHubAdapter for &mut GitHub {
        type PRAdapter = PullRequest;
        type PRComment = PullRequestComment;

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

        async fn list_comments(&self, number: u64) -> crate::error::Result<Vec<Self::PRComment>> {
            Ok(self
                .pull_requests
                .get(&number)
                .ok_or_else(|| crate::error::Error::new("No such PR"))?
                .comments
                .clone())
        }

        async fn post_comment<C>(&mut self, number: u64, content: C) -> crate::error::Result<()>
        where
            C: Into<String>,
        {
            let pr = self
                .pull_requests
                .get_mut(&number)
                .ok_or_else(|| crate::error::Error::new("No such PR"))?;

            pr.comments.push(PullRequestComment {
                content: content.into(),
                id: format!("{}-{}", number, pr.comments.len()),
                editable: true,
            });

            Ok(())
        }

        async fn update_issue_comment<S, C>(
            &mut self,
            issue: S,
            content: C,
        ) -> crate::error::Result<()>
        where
            S: Into<String>,
            C: Into<String>,
        {
            let id = issue.into();
            for (_, pr) in self.pull_requests.iter_mut() {
                for comment in pr.comments.iter_mut() {
                    if comment.id == id {
                        comment.content = content.into();
                        return Ok(());
                    }
                }
            }

            Err(crate::error::Error::new("Couldn't find the comment"))
        }
    }
}
