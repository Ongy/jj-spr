static COMMENT_MARKER: &str = "\n<!--creatd by jj-spr-->";

pub trait GithubPRComment {
    fn editable(&self) -> bool;
    fn body(&self) -> &str;
    fn id(&self) -> &str;
}

pub trait GHPullRequest {
    type PRComment: GithubPRComment;

    fn head_branch_name(&self) -> &str;
    fn base_branch_name(&self) -> &str;
    fn pr_number(&self) -> u64;
    fn body(&self) -> &str;
    fn title(&self) -> &str;
    fn closed(&self) -> bool;
    fn comments(&self) -> Vec<Self::PRComment>;
}

pub trait GitHubAdapter {
    type PRAdapter: GHPullRequest + Send;

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

    fn post_comment<C>(
        &mut self,
        pr: &Self::PRAdapter,
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
        pr: &Self::PRAdapter,
        content: S,
    ) -> impl std::future::Future<Output = crate::error::Result<()>>
    where
        S: Into<String>,
    {
        async move {
            let comments = pr.comments();
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

            return self.post_comment(pr, content).await;
        }
    }

    fn rebase_pr<S>(
        &mut self,
        number: u64,
        new_base: S,
    ) -> impl std::future::Future<Output = crate::error::Result<()>>
    where
        S: Into<String>;
}
