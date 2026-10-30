//! Transfers into the store, and the state of each for the app's list of
//! them.

use std::{
    path::Path,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use backon::Retryable;
use futures::{StreamExt, TryStreamExt, stream};
use reqwest::{Client, StatusCode, header};
use serde::Serialize;
use tokio::{
    io::{AsyncSeekExt, AsyncWriteExt, BufWriter},
    sync::broadcast::{self, error::RecvError},
};

use crate::network;

const RANGE_SIZE: u64 = 64 * 1024 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);
const WRITE_BUFFER_SIZE: usize = 256 * 1024;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static TRANSFERS: LazyLock<broadcast::Sender<Transfer>> =
    LazyLock::new(|| broadcast::channel(256).0);

/// One transfer's whole state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum State {
    Running,
    Finished,
    Failed(String),
}

/// A receiver of every transfer's state.
pub struct Transfers(broadcast::Receiver<Transfer>);

impl Transfers {
    /// The next state; a receiver that fell behind skips the oldest states it
    /// missed.
    pub async fn next(&mut self) -> Option<Transfer> {
        loop {
            match self.0.recv().await {
                Ok(transfer) => return Some(transfer),
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return None,
            }
        }
    }
}

#[must_use]
pub fn transfers() -> Transfers {
    Transfers(TRANSFERS.subscribe())
}

