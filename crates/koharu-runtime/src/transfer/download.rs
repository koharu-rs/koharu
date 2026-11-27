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
use fs_err::tokio::{File, OpenOptions};
use futures::{StreamExt, TryStreamExt, stream};
use reqwest::{Client, Response, StatusCode, header};
use tokio::io::{AsyncSeekExt, AsyncWriteExt, BufWriter};
use tokio_util::sync::CancellationToken;

use super::{Cancelled, is_cancelled};

const RANGE_SIZE: u64 = 64 * 1024 * 1024;
const WRITE_BUFFER_SIZE: usize = 256 * 1024;
/// The one retry policy for transfers.
pub(crate) const BACKOFF: ExponentialBuilder = ExponentialBuilder::new()
    .with_min_delay(Duration::from_millis(250))
    .with_max_delay(Duration::from_secs(5))
    .with_max_times(3)
    .with_jitter();

/// Fetches in 64 MiB ranges over several connections when the server takes
/// ranges, each range retried on its own, and as one stream otherwise.
pub(super) async fn download(
    client: &Client,
    url: &str,
    destination: &Path,
    cancel: &CancellationToken,
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<()> {
    if cancel.is_cancelled() {
        return Err(Cancelled.into());
    }
    match size(client, url, cancel).await? {
        Some(total) => fetch_ranges(client, url, destination, total, cancel, on_progress).await,
        None => {
            let future = (|| fetch_stream(client, url, destination, cancel, on_progress))
                .retry(BACKOFF)
                .when(|error| !cancel.is_cancelled() && retryable(error))
                .notify(|error, delay| tracing::warn!(?delay, "retrying {url}: {error:#}"));
            tokio::select! {
                biased;
                () = cancel.cancelled() => Err(Cancelled.into()),
                result = future => result,
            }
        }
    }
}

/// The file's size, when the server answers a one-byte range with its part,
/// or `None` when it doesn't take ranges.
async fn size(client: &Client, url: &str, cancel: &CancellationToken) -> Result<Option<u64>> {
    if cancel.is_cancelled() {
        return Err(Cancelled.into());
    }
    let probe_fut = (|| async {
        let response = client
            .get(url)
            .header(header::RANGE, "bytes=0-0")
            .header(header::ACCEPT_ENCODING, "identity")
            .send()
            .await?;
        if response.status().is_server_error() || response.status() == StatusCode::TOO_MANY_REQUESTS
        {
            response.error_for_status()?;
            unreachable!("server error or 429 always produces an error");
        }
        Ok::<_, anyhow::Error>(response)
    })
    .retry(BACKOFF)
    .when(|error| !cancel.is_cancelled() && retryable(error));
    let probe = tokio::select! {
        biased;
        () = cancel.cancelled() => return Err(Cancelled.into()),
        result = probe_fut => result?,
    };
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
    client: &Client,
    url: &str,
    destination: &Path,
    cancel: &CancellationToken,
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<()> {
    if cancel.is_cancelled() {
        return Err(Cancelled.into());
    }
    let response = client
        .get(url)
        .header(header::ACCEPT_ENCODING, "identity")
        .send()
        .await?
        .error_for_status()?;
    let total = response.content_length().unwrap_or(0);
    on_progress(0, total);
    let mut output = BufWriter::with_capacity(WRITE_BUFFER_SIZE, File::create(destination).await?);
    let mut completed = 0u64;
    write(response, &mut output, cancel, |delta| {
        completed += delta;
        on_progress(completed, total);
    })
    .await?;
    Ok(())
}

async fn fetch_ranges(
    client: &Client,
    url: &str,
    destination: &Path,
    total: u64,
    cancel: &CancellationToken,
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<()> {
    if cancel.is_cancelled() {
        return Err(Cancelled.into());
    }
    File::create(destination).await?.set_len(total).await?;
    on_progress(0, total);
    let completed = Arc::new(AtomicU64::new(0));
    let ranges = (0..total)
        .step_by(RANGE_SIZE as usize)
        .map(|start| (start, (start + RANGE_SIZE).min(total) - 1));
    let run = stream::iter(ranges)
        .map(|(start, end)| {
            let completed = Arc::clone(&completed);
            async move {
                let retry_fut = (|| {
                    let completed = Arc::clone(&completed);
                    async move {
                        let mut range_written = 0u64;
                        let result = fetch_range(
                            client,
                            url,
                            destination,
                            total,
                            start,
                            end,
                            cancel,
                            |delta| {
                                range_written += delta;
                                let new_completed =
                                    completed.fetch_add(delta, Ordering::Relaxed) + delta;
                                on_progress(new_completed.min(total), total);
                            },
                        )
                        .await;
                        if result.is_err() && range_written > 0 {
                            completed.fetch_sub(range_written, Ordering::Relaxed);
                        }
                        result
                    }
                })
                .retry(BACKOFF)
                .when(|error| !cancel.is_cancelled() && retryable(error))
                .notify(|error, delay| {
                    tracing::warn!(start, end, ?delay, "retrying a range of {url}: {error:#}");
                });
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => Err(Cancelled.into()),
                    result = retry_fut => result,
                }
            }
        })
        .buffer_unordered(connections())
        .try_collect::<()>();
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(Cancelled.into()),
        result = run => result,
    }
}

#[allow(clippy::too_many_arguments)]
async fn fetch_range(
    client: &Client,
    url: &str,
    destination: &Path,
    total: u64,
    start: u64,
    end: u64,
    cancel: &CancellationToken,
    mut on_chunk: impl FnMut(u64),
) -> Result<()> {
    if cancel.is_cancelled() {
        return Err(Cancelled.into());
    }
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

    let mut file = OpenOptions::new().write(true).open(destination).await?;
    file.seek(std::io::SeekFrom::Start(start)).await?;
    let mut file = BufWriter::with_capacity(WRITE_BUFFER_SIZE, file);
    let received = write(response, &mut file, cancel, &mut on_chunk).await?;
    ensure!(
        received == end - start + 1,
        "a range ended after {received} bytes"
    );
    Ok(())
}

/// Writes `response`'s body to `output`, passing each chunk's byte delta to
/// `on_chunk`, and returns how many bytes it wrote.
async fn write(
    response: Response,
    output: &mut BufWriter<File>,
    cancel: &CancellationToken,
    mut on_chunk: impl FnMut(u64),
) -> Result<u64> {
    let mut body = response.bytes_stream();
    let mut length = 0;
    loop {
        let next = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(Cancelled.into()),
            chunk = body.next() => chunk,
        };
        let Some(bytes) = next else { break };
        let bytes = bytes?;
        output.write_all(&bytes).await?;
        let n = bytes.len() as u64;
        length += n;
        on_chunk(n);
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

/// Network failures and server errors are retried; cancellation and client
/// errors such as a missing file aren't.
fn retryable(error: &anyhow::Error) -> bool {
    if is_cancelled(error) {
        return false;
    }
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

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicUsize, Ordering as AtomicOrdering},
        time::Instant,
    };

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    #[test]
    fn retryable_excludes_cancelled() {
        let err: anyhow::Error = Cancelled.into();
        assert!(!retryable(&err));
        let wrapped = err.context("outer");
        assert!(!retryable(&wrapped));
    }

    #[tokio::test]
    async fn cancel_stream_aborts_immediately() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            // Probe returns 200 OK (no ranges), then second request streams partial body and stalls.
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\nhello")
                .await;
            let (mut socket2, _) = listener.accept().await.unwrap();
            let _ = socket2.read(&mut buf).await;
            let _ = socket2
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\nhello")
                .await;
            tokio::time::sleep(Duration::from_secs(30)).await;
        });

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("stream.bin");
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel_clone.cancel();
        });

        let client = Client::new();
        let url = format!("http://{addr}/stream.bin");
        let started = Instant::now();
        let result = download(&client, &url, &dest, &cancel, &|_, _| {}).await;
        assert!(is_cancelled(&result.unwrap_err()));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn cancel_range_aborts_immediately() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            // 1. Range probe returns 206 Partial Content for 1024-byte file.
            let (mut s1, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = s1.read(&mut buf).await;
            let _ = s1
                .write_all(
                    b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-0/1024\r\nContent-Length: 1\r\n\r\nx",
                )
                .await;

            // 2. Range 0-1023 stalls after sending headers + 4 bytes.
            let (mut s2, _) = listener.accept().await.unwrap();
            let _ = s2.read(&mut buf).await;
            let _ = s2
                .write_all(
                    b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-1023/1024\r\nContent-Length: 1024\r\n\r\nabcd",
                )
                .await;
            tokio::time::sleep(Duration::from_secs(30)).await;
        });

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("range.bin");
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel_clone.cancel();
        });

        let client = Client::new();
        let url = format!("http://{addr}/range.bin");
        let started = Instant::now();
        let result = download(&client, &url, &dest, &cancel, &|_, _| {}).await;
        assert!(is_cancelled(&result.unwrap_err()));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn cancel_during_backoff_aborts_without_retrying() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_clone = Arc::clone(&hits);

        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                hits_clone.fetch_add(1, AtomicOrdering::Relaxed);
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let _ = socket
                    .write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n")
                    .await;
            }
        });

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("retry.bin");
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            cancel_clone.cancel();
        });

        let client = Client::new();
        let url = format!("http://{addr}/retry.bin");
        let started = Instant::now();
        let result = download(&client, &url, &dest, &cancel, &|_, _| {}).await;
        assert!(is_cancelled(&result.unwrap_err()));
        assert!(started.elapsed() < Duration::from_millis(250));
        assert_eq!(hits.load(AtomicOrdering::Relaxed), 1);
    }
}
