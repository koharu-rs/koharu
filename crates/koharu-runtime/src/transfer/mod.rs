//! Transfers into the store, and the state of each for the app's list of
//! them.

mod activity;
mod download;

use std::{path::Path, sync::LazyLock, time::Duration};

use anyhow::{Context, Result};
use futures::{Stream, StreamExt};
use reqwest::Client;
use serde::Serialize;
use tokio::sync::{OnceCell, broadcast};
use tokio_stream::wrappers::BroadcastStream;

pub(crate) use activity::Activity;

/// The user agent every transfer sends.
pub(crate) const USER_AGENT: &str = concat!("koharu/", env!("CARGO_PKG_VERSION"));

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

static TRANSFERS: LazyLock<broadcast::Sender<Transfer>> =
    LazyLock::new(|| broadcast::channel(256).0);

/// One transfer's whole state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, specta::Type)]
pub struct Transfer {
    /// The same for every state of one transfer.
    pub id: u64,
    /// The file's name.
    pub name: String,
    pub completed: u64,
    /// 0 while the size isn't known.
    pub total: u64,
    pub state: State,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, specta::Type)]
pub enum State {
    Running,
    Finished,
    Failed(String),
}

/// Every transfer's states from now on; a receiver that falls behind skips
/// the oldest states it missed.
pub fn transfers() -> impl Stream<Item = Transfer> + Unpin {
    BroadcastStream::new(TRANSFERS.subscribe())
        .filter_map(|transfer| futures::future::ready(transfer.ok()))
}

/// Fetches `url` to `destination`, which names the transfer; a transfer that
/// fails removes what it wrote.
pub(crate) async fn fetch(url: &str, destination: &Path) -> Result<()> {
    let name = destination
        .file_name()
        .context("a transfer's destination has no name")?;
    let activity = Activity::start(name.to_string_lossy());
    let result =
        async { download::download(&activity, client().await?, url, destination).await }.await;
    if result.is_err() {
        let _ = fs_err::tokio::remove_file(destination).await;
    }
    activity.finish(result)
}

/// The client every transfer goes through, the Hub's included. A transfer
/// takes as long as the file does, so a response is never cut off for
/// arriving slowly.
pub(crate) async fn client() -> Result<&'static Client> {
    static CLIENT: OnceCell<Client> = OnceCell::const_new();
    CLIENT
        .get_or_try_init(|| async {
            Client::builder()
                .user_agent(USER_AGENT)
                .connect_timeout(CONNECT_TIMEOUT)
                .http2_adaptive_window(true)
                .tcp_keepalive(Duration::from_secs(60))
                .build()
                .context("couldn't build the HTTP client")
        })
        .await
}
