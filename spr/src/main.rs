/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

//! A Jujutsu subcommand for submitting and updating GitHub Pull Requests from
//! local Jujutsu commits that may be amended and rebased. Pull Requests can be
//! stacked to allow for a series of code reviews of interdependent code.

use clap::{Parser, Subcommand};
use jj_spr::{
    commands,
    config::{self, get_auth_token},
    error::{Error, Result, ResultExt},
    output::output,
};
use reqwest::{self, header};

#[derive(Parser, Debug)]
#[clap(
    name = "jj-spr",
    version,
    about = "Jujutsu subcommand: Submit pull requests for individual, amendable, rebaseable commits to GitHub"
)]
pub struct Cli {
    /// GitHub personal access token (if not given taken from jj config
    /// spr.githubAuthToken)
    #[clap(long)]
    github_auth_token: Option<String>,

    /// GitHub repository ('org/name', if not given taken from config
    /// spr.githubRepository)
    #[clap(long)]
    github_repository: Option<String>,

    /// prefix to be used for branches created for pull requests (if not given
    /// taken from jj config spr.branchPrefix, defaulting to
    /// 'spr/<GITHUB_USERNAME>/')
    #[clap(long)]
    branch_prefix: Option<String>,

    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Interactive assistant for configuring spr in a local GitHub-backed Git
    /// repository
    Init,

    /// Create a new or update an existing Pull Request on GitHub
    Push(commands::push::PushOptions),

    /// Pull state from github and merge into local pull requests
    Sync(commands::sync::SyncOpts),

    /// Update local commit message with content on GitHub
    Fetch(commands::fetch::FetchOptions),

    /// List open Pull Requests on GitHub and their review decision
    List,

    /// Create a new branch with the contents of an existing Pull Request
    Adopt(commands::adopt::AdoptOptions),
}

#[derive(Debug, thiserror::Error)]
pub enum OptionsError {
    #[error("GitHub repository must be given as 'OWNER/REPO', but given value was '{0}'")]
    InvalidRepository(String),
}

pub async fn spr() -> Result<()> {
    let cli = Cli::parse();

    if let Commands::Init = cli.command {
        return commands::init::init().await;
    }

    // Discover the Jujutsu repository and get the colocated Git repo
    let current_dir = std::env::current_dir()?;
    let repo = git2::Repository::discover(&current_dir)?;

    // Verify this is a Jujutsu repository by checking for .jj directory
    let repo_path = repo
        .workdir()
        .ok_or_else(|| Error::new("Repository must have a working directory".to_string()))?
        .to_path_buf();

    let jj_dir = repo_path.join(".jj");
    if !jj_dir.exists() {
        return Err(Error::new(
            "This command requires a Jujutsu repository. Run 'jj git init --colocate' to create one.".to_string()
        ));
    }

    let git_config = repo.config()?;

    let mut jj = jj_spr::jj::Jujutsu::new(repo)
        .context("could not initialize Jujutsu backend".to_owned())?;

    let github_auth_token = match cli.github_auth_token {
        Some(v) => v,
        None => get_auth_token(&git_config)
            .ok_or_else(|| Error::new("GitHub auth token must be configured".to_string()))?,
    };

    let crab = octocrab::OctocrabBuilder::default()
        .personal_token(github_auth_token.clone())
        .add_header(
            http::header::HeaderName::from_static("x-github-next-global-id"),
            String::from("1"),
        )
        .build()?;

    let mut headers = header::HeaderMap::new();
    headers.insert(header::ACCEPT, "application/json".parse()?);
    headers.insert(
        header::USER_AGENT,
        format!("spr/{}", env!("CARGO_PKG_VERSION")).try_into()?,
    );
    headers.insert(
        header::AUTHORIZATION,
        format!("Bearer {}", github_auth_token).parse()?,
    );

    let graphql_client = reqwest::Client::builder()
        .default_headers(headers)
        .build()?;
    let config = config::from_jj(&jj, async || {
        let user = crab.current().user().await?;
        Ok(user.login)
    })
    .await?;
    let mut gh = jj_spr::github::GitHub::new(config.clone(), crab);

    match cli.command {
        Commands::Fetch(opts) => commands::fetch::fetch(opts, &mut jj, &mut gh, &config).await?,
        Commands::List => commands::list::list(graphql_client, &config).await?,
        Commands::Adopt(opts) => commands::adopt::adopt(opts, &mut jj, &mut gh, &config).await?,
        Commands::Push(opts) => commands::push::push(&mut jj, &mut gh, &config, opts).await?,
        Commands::Sync(opts) => commands::sync::sync(&mut jj, &mut gh, &config, opts).await?,
        // The following commands are executed above and return from this
        // function before it reaches this match.
        Commands::Init => (),
    };

    Ok::<_, Error>(())
}

#[tokio::main]
async fn main() -> Result<()> {
    if let Err(error) = spr().await {
        for message in error.messages() {
            output("🛑", message)?;
        }
        std::process::exit(1);
    }

    Ok(())
}
