use std::{sync::OnceLock, time::Duration};

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

const USER_AGENT: &str = concat!("koharu/", env!("CARGO_PKG_VERSION"));

static HTTP_CLIENT: OnceLock<Client> = OnceLock::new();
static HTTP_CONFIG: OnceLock<Config> = OnceLock::new();

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct Config {
    connect_timeout: u64,
    pub(crate) read_timeout: u64,
    pub(crate) max_retries: u32,
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

pub(crate) fn config() -> Result<Config> {
    if let Some(config) = HTTP_CONFIG.get() {
        return Ok(*config);
    }
    let config = koharu_config::load::<Config>("http")?;
    let config = *config.read()?;
    Ok(*HTTP_CONFIG.get_or_init(|| config))
}

pub(crate) fn http() -> Result<Client> {
    if let Some(client) = HTTP_CLIENT.get() {
        return Ok(client.clone());
    }
    let config = config()?;
    let client = Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(config.connect_timeout.max(1)))
        .http2_adaptive_window(true)
        .tcp_keepalive(Duration::from_secs(60))
        .build()
        .context("failed to build HTTP client")?;
    Ok(HTTP_CLIENT.get_or_init(|| client).clone())
}
