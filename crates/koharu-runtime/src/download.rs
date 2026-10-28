//! Downloads, checked against a pinned hash, and their progress events.

use std::{
    fs::File,
    io::Read,
    path::Path,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use backon::Retryable;
use futures::{StreamExt, TryStreamExt, stream};
use reqwest::{Client, StatusCode, header};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest as _, Sha256};
use tokio::{
    io::{AsyncSeekExt, AsyncWriteExt, BufWriter},
    sync::broadcast,
};

use crate::network;

const RANGE_SIZE: u64 = 64 * 1024 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);
const WRITE_BUFFER_SIZE: usize = 256 * 1024;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static EVENTS: LazyLock<broadcast::Sender<Event>> = LazyLock::new(|| broadcast::channel(256).0);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Event {
    Started {
        id: u64,
        name: String,
    },
    Progress {
        id: u64,
        name: String,
        completed: u64,
        total: u64,
    },
    Finished {
        id: u64,
    },
    Failed {
        id: u64,
        name: String,
        error: String,
    },
}

#[must_use]
pub fn subscribe() -> broadcast::Receiver<Event> {
    EVENTS.subscribe()
}

/// What a downloaded file must hash to, as lowercase hex.
#[derive(Clone, Debug)]
pub(crate) enum Digest {
    Sha256(String),
    /// Git's SHA-1 of the file as a blob, which is what the Hugging Face Hub
    /// gives for a file kept in git rather than LFS.
    GitSha1(String),
}

/// Downloads `url` to `destination` and checks it against `digest`; a file
/// that doesn't match is deleted and the download fails.
pub(crate) async fn fetch(url: &str, destination: &Path, digest: &Digest) -> Result<()> {
    let name = reqwest::Url::parse(url)
        .ok()
        .and_then(|url| {
            url.path_segments()?
                .next_back()
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "download".to_owned());
    let activity = Activity::start(name);
    let result = async {
        download(&activity, &network::http()?, url, destination).await?;
        verify(destination, digest).await
    }
    .await;
    activity.finish(destination, result).await
}

struct Activity {
    id: u64,
    name: String,
}

impl Activity {
    fn start(name: String) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let _ = EVENTS.send(Event::Started {
            id,
            name: name.clone(),
        });
        Self { id, name }
    }

    fn progress(&self, completed: u64, total: u64) {
        let _ = EVENTS.send(Event::Progress {
            id: self.id,
            name: self.name.clone(),
            completed,
            total,
        });
    }

    async fn finish(self, destination: &Path, result: Result<()>) -> Result<()> {
        match result {
            Ok(()) => {
                let _ = EVENTS.send(Event::Finished { id: self.id });
                Ok(())
            }
            Err(error) => {
                let _ = tokio::fs::remove_file(destination).await;
                let _ = EVENTS.send(Event::Failed {
                    id: self.id,
                    name: self.name,
                    error: format!("{error:#}"),
                });
                Err(error)
            }
        }
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

/// Hashes the finished file off the async threads; the ranges arrive out of
/// order, so it's hashed once whole.
async fn verify(path: &Path, digest: &Digest) -> Result<()> {
    let path = path.to_owned();
    let digest = digest.clone();
    tokio::task::spawn_blocking(move || {
        let mut file = File::open(&path)?;
        let (actual, expected) = match &digest {
            Digest::Sha256(expected) => (hash(Sha256::new(), &mut file)?, expected),
            Digest::GitSha1(expected) => {
                let mut sha = Sha1::new();
                sha.update(format!("blob {}\0", file.metadata()?.len()));
                (hash(sha, &mut file)?, expected)
            }
        };
        if !actual.eq_ignore_ascii_case(expected) {
            bail!("the download hashes to {actual}, not the pinned {expected}");
        }
        Ok(())
    })
    .await?
}

fn hash(mut hasher: impl sha2::Digest, file: &mut File) -> Result<String> {
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
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
