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

impl super::GHPullRequest for octocrab::models::pulls::PullRequest {
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

impl super::GithubPRComment for old_comments::PrCommentsNodes {
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

impl super::GitHubAdapter for &mut GitHub {
    type PRAdapter = octocrab::models::pulls::PullRequest;
    type PRComment = old_comments::PrCommentsNodes;

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
        let pull_requests: Vec<_> = numbers
            .into_iter()
            .map(|number| {
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
        let variables = super::queries::mutations::request_reviews::Variables {
            pull_request_id: pr
                .node_id
                .as_ref()
                .ok_or_else(|| {
                    crate::error::Error::new(format!("PR {} does not have an node id.", pr.url))
                })?
                .to_string(),
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
            assignable_id: pr
                .node_id
                .as_ref()
                .ok_or_else(|| {
                    crate::error::Error::new(format!("PR {} does not have an node id.", pr.url))
                })?
                .to_string(),
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
        octo_pr: &Self::PRAdapter,
        content: C,
    ) -> crate::error::Result<()>
    where
        C: Into<String>,
    {
        let variables = super::queries::mutations::add_comment::Variables {
            pull_request_id: octo_pr.node_id.clone().ok_or_else(|| {
                crate::error::Error::new(format!(
                    "PR {} does not have a node-id? Not sure how to handle that.",
                    octo_pr.url,
                ))
            })?,
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

    async fn list_comments(
        &self,
        pr: &Self::PRAdapter,
    ) -> crate::error::Result<Vec<Self::PRComment>> {
        let variables = old_comments::Variables {
            owner: self.config.owner.clone(),
            name: self.config.repo.clone(),
            number: pr.number as i64,
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
                crate::error::Error::new(format!(
                    "No data on OldComments request for {}",
                    pr.number
                ))
            })?
            .repository
            .ok_or_else(|| {
                crate::error::Error::new(format!(
                    "No repository on OldComments request for {}",
                    pr.number
                ))
            })?
            .pull_request
            .ok_or_else(|| {
                crate::error::Error::new(format!(
                    "No pullRequest on OldComments request for {}",
                    pr.number
                ))
            })?
            .comments
            .nodes
            .ok_or_else(|| {
                crate::error::Error::new(format!(
                    "No comments on OldComments request for {}",
                    pr.number
                ))
            })?
            .into_iter()
            .filter_map(|c| c)
            .collect();

        Ok(comments)
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
