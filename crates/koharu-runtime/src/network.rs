use std::{sync::OnceLock, time::Duration};

use anyhow::{Context, Result, ensure};
use backon::ExponentialBuilder;
use reqwest::Client;
use serde::{Deserialize, Serialize};

const USER_AGENT: &str = concat!("koharu/", env!("CARGO_PKG_VERSION"));

/// `[http]`: the shared client's timeouts, in seconds, and how many times a
/// transfer retries a failed request.
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

/// The app's one HTTP client, which `koharu-llm`'s hosted providers send
/// through: Koharu's user agent, the proxies the OS and the environment set,
/// and `[http]`'s timeouts, read when it's first built. A response that stops
/// arriving fails after the read timeout; the client retries nothing, as a
/// generation request mustn't be replayed.
pub fn http_client() -> Result<Client> {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }
    let config = config()?;
    let client = Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(config.connect_timeout))
        .read_timeout(Duration::from_secs(config.read_timeout))
        .http2_adaptive_window(true)
        .tcp_keepalive(Duration::from_secs(60))
        .build()
        .context("couldn't build the HTTP client")?;
    Ok(CLIENT.get_or_init(|| client).clone())
}

/// The one retry policy for transfers.
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
