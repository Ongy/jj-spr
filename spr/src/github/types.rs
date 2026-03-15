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
    pub node: String,
    pub title: String,
    pub body: String,
    pub _reviewers: Vec<String>,
    pub _assignees: Vec<String>,
    pub comments: Vec<PullRequestComment>,
    pub closed: bool,
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
