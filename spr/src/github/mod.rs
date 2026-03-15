#[cfg(test)]
pub mod fakes;

mod traits;

pub use traits::GHPullRequest;
pub use traits::GitHubAdapter;
use traits::GithubPRComment;

mod real;
pub use real::GitHub;

mod queries;
