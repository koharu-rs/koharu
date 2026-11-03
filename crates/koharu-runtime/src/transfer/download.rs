//! Downloads a file in parallel ranges when the server takes them, and as one
//! stream otherwise.

use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use backon::{ExponentialBuilder, Retryable};
use futures::{StreamExt, TryStreamExt, stream};
use reqwest::{Client, Response, StatusCode, header};
use tokio::{
    fs::File,
    io::{AsyncSeekExt, AsyncWriteExt, BufWriter},
};

use super::Activity;

const RANGE_SIZE: u64 = 64 * 1024 * 1024;
const WRITE_BUFFER_SIZE: usize = 256 * 1024;
/// The one retry policy for transfers.
const BACKOFF: ExponentialBuilder = ExponentialBuilder::new()
    .with_min_delay(Duration::from_millis(250))
    .with_max_delay(Duration::from_secs(5))
    .with_max_times(3)
    .with_jitter();

/// Fetches in 64 MiB ranges over several connections when the server takes
/// ranges, each range retried on its own, and as one stream otherwise.
pub(super) async fn download(
    activity: &Activity,
    client: &Client,
    url: &str,
    destination: &Path,
) -> Result<()> {
    match size(client, url).await? {
        Some(total) => fetch_ranges(activity, client, url, destination, total).await,
        None => {
            (|| fetch_stream(activity, client, url, destination))
                .retry(BACKOFF)
                .when(retryable)
                .notify(|error, delay| tracing::warn!(?delay, "retrying {url}: {error:#}"))
                .await
        }
    }
}

/// The file's size, when the server answers a one-byte range with its part,
/// or `None` when it doesn't take ranges.
async fn size(client: &Client, url: &str) -> Result<Option<u64>> {
    let probe = (|| async {
        Ok(client
            .get(url)
            .header(header::RANGE, "bytes=0-0")
            .header(header::ACCEPT_ENCODING, "identity")
            .send()
            .await?)
    })
    .retry(BACKOFF)
    .when(retryable)
    .await?;
    if probe.status() != StatusCode::PARTIAL_CONTENT {
        probe.error_for_status()?;
        return Ok(None);
    }
    let range = probe
        .headers()
        .get(header::CONTENT_RANGE)
        .context("the server took a range but sent no Content-Range")?
        .to_str()?;
    let (_, total) = range
        .split_once('/')
        .with_context(|| format!("invalid Content-Range `{range}`"))?;
    Ok(Some(total.parse()?))
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
    let mut output = BufWriter::with_capacity(WRITE_BUFFER_SIZE, File::create(destination).await?);
    write(response, &mut output, |completed| {
        activity.progress(completed, total);
    })
    .await?;
    Ok(())
}

async fn fetch_ranges(
    activity: &Activity,
    client: &Client,
    url: &str,
    destination: &Path,
    total: u64,
) -> Result<()> {
    File::create(destination).await?.set_len(total).await?;
    let completed = Arc::new(AtomicU64::new(0));
    let ranges = (0..total)
        .step_by(RANGE_SIZE as usize)
        .map(|start| (start, (start + RANGE_SIZE).min(total) - 1));
    stream::iter(ranges)
        .map(|(start, end)| {
            let completed = Arc::clone(&completed);
            async move {
                (|| fetch_range(client, url, destination, total, start, end))
                    .retry(BACKOFF)
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
    let received = write(response, &mut file, |_| {}).await?;
    ensure!(
        received == end - start + 1,
        "a range ended after {received} bytes"
    );
    Ok(())
}

/// Writes `response`'s body to `output`, passing the bytes written so far to
/// `written` after each chunk, and returns how many it wrote.
async fn write(
    response: Response,
    output: &mut BufWriter<File>,
    mut written: impl FnMut(u64),
) -> Result<u64> {
    let mut body = response.bytes_stream();
    let mut length = 0;
    while let Some(bytes) = body.next().await {
        let bytes = bytes?;
        output.write_all(&bytes).await?;
        length += bytes.len() as u64;
        written(length);
    }
    output.flush().await?;
    Ok(length)
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
