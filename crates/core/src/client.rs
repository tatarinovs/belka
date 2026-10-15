//! HTTP plumbing shared by all providers: client setup, retries with
//! exponential backoff, uniform error messages, streamed bodies with progress
//! and resumable downloads.

use crate::progress::Progress;
use anyhow::{anyhow, Context, Result};
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use reqwest::{header, Body, Client, ClientBuilder, RequestBuilder, Response, StatusCode};
use std::fmt::Display;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tracing::{debug, warn};

pub const USER_AGENT: &str = concat!("belka/", env!("CARGO_PKG_VERSION"));

/// Timeout for metadata / API calls that carry no file payload.
pub const API_TIMEOUT: Duration = Duration::from_secs(60);

/// A download is considered stalled after this long without receiving data.
const DOWNLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

pub const MAX_ATTEMPTS: u32 = 5;

/// Base client settings. Deliberately no overall request timeout: transfers of
/// large files can legitimately take hours. API calls set [`API_TIMEOUT`] per
/// request, uploads use [`upload_timeout`].
pub fn client_builder() -> ClientBuilder {
    Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(30))
        .tcp_keepalive(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(60))
}

pub fn build_http_client() -> Result<Client> {
    client_builder()
        .build()
        .context("Failed to initialize HTTP client")
}

/// Upper bound for a single upload request: a generous base plus the time
/// needed at a pessimistic 64 KiB/s. It only guards against hung connections.
pub fn upload_timeout(size: u64) -> Duration {
    Duration::from_secs(15 * 60 + size / (64 * 1024))
}

/// Size of the chunks a streamed body is read in; also the progress granularity.
const BODY_CHUNK: usize = 64 * 1024;

/// Opens a local file as a streaming request body (nothing is buffered in memory).
/// Resets `progress` to 0 of the file size and advances it as the body is sent.
pub fn file_body(path: &Path, progress: &Progress) -> Result<(Body, u64)> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("Failed to open '{}'", path.display()))?;
    let len = file
        .metadata()
        .with_context(|| format!("Failed to read metadata of '{}'", path.display()))?
        .len();
    progress.set_total(len);
    progress.set_done(0);
    Ok((
        reader_body(tokio::fs::File::from_std(file), progress.clone()),
        len,
    ))
}

/// A streaming body that adds every chunk it yields to `progress`.
pub fn reader_body<R>(reader: R, progress: Progress) -> Body
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let stream =
        futures_util::stream::try_unfold((reader, progress), |(mut reader, progress)| async move {
            let mut buf = BytesMut::with_capacity(BODY_CHUNK);
            if reader.read_buf(&mut buf).await? == 0 {
                return Ok::<_, std::io::Error>(None);
            }
            progress.add(buf.len() as u64);
            Ok(Some((buf.freeze(), (reader, progress))))
        });
    Body::wrap_stream(stream)
}

/// An in-memory body sent in chunks, so that `progress` advances smoothly.
pub fn bytes_body(data: Bytes, progress: Progress) -> Body {
    let chunks: Vec<Bytes> = (0..data.len())
        .step_by(BODY_CHUNK)
        .map(|start| data.slice(start..data.len().min(start + BODY_CHUNK)))
        .collect();
    let stream = futures_util::stream::iter(chunks).map(move |chunk| {
        progress.add(chunk.len() as u64);
        Ok::<_, std::io::Error>(chunk)
    });
    Body::wrap_stream(stream)
}

#[derive(Clone, Copy, Debug)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    /// Retry on 5xx responses. Some servers use 5xx for "not found" quirks.
    pub retry_server_errors: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: MAX_ATTEMPTS,
            retry_server_errors: true,
        }
    }
}

/// Sends a request with retries. `build` is called for every attempt, so
/// requests with streaming bodies (file uploads) are rebuilt from scratch.
pub async fn send<F>(op: &str, build: F) -> Result<Response>
where
    F: FnMut() -> Result<RequestBuilder>,
{
    send_with(op, RetryPolicy::default(), build).await
}

pub async fn send_with<F>(op: &str, policy: RetryPolicy, mut build: F) -> Result<Response>
where
    F: FnMut() -> Result<RequestBuilder>,
{
    let mut attempt = 1;
    loop {
        let can_retry = attempt < policy.max_attempts;
        match build()?.send().await {
            Ok(resp)
                if can_retry && is_retryable_status(resp.status(), policy.retry_server_errors) =>
            {
                let delay = retry_after(&resp).unwrap_or_else(|| backoff(attempt));
                warn!(
                    "{op}: HTTP {}, retrying in {:.1}s (attempt {attempt}/{})",
                    resp.status(),
                    delay.as_secs_f32(),
                    policy.max_attempts
                );
                tokio::time::sleep(delay).await;
            }
            Ok(resp) => return Ok(resp),
            Err(err) if can_retry && is_transient(&err) => {
                let delay = backoff(attempt);
                warn!(
                    "{op}: {}, retrying in {:.1}s (attempt {attempt}/{})",
                    describe(&err),
                    delay.as_secs_f32(),
                    policy.max_attempts
                );
                tokio::time::sleep(delay).await;
            }
            Err(err) => {
                return Err(anyhow::Error::new(err).context(format!("{op}: request failed")))
            }
        }
        attempt += 1;
    }
}

