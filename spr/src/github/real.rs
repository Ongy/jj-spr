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

#[derive(Debug, Clone)]
pub struct PullRequestComment {
    content: String,
    id: String,
    editable: bool,
}

#[derive(Debug)]
pub struct PullRequest {
    base: String,
    head: String,
    number: u64,
    node: String,
    title: String,
    body: String,
    _reviewers: Vec<String>,
    _assignees: Vec<String>,
    comments: Vec<PullRequestComment>,
    closed: bool,
}

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/gql/schema.docs.graphql",
    query_path = "src/gql/update_issuecomment.graphql",
    variables_derives = "Clone, Debug",
    response_derives = "Clone, Debug"
)]
pub struct OldComments;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/gql/schema.docs.graphql",
    query_path = "src/gql/update_issuecomment.graphql",
    variables_derives = "Clone, Debug",
    response_derives = "Clone, Debug"
)]
pub struct ByHead;

impl From<old_comments::OldCommentsRepositoryPullRequest> for PullRequest {
    fn from(pr: old_comments::PR) -> Self {
        let assignees = pr
            .assignees
            .nodes
            .unwrap_or(Vec::new())
            .into_iter()
            .filter_map(|node| node.map(|assignee| assignee.id))
            .collect();
        let reviewers = pr
            .review_requests
            .and_then(|r| r.nodes)
            .unwrap_or(Vec::new())
            .into_iter()
            .filter_map(|node| node.map(|r| r.id))
            .collect();

        let comments = pr
            .comments
            .nodes
            .unwrap_or(Vec::new())
            .into_iter()
            .filter_map(|node| {
                node.map(|comment| PullRequestComment {
                    editable: comment.viewer_can_update,
                    content: comment.body,
                    id: comment.id,
                })
            })
            .collect();

        Self {
            base: pr.base_ref_name,
            head: pr.head_ref_name,
            number: pr.number as u64,
            node: pr.id,
            body: pr.body,
            title: pr.title,
            closed: pr.closed,
            _reviewers: reviewers,
            _assignees: assignees,
            comments,
        }
    }
}

impl From<by_head::PR> for PullRequest {
    fn from(pr: by_head::PR) -> Self {
        unsafe {
            // These types are generated from the same fragment in graphql.
            // They should be exacly equal, but we'll see if this holds in practice.
            let pr: old_comments::PR = std::mem::transmute(pr);
            Self::from(pr)
        }
    }
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

impl super::GHPullRequest for PullRequest {
    type PRComment = PullRequestComment;

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
        self.body.as_ref()
    }

    fn title(&self) -> &str {
        self.title.as_ref()
    }

    fn closed(&self) -> bool {
        self.closed
    }

    fn comments(&self) -> Vec<Self::PRComment> {
        self.comments.clone()
    }
}

impl GitHub {
    pub fn new(config: crate::config::Config, crab: octocrab::Octocrab) -> Self {
        Self { config, crab }
    }
}

impl super::GitHubAdapter for &mut GitHub {
    type PRAdapter = PullRequest;

    async fn pull_request(&mut self, number: u64) -> crate::error::Result<Self::PRAdapter> {
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

        let pr = resp
            .data
            .ok_or_else(|| crate::error::Error::new("No data on PR request"))?
            .repository
            .ok_or_else(|| crate::error::Error::new("No repository in PR request"))?
            .pull_request
            .ok_or_else(|| crate::error::Error::new("No PR in PR request"))?;

        Ok(PullRequest::from(pr))
    }

