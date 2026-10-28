use std::{sync::OnceLock, time::Duration};

use anyhow::{Context, Result, ensure};
use backon::ExponentialBuilder;
use reqwest::{Client, redirect};
use serde::{Deserialize, Serialize};

const USER_AGENT: &str = concat!("koharu/", env!("CARGO_PKG_VERSION"));

/// `[http]`: the shared client's timeouts, in seconds, and how many times a
/// download retries a failed request.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct Config {
    connect_timeout: u64,
    read_timeout: u64,
    max_retries: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            connect_timeout: 20,
            read_timeout: 300,
            max_retries: 3,
        }
    }
}

impl koharu_config::Section for Config {
    const NAME: &'static str = "http";
}

/// The app's one HTTP client: its user agent, the proxies the OS and the
/// environment set, and its timeouts, read when it's first built.
pub(crate) fn http() -> Result<Client> {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }
    let client = build(redirect::Policy::default())?;
    Ok(CLIENT.get_or_init(|| client).clone())
}

/// A client that doesn't follow redirects, for reading where the Hugging Face
/// Hub sends a file and what it says the file's hash is.
pub(crate) fn http_without_redirects() -> Result<Client> {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }
    let client = build(redirect::Policy::none())?;
    Ok(CLIENT.get_or_init(|| client).clone())
}

fn build(redirects: redirect::Policy) -> Result<Client> {
    let config = config()?;
    Client::builder()
        .user_agent(USER_AGENT)
        .redirect(redirects)
        .connect_timeout(Duration::from_secs(config.connect_timeout))
        // A response that stops arriving fails instead of hanging.
        .read_timeout(Duration::from_secs(config.read_timeout))
        .http2_adaptive_window(true)
        .tcp_keepalive(Duration::from_secs(60))
        .build()
        .context("couldn't build the HTTP client")
}

/// The one retry policy for downloads.
pub(crate) fn backoff() -> Result<ExponentialBuilder> {
    Ok(ExponentialBuilder::default()
        .with_min_delay(Duration::from_millis(250))
        .with_max_delay(Duration::from_secs(5))
        .with_max_times(config()?.max_retries)
        .with_jitter())
}

fn config() -> Result<Config> {
    let config: Config = koharu_config::get()?;
    ensure!(
        config.connect_timeout > 0 && config.read_timeout > 0,
        "[http] timeouts must be at least one second"
    );
    Ok(config)
}
