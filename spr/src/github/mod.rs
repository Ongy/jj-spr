#[cfg(test)]
pub mod fakes;

mod traits;

pub use traits::GHPullRequest;
pub use traits::GitHubAdapter;
use traits::GithubPRComment;
pub use traits::ReviewDecision;

mod real;
pub use real::GitHub;

mod queries;
mod types;