    async fn pull_request_by_head<S>(&mut self, head: S) -> crate::error::Result<Self::PRAdapter>
    where
        S: Into<String>,
    {
        let head = head.into();
        let variables = by_head::Variables {
            owner: self.config.owner.clone(),
            name: self.config.repo.clone(),
            head: head.clone(),
        };

        let resp: graphql_client::Response<by_head::ResponseData> =
            self.crab.graphql(&ByHead::build_query(variables)).await?;
        if let Some(errs) = resp.errors
            && !errs.is_empty()
        {
            return Err(crate::error::Error::new(format!("{:?}", errs)));
        }

        let prs: Vec<_> = resp
            .data
            .ok_or_else(|| crate::error::Error::new("No data on by_head request"))?
            .repository
            .ok_or_else(|| crate::error::Error::new("No repository in by_head request"))?
            .pull_requests
            .nodes
            .ok_or_else(|| crate::error::Error::new("No nodes in pull_requests for by_head"))?
            .into_iter()
            .filter_map(|pr| pr)
            .collect();

        if prs.len() > 1 {
            return Err(crate::error::Error::new("Found more than one candidate PR"));
        }

        if let Some(pr) = prs.into_iter().next() {
            Ok(PullRequest::from(pr))
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
        let pull_requests: Vec<_> = numbers
            .into_iter()
            .map(|number| {
                let mut gh = self.clone();
                tokio::spawn(async move {
                    match number {
                        Some(number) => {
                            let mut r = &mut gh;
                            let pr = r.pull_request(number).await;
                            pr.map(|v| Some(v))
                        }
                        None => Ok(None),
                    }
                })
            })
            .collect();

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

        Ok(PullRequest {
            node: octo_pr
                .node_id
                .ok_or_else(|| crate::error::Error::new("No nodeID on new PR"))?,
            base: String::from(base_ref_name.as_ref()),
            head: String::from(head_ref_name.as_ref()),
            number: octo_pr.number,
            title: octo_pr.title.unwrap_or(String::new()),
            body: octo_pr.body.unwrap_or(String::new()),
            _reviewers: Vec::new(),
            _assignees: Vec::new(),
            comments: Vec::new(),
            closed: false,
        })
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
        let variables = super::queries::mutations::request_reviews::Variables {
            pull_request_id: pr.node.clone(),
            users: Some(reviewers.collect()),
        };

        let resp: graphql_client::Response<
            super::queries::mutations::request_reviews::ResponseData,
        > = self
            .crab
            .graphql(&super::queries::mutations::RequestReviews::build_query(
                variables,
            ))
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
            let variables = super::queries::userid::user_id::Variables {
                login: assignee_login.clone(),
            };

            let resp: graphql_client::Response<super::queries::userid::user_id::ResponseData> =
                self.crab
                    .graphql(&super::queries::userid::UserId::build_query(variables))
                    .await?;
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

        let variables = super::queries::mutations::add_assignees::Variables {
            assignees: assignee_ids,
            assignable_id: pr.node.clone(),
        };

        let resp: graphql_client::Response<super::queries::mutations::add_assignees::ResponseData> =
            self.crab
                .graphql(&super::queries::mutations::AddAssignees::build_query(
                    variables,
                ))
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
        let variables = super::queries::mutations::update_issue_comment::Variables {
            comment_id: issue_comment.into(),
            body: content.into(),
        };

        let resp: graphql_client::Response<
            super::queries::mutations::update_issue_comment::ResponseData,
        > = self
            .crab
            .graphql(&super::queries::mutations::UpdateIssueComment::build_query(
                variables,
            ))
            .await?;
        if let Some(errs) = resp.errors
            && !errs.is_empty()
        {
            return Err(crate::error::Error::new(format!("{:?}", errs)));
        }
        return Ok(());
    }

    async fn post_comment<C>(
        &mut self,
        pr: &Self::PRAdapter,
        content: C,
    ) -> crate::error::Result<()>
    where
        C: Into<String>,
    {
        let variables = super::queries::mutations::add_comment::Variables {
            pull_request_id: pr.node.clone(),
            body: content.into(),
        };

        let resp: graphql_client::Response<super::queries::mutations::add_comment::ResponseData> =
            self.crab
                .graphql(&super::queries::mutations::AddComment::build_query(
                    variables,
                ))
                .await?;
        if let Some(errs) = resp.errors
            && !errs.is_empty()
        {
            return Err(crate::error::Error::new(format!("{:?}", errs)));
        }
        Ok(())
    }

    async fn rebase_pr<S>(&mut self, number: u64, new_base: S) -> crate::error::Result<()>
    where
        S: Into<String>,
    {
        let octo_pr = self
            .crab
            .pulls(self.config.owner.clone(), self.config.repo.clone())
            .get(number)
            .await?;

        let variables = super::queries::mutations::update_pr_base::Variables {
            pull_request_id: octo_pr
                .node_id
                .ok_or_else(|| crate::error::Error::new("Couldn't find id for PR"))?,
            branch: new_base.into(),
        };

        let resp: graphql_client::Response<
            super::queries::mutations::update_pr_base::ResponseData,
        > = self
            .crab
            .graphql(&super::queries::mutations::UpdatePRBase::build_query(
                variables,
            ))
            .await?;
        if let Some(errs) = resp.errors
            && !errs.is_empty()
        {
            return Err(crate::error::Error::new(format!("{:?}", errs)));
        }
        Ok(())
    }
}