/// Outcome of a multi-step operation that decides itself what is retryable.
pub enum Failure {
    Transient(anyhow::Error),
    Fatal(anyhow::Error),
}

impl Failure {
    pub async fn from_response(resp: Response, what: impl Display) -> Self {
        let status = resp.status();
        let err = http_error(resp, what).await;
        if is_retryable_status(status, true) {
            Self::Transient(err)
        } else {
            Self::Fatal(err)
        }
    }

    pub fn into_inner(self) -> anyhow::Error {
        match self {
            Self::Transient(e) | Self::Fatal(e) => e,
        }
    }
}

impl From<anyhow::Error> for Failure {
    fn from(err: anyhow::Error) -> Self {
        Self::Fatal(err)
    }
}

impl From<reqwest::Error> for Failure {
    fn from(err: reqwest::Error) -> Self {
        let transient = is_transient(&err);
        let err = anyhow::Error::new(err);
        if transient {
            Self::Transient(err)
        } else {
            Self::Fatal(err)
        }
    }
}

impl From<std::io::Error> for Failure {
    fn from(err: std::io::Error) -> Self {
        Self::Fatal(err.into())
    }
}

/// Runs `f` until it succeeds, fails fatally, or runs out of attempts.
pub async fn retry_transient<T, F, Fut>(op: &str, mut f: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<T, Failure>>,
{
    let mut attempt = 1;
    loop {
        match f().await {
            Ok(value) => return Ok(value),
            Err(Failure::Transient(err)) if attempt < MAX_ATTEMPTS => {
                let delay = backoff(attempt);
                warn!(
                    "{op}: {err:#}; retrying in {:.1}s (attempt {attempt}/{MAX_ATTEMPTS})",
                    delay.as_secs_f32()
                );
                tokio::time::sleep(delay).await;
            }
            Err(failure) => return Err(failure.into_inner()),
        }
        attempt += 1;
    }
}

pub fn is_transient(err: &reqwest::Error) -> bool {
    err.is_timeout() || err.is_connect() || err.is_request() || err.is_body()
}

fn is_retryable_status(status: StatusCode, server_errors: bool) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::REQUEST_TIMEOUT
        || (server_errors && status.is_server_error() && status != StatusCode::NOT_IMPLEMENTED)
}

/// 1s, 2s, 4s, ... capped at 30s, plus up to 0.5s of jitter.
pub fn backoff(attempt: u32) -> Duration {
    let base =
        Duration::from_secs(1u64 << attempt.saturating_sub(1).min(5)).min(Duration::from_secs(30));
    let jitter_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::from(d.subsec_millis() % 500))
        .unwrap_or(0);
    base + Duration::from_millis(jitter_ms)
}

fn retry_after(resp: &Response) -> Option<Duration> {
    let secs: u64 = resp
        .headers()
        .get(header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs(secs.clamp(1, 120)))
}

/// Error message including the whole source chain (reqwest hides the root cause).
fn describe(err: &(dyn std::error::Error + 'static)) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        let msg = cause.to_string();
        if !text.contains(&msg) {
            text.push_str(": ");
            text.push_str(&msg);
        }
        source = cause.source();
    }
    text
}

/// Builds an error from a non-success response, including a trimmed body.
pub async fn http_error(resp: Response, what: impl Display) -> anyhow::Error {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    let body = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if body.is_empty() {
        anyhow!("{what}: HTTP {status}")
    } else {
        anyhow!("{what}: HTTP {status}: {}", truncate(&body, 400))
    }
}

pub async fn ensure_success(resp: Response, what: impl Display) -> Result<Response> {
    if resp.status().is_success() {
        Ok(resp)
    } else {
        Err(http_error(resp, what).await)
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => format!("{}…", &s[..idx]),
        None => s.to_string(),
    }
}

/// Downloads `url` into `dest`; see [`download`].
pub async fn download_to_file(client: &Client, url: &str, dest: &Path) -> Result<u64> {
    download(
        &format!("download {url}"),
        dest,
        &Progress::new(),
        || async { Ok(client.get(url)) },
    )
    .await
}

/// Downloads into `dest` with retries. `request` builds the GET request for
/// every attempt (so signatures, tokens and expiring links stay fresh) and must
/// not set an overall timeout. After a network failure the download resumes
/// with a `Range` request when the server supports it.
///
/// Data goes to `<dest>.part`, which replaces `dest` only once complete. The
/// partial file is removed on failure and when the future is dropped, which
/// is how a transfer is cancelled.
pub async fn download<F, Fut>(
    op: &str,
    dest: &Path,
    progress: &Progress,
    mut request: F,
) -> Result<u64>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<RequestBuilder>>,
{
    let part = PartFile::new(dest);
    match tokio::fs::remove_file(&part.path).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).with_context(|| format!("Failed to remove '{}'", part.path.display()))
        }
    }
    progress.set_done(0);

    let size = retry_transient(op, || download_attempt(&part.path, progress, request())).await?;
    tokio::fs::rename(&part.path, dest)
        .await
        .with_context(|| format!("Failed to move the download to '{}'", dest.display()))?;
    part.keep();
    debug!("Downloaded {size} bytes to {}", dest.display());
    Ok(size)
}