/// Fetches `url` to `destination`; a transfer that fails removes what it
/// wrote.
pub(crate) async fn fetch(url: &str, destination: &Path) -> Result<()> {
    let name = reqwest::Url::parse(url)
        .ok()
        .and_then(|url| {
            url.path_segments()?
                .next_back()
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "download".to_owned());
    let activity = Activity {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        name,
        completed: AtomicU64::new(0),
        total: AtomicU64::new(0),
    };
    activity.send(State::Running);
    let result = async { download(&activity, &network::http_client()?, url, destination).await };
    match result.await {
        Ok(()) => {
            activity.send(State::Finished);
            Ok(())
        }
        Err(error) => {
            let _ = tokio::fs::remove_file(destination).await;
            activity.send(State::Failed(format!("{error:#}")));
            Err(error)
        }
    }
}

/// A transfer in progress, and the last progress it reported.
struct Activity {
    id: u64,
    name: String,
    completed: AtomicU64,
    total: AtomicU64,
}

impl Activity {
    fn send(&self, state: State) {
        let _ = TRANSFERS.send(Transfer {
            id: self.id,
            name: self.name.clone(),
            completed: self.completed.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
            state,
        });
    }

    fn progress(&self, completed: u64, total: u64) {
        self.completed.store(completed, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
        self.send(State::Running);
    }
}

/// Fetches in 64 MiB ranges over several connections when the server takes
/// ranges, each range retried on its own, and as one stream otherwise.
async fn download(
    activity: &Activity,
    client: &Client,
    url: &str,
    destination: &Path,
) -> Result<()> {
    let probe = (|| async {
        Ok(client
            .get(url)
            .header(header::RANGE, "bytes=0-0")
            .header(header::ACCEPT_ENCODING, "identity")
            .send()
            .await?)
    })
    .retry(network::backoff()?)
    .when(retryable)
    .await?;
    let total = match probe.status() {
        StatusCode::PARTIAL_CONTENT => {
            let range = probe
                .headers()
                .get(header::CONTENT_RANGE)
                .context("the server took a range but sent no Content-Range")?
                .to_str()?;
            let (_, total) = range
                .split_once('/')
                .with_context(|| format!("invalid Content-Range `{range}`"))?;
            Some(total.parse::<u64>()?)
        }
        StatusCode::RANGE_NOT_SATISFIABLE
            if probe
                .headers()
                .get(header::CONTENT_RANGE)
                .is_some_and(|value| value == "bytes */0") =>
        {
            Some(0)
        }
        _ => {
            probe.error_for_status()?;
            None
        }
    };
    match total {
        Some(0) => {
            tokio::fs::File::create(destination).await?;
            Ok(())
        }
        Some(total) => fetch_ranges(activity, client, url, destination, total).await,
        None => {
            (|| fetch_stream(activity, client, url, destination))
                .retry(network::backoff()?)
                .when(retryable)
                .notify(|error, delay| tracing::warn!(?delay, "retrying {url}: {error:#}"))
                .await
        }
    }
}

async fn fetch_stream(
    activity: &Activity,
    client: &Client,
    url: &str,
    destination: &Path,
) -> Result<()> {
    let response = client
        .get(url)
        .header(header::ACCEPT_ENCODING, "identity")
        .send()
        .await?
        .error_for_status()?;
    let total = response.content_length().unwrap_or(0);
    let mut output = BufWriter::with_capacity(
        WRITE_BUFFER_SIZE,
        tokio::fs::File::create(destination).await?,
    );
    let mut body = response.bytes_stream();
    let mut completed = 0;
    let mut reported = Instant::now();
    while let Some(bytes) = body.next().await {
        let bytes = bytes?;
        output.write_all(&bytes).await?;
        completed += bytes.len() as u64;
        if reported.elapsed() >= PROGRESS_INTERVAL {
            activity.progress(completed, total);
            reported = Instant::now();
        }
    }
    output.flush().await?;
    activity.progress(completed, total);
    Ok(())
}

async fn fetch_ranges(
    activity: &Activity,
    client: &Client,
    url: &str,
    destination: &Path,
    total: u64,
) -> Result<()> {
    tokio::fs::File::create(destination)
        .await?
        .set_len(total)
        .await?;
    let completed = Arc::new(AtomicU64::new(0));
    let backoff = network::backoff()?;
    let ranges = (0..total)
        .step_by(RANGE_SIZE as usize)
        .map(|start| (start, (start + RANGE_SIZE).min(total) - 1));
    stream::iter(ranges)
        .map(|(start, end)| {
            let completed = Arc::clone(&completed);
            async move {
                (|| fetch_range(client, url, destination, total, start, end))
                    .retry(backoff)
                    .when(retryable)
                    .notify(|error, delay| {
                        tracing::warn!(start, end, ?delay, "retrying a range of {url}: {error:#}");
                    })
                    .await?;
                let length = end - start + 1;
                activity.progress(
                    completed.fetch_add(length, Ordering::Relaxed) + length,
                    total,
                );
                Ok::<(), anyhow::Error>(())
            }
        })
        .buffer_unordered(connections())
        .try_collect::<()>()
        .await
}

async fn fetch_range(
    client: &Client,
    url: &str,
    destination: &Path,
    total: u64,
    start: u64,
    end: u64,
) -> Result<()> {
    let response = client
        .get(url)
        .header(header::RANGE, format!("bytes={start}-{end}"))
        .header(header::ACCEPT_ENCODING, "identity")
        .send()
        .await?
        .error_for_status()?;
    let expected = format!("bytes {start}-{end}/{total}");
    let actual = response
        .headers()
        .get(header::CONTENT_RANGE)
        .context("a range came back without Content-Range")?
        .to_str()?;
    ensure!(actual == expected, "asked for {expected}, got {actual}");

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(destination)
        .await?;
    file.seek(std::io::SeekFrom::Start(start)).await?;
    let mut file = BufWriter::with_capacity(WRITE_BUFFER_SIZE, file);
    let mut body = response.bytes_stream();
    let mut received = 0;
    while let Some(bytes) = body.next().await {
        let bytes = bytes?;
        received += bytes.len() as u64;
        file.write_all(&bytes).await?;
    }
    file.flush().await?;
    ensure!(
        received == end - start + 1,
        "a range ended after {received} bytes"
    );
    Ok(())
}

fn connections() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(8)
        .saturating_mul(2)
        .clamp(8, 32)
}

/// Network failures and server errors are retried; a client error such as a
/// missing file isn't.
fn retryable(error: &anyhow::Error) -> bool {
    match error
        .chain()
        .find_map(|cause| cause.downcast_ref::<reqwest::Error>())
    {
        Some(error) => match error.status() {
            Some(status) => status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS,
            None => true,
        },
        None => true,
    }
}
