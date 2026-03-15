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
        self.body.as_str()
    }

    fn title(&self) -> &str {
        self.title.as_str()
    }

    fn closed(&self) -> bool {
        false
    }

    fn comments(&self) -> Vec<Self::PRComment> {
        self.comments.clone()
    }
}

#[derive(Debug, Clone)]
pub struct GitHub {
    pub pull_requests: std::collections::BTreeMap<u64, PullRequest>,
}

impl GitHub {
    pub fn new() -> Self {
        Self {
            pull_requests: std::collections::BTreeMap::new(),
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

impl super::GitHubAdapter for &mut GitHub {
    type PRAdapter = PullRequest;

    async fn pull_request(&mut self, number: u64) -> crate::error::Result<Self::PRAdapter> {
        self.pull_requests
            .get(&number)
            .map_or(Err(crate::error::Error::new("No such PR")), |pr| {
                Ok(pr.clone())
            })
    }

    async fn pull_request_by_head<S>(&mut self, head: S) -> crate::error::Result<Self::PRAdapter>
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

    async fn post_comment<C>(
        &mut self,
        pr: &Self::PRAdapter,
        content: C,
    ) -> crate::error::Result<()>
    where
        C: Into<String>,
    {
        let pr = self
            .pull_requests
            .get_mut(&pr.number)
            .ok_or_else(|| crate::error::Error::new("No such PR"))?;

        pr.comments.push(PullRequestComment {
            content: content.into(),
            id: format!("{}-{}", pr.number, pr.comments.len()),
            editable: true,
        });

        Ok(())
    }

    async fn update_issue_comment<S, C>(&mut self, issue: S, content: C) -> crate::error::Result<()>
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

    async fn rebase_pr<S>(&mut self, number: u64, new_base: S) -> crate::error::Result<()>
    where
        S: Into<String>,
    {
        self.pull_requests
            .get_mut(&number)
            .ok_or_else(|| crate::error::Error::new("no such pr :("))?
            .base = new_base.into();
        Ok(())
    }
}