/// Removes the partial download when dropped, unless [`PartFile::keep`] is called.
struct PartFile {
    path: PathBuf,
    armed: bool,
}

impl PartFile {
    fn new(dest: &Path) -> Self {
        let mut name = dest.file_name().unwrap_or_default().to_os_string();
        name.push(".part");
        Self {
            path: dest.with_file_name(name),
            armed: true,
        }
    }

    fn keep(mut self) {
        self.armed = false;
    }
}

impl Drop for PartFile {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

async fn download_attempt(
    part: &Path,
    progress: &Progress,
    request: impl Future<Output = Result<RequestBuilder>>,
) -> std::result::Result<u64, Failure> {
    let offset = match tokio::fs::metadata(part).await {
        Ok(meta) => meta.len(),
        Err(_) => 0,
    };
    let mut req = request.await?;
    if offset > 0 {
        req = req.header(header::RANGE, format!("bytes={offset}-"));
    }
    let mut resp = match tokio::time::timeout(API_TIMEOUT, req.send()).await {
        Err(_) => {
            return Err(Failure::Transient(anyhow!(
                "no response within {}s",
                API_TIMEOUT.as_secs()
            )))
        }
        Ok(result) => result?,
    };

    let status = resp.status();
    let (mut pos, expected) = if offset > 0 && status == StatusCode::PARTIAL_CONTENT {
        match parse_content_range(&resp) {
            Some((start, total)) if start == offset => (offset, total),
            _ => {
                // Not the range we asked for: start over.
                reset_file(part).await?;
                return Err(Failure::Transient(anyhow!(
                    "server returned an unexpected range"
                )));
            }
        }
    } else if status.is_success() {
        // A full response: either a fresh start or the server ignored Range.
        (0, resp.content_length())
    } else if offset > 0 && status == StatusCode::RANGE_NOT_SATISFIABLE {
        reset_file(part).await?;
        return Err(Failure::Transient(anyhow!(
            "server rejected the resume range"
        )));
    } else {
        return Err(Failure::from_response(resp, "Download failed").await);
    };
    if let Some(total) = expected {
        progress.set_total(total);
    }
    progress.set_done(pos);

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(pos > 0)
        .truncate(pos == 0)
        .open(part)
        .await
        .with_context(|| format!("Failed to create '{}'", part.display()))?;

    loop {
        let chunk = match tokio::time::timeout(DOWNLOAD_IDLE_TIMEOUT, resp.chunk()).await {
            Err(_) => {
                return Err(Failure::Transient(anyhow!(
                    "download stalled: no data for {}s",
                    DOWNLOAD_IDLE_TIMEOUT.as_secs()
                )))
            }
            // A connection dropped mid-body surfaces as a "decode" error, which
            // `is_transient` does not cover; any failure here is worth a resume.
            Ok(chunk) => chunk.map_err(|err| Failure::Transient(err.into()))?,
        };
        let Some(chunk) = chunk else { break };
        file.write_all(&chunk)
            .await
            .with_context(|| format!("Failed to write '{}'", part.display()))?;
        pos += chunk.len() as u64;
        progress.set_done(pos);
    }
    file.flush()
        .await
        .with_context(|| format!("Failed to write '{}'", part.display()))?;

    if let Some(expected) = expected {
        if expected != pos {
            return Err(Failure::Transient(anyhow!(
                "download truncated: received {pos} of {expected} bytes"
            )));
        }
    }
    Ok(pos)
}

async fn reset_file(path: &Path) -> std::result::Result<(), Failure> {
    tokio::fs::File::create(path)
        .await
        .with_context(|| format!("Failed to reset '{}'", path.display()))?;
    Ok(())
}

/// Parses `Content-Range: bytes <start>-<end>/<total>` into (start, total).
fn parse_content_range(resp: &Response) -> Option<(u64, Option<u64>)> {
    let value = resp.headers().get(header::CONTENT_RANGE)?.to_str().ok()?;
    let (range, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let start = range.split_once('-')?.0.trim().parse().ok()?;
    Some((start, total.trim().parse().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_is_capped() {
        assert!(backoff(1) >= Duration::from_secs(1) && backoff(1) < Duration::from_secs(2));
        assert!(backoff(3) >= Duration::from_secs(4));
        assert!(backoff(20) <= Duration::from_millis(30_500));
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        assert_eq!(truncate("привет", 3), "при…");
        assert_eq!(truncate("abc", 5), "abc");
    }

    #[test]
    fn upload_timeout_scales_with_size() {
        assert_eq!(upload_timeout(0), Duration::from_secs(900));
        assert!(upload_timeout(10 * 1024 * 1024 * 1024) > Duration::from_secs(150_000));
    }
}
