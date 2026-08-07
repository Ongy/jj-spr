/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

//! A Jujutsu subcommand for submitting and updating GitHub Pull Requests from
//! local Jujutsu commits that may be amended and rebased. Pull Requests can be
//! stacked to allow for a series of code reviews of interdependent code.

use std::{pin::Pin, str::FromStr, sync::Arc, time::Duration};

use clap::{Parser, Subcommand};
use jj_spr::{
    commands,
    config::{self, get_auth_token},
    error::{Error, Result, ResultExt},
};

use octocrab::service::middleware::{auth_header::AuthHeaderLayer, base_uri::BaseUriLayer};
use octocrab::{AuthState, service::middleware::extra_headers::ExtraHeadersLayer};

use bytes::Bytes;
use http::{HeaderMap, Response};
use http::{HeaderValue, Uri, header::HeaderName};
use http::{Request, header::USER_AGENT};
use hyper_tls::HttpsConnector;
use tower_http::classify::ServerErrorsFailureClass;
use tower_layer::Layer;
use tracing::Span;

const GITHUB_BASE_URI: &str = "https://api.github.com";
const GITHUB_BASE_UPLOAD_URI: &str = "https://uploads.github.com";

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

struct DebugBody<B> {
    inner: std::pin::Pin<Box<B>>,
}

impl<B> DebugBody<B> {
    fn new(b: B) -> Self {
        DebugBody { inner: Box::pin(b) }
    }
}

impl<B> http_body::Body for DebugBody<B>
where
    B: http_body::Body<Data = Bytes>,
{
    type Data = B::Data;
    type Error = B::Error;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<std::result::Result<http_body::Frame<Self::Data>, Self::Error>>>
    {
        let s = Pin::into_inner(self);
        std::pin::pin!(&mut s.inner).poll_frame(cx).map_ok(|f| {
            if let Some(d) = f.data_ref() {
                println!("sending {} bytes", d.len());
                hexdump::hexdump(d);
            }
            f
        })
    }
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

    /// Create a new branch with the contents of an existing Pull Request
    Adopt(commands::adopt::AdoptOptions),

    /// Remove the PR tracking information form a revision. E.g. to have a "clean" change after adopt.
    Detach(commands::detach::DetachOptions),
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
    let current_dir =
        std::env::current_dir().context(format!("Failed to find the working directory"))?;
    let mut jj = jj_spr::jj::Jujutsu::new(current_dir)
        .context("could not initialize Jujutsu backend".to_owned())?;
    let git_config = jj.git_repo.config()?;

    let github_auth_token = match cli.github_auth_token {
        Some(v) => v,
        None => get_auth_token(&git_config)
            .ok_or_else(|| Error::new("GitHub auth token must be configured".to_string()))?,
    };

    let crab = {
        let connector = HttpsConnector::new();
        let client =
            hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
                .build(connector);

        let mut hmap: Vec<(HeaderName, HeaderValue)> = vec![];

        // Add the user agent header required by GitHub
        hmap.push((USER_AGENT, HeaderValue::from_str("octocrab").unwrap()));

        let auth_header = Some(
            format!("Bearer {}", github_auth_token.clone())
                .parse()
                .unwrap(),
        );
        let base_uri = Uri::from_str(GITHUB_BASE_URI).unwrap();
        let upload_uri = Uri::from_str(GITHUB_BASE_UPLOAD_URI).unwrap();

        let client = ExtraHeadersLayer::new(Arc::new(hmap)).layer(client);
        let client = BaseUriLayer::new(base_uri.clone()).layer(client);
        let client = AuthHeaderLayer::new(auth_header, base_uri, upload_uri).layer(client);
        let client = tower_http::trace::TraceLayer::new_for_http()
            .make_span_with(|_request: &Request<_>| tracing::debug_span!("http-request"))
            .on_request(|request: &Request<_>, _span: &Span| {
                println!(
                    "started {} {} ({:?})",
                    request.method(),
                    request.uri().path(),
                    request.headers()
                );
            })
            .on_response(|_response: &Response<_>, latency: Duration, _span: &Span| {
                println!("response generated in {:?}", latency)
            })
            .on_body_chunk(|chunk: &Bytes, _latency: Duration, _span: &Span| {
                println!("receiving {} bytes", chunk.len());
                hexdump::hexdump(chunk);
            })
            .on_eos(
                |_trailers: Option<&HeaderMap>, stream_duration: Duration, _span: &Span| {
                    println!("stream closed after {:?}", stream_duration)
                },
            )
            .on_failure(
                |_error: ServerErrorsFailureClass, _latency: Duration, _span: &Span| {
                    println!("something went wrong")
                },
            )
            .layer(client);
        fn request_map_fun<B>(request: Request<B>) -> Request<DebugBody<B>>
        where
            B: http_body::Body + Clone,
        {
            let (parts, body) = request.into_parts();

            Request::from_parts(parts, DebugBody::new(body))
        }
        let client = tower::util::MapRequestLayer::new(request_map_fun).layer(client);

        octocrab::OctocrabBuilder::new_empty()
            .with_service(client)
            .with_auth(AuthState::None)
            .build()
            .unwrap()
    };

    let config = config::from_jj(&jj, async || {
        let user = crab
            .current()
            .user()
            .await
            .context(String::from("Get current user from github"))?;
        Ok(user.login)
    })
    .await
    .context(String::from("Read configuration"))?;
    let mut gh = jj_spr::github::GitHub::new(config.clone(), crab);

    match cli.command {
        Commands::Fetch(opts) => commands::fetch::fetch(opts, &mut jj, &mut gh, &config).await?,
        Commands::Adopt(opts) => commands::adopt::adopt(opts, &mut jj, &mut gh, &config).await?,
        Commands::Push(opts) => commands::push::push(&mut jj, &mut gh, &config, opts).await?,
        Commands::Sync(opts) => commands::sync::sync(&mut jj, &mut gh, &config, opts).await?,
        Commands::Detach(opts) => commands::detach::detach(&mut jj, &config, opts).await?,
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
            jj_spr::output::output(&jj_spr::config::icons::Icons::default().stop, message)?;
        }
        std::process::exit(1);
    }

    Ok(())
}
