//! HTTP plumbing shared by all providers: client setup, retries with
//! exponential backoff, uniform error messages, streamed bodies with progress
//! and resumable downloads.
//!
//! Uploads read from a [`Source`] and downloads write into a [`Sink`], so data
//! can be transformed on the way (encrypted, decrypted) without temporary files.

use crate::progress::Progress;
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use reqwest::{header, Body, Client, ClientBuilder, RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;
use std::fmt::Display;
use std::future::Future;
use std::io::{Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
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

/// The client with the base settings. It is built once and shared, so all
/// providers reuse one connection pool and one TLS setup.
pub fn build_http_client() -> Result<Client> {
    static SHARED: OnceLock<Client> = OnceLock::new();
    if let Some(client) = SHARED.get() {
        return Ok(client.clone());
    }
    let client = client_builder()
        .build()
        .context("Failed to initialize HTTP client")?;
    Ok(SHARED.get_or_init(|| client).clone())
}

/// Upper bound for a single upload request: a generous base plus the time
/// needed at a pessimistic 64 KiB/s. It only guards against hung connections.
pub fn upload_timeout(size: u64) -> Duration {
    Duration::from_secs(15 * 60 + size / (64 * 1024))
}

/// Size of the chunks a streamed body is read in; also the progress granularity.
const BODY_CHUNK: usize = 64 * 1024;

/// A reader handed out by a [`Source`].
pub type SourceReader = Box<dyn AsyncRead + Send + Sync + Unpin>;

/// Data to upload. Its size is known up front and it can be read from any
/// offset, so a failed upload is retried or resumed without temporary files.
pub trait Source: Send + Sync {
    fn len(&self) -> u64;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Reads the data from `offset` to the end.
    fn open_at(&self, offset: u64) -> Result<SourceReader>;

    /// When the data was last changed, for providers that keep the date of
    /// the original file instead of the time of the upload.
    fn modified(&self) -> Option<SystemTime> {
        None
    }
}

/// A local file.
pub struct FileSource {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
}

impl FileSource {
    pub fn new(path: &Path) -> Result<Self> {
        let meta = std::fs::metadata(path)
            .with_context(|| format!("Failed to read metadata of '{}'", path.display()))?;
        Ok(Self {
            path: path.to_owned(),
            len: meta.len(),
            modified: meta.modified().ok(),
        })
    }
}

impl Source for FileSource {
    fn len(&self) -> u64 {
        self.len
    }

    /// Fails if the file has changed since [`FileSource::new`]: a retried or
    /// resumed upload would mix two versions (and with encryption, use one
    /// nonce for different data).
    fn open_at(&self, offset: u64) -> Result<SourceReader> {
        let mut file = std::fs::File::open(&self.path)
            .with_context(|| format!("Failed to open '{}'", self.path.display()))?;
        let meta = file
            .metadata()
            .with_context(|| format!("Failed to read metadata of '{}'", self.path.display()))?;
        if meta.len() != self.len || meta.modified().ok() != self.modified {
            bail!(
                "'{}' changed while it was being uploaded",
                self.path.display()
            );
        }
        file.seek(SeekFrom::Start(offset))
            .with_context(|| format!("Failed to read '{}'", self.path.display()))?;
        // A file that grows meanwhile must not outrun the announced length.
        let rest = self.len.saturating_sub(offset);
        Ok(Box::new(tokio::fs::File::from_std(file).take(rest)))
    }

    fn modified(&self) -> Option<SystemTime> {
        self.modified
    }
}

/// Data in memory.
pub struct BytesSource(pub Bytes);

impl Source for BytesSource {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn open_at(&self, offset: u64) -> Result<SourceReader> {
        let start = (offset as usize).min(self.0.len());
        Ok(Box::new(std::io::Cursor::new(self.0.slice(start..))))
    }
}

/// A streaming request body with `source` from `offset` to the end. Sets
/// `progress` to `offset` of the source size and advances it as data is sent.
pub fn source_body(source: &dyn Source, offset: u64, progress: &Progress) -> Result<Body> {
    progress.set_total(source.len());
    progress.set_done(offset);
    Ok(reader_body(source.open_at(offset)?, progress.clone()))
}

/// A streaming body that adds every chunk it yields to `progress`.
fn reader_body<R>(reader: R, progress: Progress) -> Body
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

/// An in-memory body sent in chunks; `sent` gets the size of every chunk, so
/// progress advances smoothly.
pub fn bytes_body(data: Bytes, sent: impl Fn(u64) + Send + Sync + 'static) -> Body {
    let chunks: Vec<Bytes> = (0..data.len())
        .step_by(BODY_CHUNK)
        .map(|start| data.slice(start..data.len().min(start + BODY_CHUNK)))
        .collect();
    let stream = futures_util::stream::iter(chunks).map(move |chunk| {
        sent(chunk.len() as u64);
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
                let reason = format!("HTTP {}", resp.status());
                pause(op, &reason, delay, attempt, policy.max_attempts).await;
            }
            Ok(resp) => return Ok(resp),
            Err(err) if can_retry && is_transient(&err) => {
                let delay = backoff(attempt);
                let err = err.without_url();
                pause(op, &describe(&err), delay, attempt, policy.max_attempts).await;
            }
            // URLs can hold tokens (Mail.ru, Synology, bot tokens), and the
            // message ends up in logs, the task history and notifications.
            Err(err) => {
                return Err(
                    anyhow::Error::new(err.without_url()).context(format!("{op}: request failed"))
                )
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
        let err = anyhow::Error::new(err.without_url());
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
/// `progress` shows the attempt in progress.
pub async fn retry_transient<T, F, Fut>(op: &str, progress: &Progress, mut f: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<T, Failure>>,
{
    progress.set_attempt(1);
    let mut attempt = 1;
    loop {
        match f().await {
            Ok(value) => return Ok(value),
            Err(Failure::Transient(err)) if attempt < MAX_ATTEMPTS => {
                pause_before_retry(op, &err, attempt, progress).await;
            }
            Err(failure) => return Err(failure.into_inner()),
        }
        attempt += 1;
    }
}

/// Logs a transient failure and waits [`backoff`] before attempt `attempt + 1`,
/// which `progress` shows from now on.
pub async fn pause_before_retry(op: &str, err: &anyhow::Error, attempt: u32, progress: &Progress) {
    progress.set_attempt(attempt + 1);
    pause(
        op,
        &format!("{err:#}"),
        backoff(attempt),
        attempt,
        MAX_ATTEMPTS,
    )
    .await;
}

async fn pause(op: &str, reason: &str, delay: Duration, attempt: u32, max_attempts: u32) {
    warn!(
        "{op}: {reason}; retrying in {:.1}s (attempt {attempt}/{max_attempts})",
        delay.as_secs_f32()
    );
    tokio::time::sleep(delay).await;
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

/// The JSON body of a successful response. `what` describes the request in
/// error messages.
pub async fn json_response<T: DeserializeOwned>(resp: Response, what: impl Display) -> Result<T> {
    ensure_success(resp, &what)
        .await?
        .json()
        .await
        .map_err(reqwest::Error::without_url)
        .with_context(|| format!("{what}: invalid response"))
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

/// Where downloaded data goes.
#[async_trait]
pub trait Sink: Send {
    /// How many bytes of the remote data the sink holds: a download resumed
    /// after a network failure continues from here.
    fn position(&self) -> u64;

    async fn write(&mut self, data: &[u8]) -> Result<()>;

    /// Drops everything written so far: the download starts over.
    async fn reset(&mut self) -> Result<()>;
}

/// Writes into `<dest>.belka.part`, which replaces `dest` on [`PartSink::commit`].
/// The partial file is removed when the sink is dropped without committing,
/// which is how a cancelled download cleans up. A complete download that
/// cannot be moved into place stays there. The suffix is unusual so that a
/// real file such as `video.mkv.part` next to `video.mkv` is never taken.
pub struct PartSink {
    path: PathBuf,
    file: Option<tokio::fs::File>,
    len: u64,
    committed: bool,
}

impl PartSink {
    pub fn new(dest: &Path) -> Self {
        let mut name = dest.file_name().unwrap_or_default().to_os_string();
        name.push(".belka.part");
        Self {
            path: dest.with_file_name(name),
            file: None,
            len: 0,
            committed: false,
        }
    }

    pub async fn commit(mut self, dest: &Path) -> Result<()> {
        if self.file.is_none() {
            self.reset().await?;
        }
        let mut file = self.file.take().expect("opened above");
        file.flush()
            .await
            .with_context(|| format!("Failed to write '{}'", self.path.display()))?;
        drop(file);
        let mut delay = Duration::from_millis(100);
        let mut result = tokio::fs::rename(&self.path, dest).await;
        // Antivirus or the search indexer may briefly hold the file that has
        // just been closed.
        for _ in 0..5 {
            match &result {
                Err(e) if is_transient_lock(e) => {
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                    result = tokio::fs::rename(&self.path, dest).await;
                }
                _ => break,
            }
        }
        // The data is complete either way: keep it for the user instead of
        // deleting it on drop.
        self.committed = true;
        result.with_context(|| {
            format!(
                "Failed to move the download to '{}', the data is kept in '{}'",
                dest.display(),
                self.path.display()
            )
        })
    }
}

/// Access denied, sharing or lock violation: on Windows these mean another
/// process has the file open and usually go away soon.
fn is_transient_lock(e: &std::io::Error) -> bool {
    cfg!(windows) && matches!(e.raw_os_error(), Some(5 | 32 | 33))
}

#[async_trait]
impl Sink for PartSink {
    fn position(&self) -> u64 {
        self.len
    }

    async fn write(&mut self, data: &[u8]) -> Result<()> {
        if self.file.is_none() {
            self.reset().await?;
        }
        let file = self.file.as_mut().expect("opened above");
        file.write_all(data)
            .await
            .with_context(|| format!("Failed to write '{}'", self.path.display()))?;
        self.len += data.len() as u64;
        Ok(())
    }

    async fn reset(&mut self) -> Result<()> {
        self.file = None;
        let file = tokio::fs::File::create(&self.path)
            .await
            .with_context(|| format!("Failed to create '{}'", self.path.display()))?;
        self.file = Some(file);
        self.len = 0;
        Ok(())
    }
}

impl Drop for PartSink {
    fn drop(&mut self) {
        // Windows cannot delete a file that is still open.
        self.file = None;
        if !self.committed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Collects a download in memory.
#[derive(Default)]
pub struct MemorySink(pub Vec<u8>);

#[async_trait]
impl Sink for MemorySink {
    fn position(&self) -> u64 {
        self.0.len() as u64
    }

    async fn write(&mut self, data: &[u8]) -> Result<()> {
        self.0.extend_from_slice(data);
        Ok(())
    }

    async fn reset(&mut self) -> Result<()> {
        self.0.clear();
        Ok(())
    }
}

/// Downloads into `dest` with retries; see [`download_into`]. Data goes to
/// `<dest>.part`, which replaces `dest` only once complete.
pub async fn download<F, Fut>(op: &str, dest: &Path, progress: &Progress, request: F) -> Result<u64>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<RequestBuilder>>,
{
    let mut part = PartSink::new(dest);
    let size = download_into(op, &mut part, progress, request).await?;
    part.commit(dest).await?;
    debug!("Downloaded {size} bytes to {}", dest.display());
    Ok(size)
}

/// Downloads into `sink` with retries. `request` builds the GET request for
/// every attempt (so signatures, tokens and expiring links stay fresh) and must
/// not set an overall timeout. After a network failure the download resumes
/// from the sink's position with a `Range` request when the server supports it.
/// Returns the number of bytes received.
pub async fn download_into<F, Fut>(
    op: &str,
    sink: &mut dyn Sink,
    progress: &Progress,
    request: F,
) -> Result<u64>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<RequestBuilder>>,
{
    download_into_checked(op, sink, progress, request, |_, _| Ok(())).await
}

/// [`download_into`] where `check` looks at every successful response and
/// its expected total size before any data is written, so a server that
/// answers errors with status 200 (Synology DSM) cannot end up in the file.
pub async fn download_into_checked<F, Fut, C>(
    op: &str,
    sink: &mut dyn Sink,
    progress: &Progress,
    mut request: F,
    check: C,
) -> Result<u64>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<RequestBuilder>>,
    C: Fn(&Response, Option<u64>) -> std::result::Result<(), Failure>,
{
    sink.reset().await?;
    progress.set_done(0);
    progress.set_attempt(1);
    let mut attempt = 1;
    // A failure after new data does not use up an attempt, so a long
    // download survives a flaky connection as long as it moves on.
    let mut furthest = 0;
    loop {
        match download_attempt(sink, progress, request(), &check).await {
            Ok(size) => return Ok(size),
            Err(Failure::Transient(err)) if attempt < MAX_ATTEMPTS => {
                if sink.position() > furthest {
                    furthest = sink.position();
                    attempt = 0;
                }
                pause_before_retry(op, &err, attempt.max(1), progress).await;
            }
            Err(failure) => return Err(failure.into_inner()),
        }
        attempt += 1;
    }
}

async fn download_attempt<C>(
    sink: &mut dyn Sink,
    progress: &Progress,
    request: impl Future<Output = Result<RequestBuilder>>,
    check: &C,
) -> std::result::Result<u64, Failure>
where
    C: Fn(&Response, Option<u64>) -> std::result::Result<(), Failure>,
{
    let offset = sink.position();
    let mut req = request.await?;
    if offset > 0 {
        req = req.header(header::RANGE, format!("bytes={offset}-"));
    }
    let started = std::time::Instant::now();
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
    debug!(
        "{} answered {status} in {:.1}s, {} bytes",
        resp.url().host_str().unwrap_or_default(),
        started.elapsed().as_secs_f32(),
        resp.content_length()
            .map_or_else(|| "unknown".to_string(), |n| n.to_string())
    );
    let (mut pos, expected) = if offset > 0 && status == StatusCode::PARTIAL_CONTENT {
        match parse_content_range(&resp) {
            Some((start, total)) if start == offset => (offset, total),
            _ => {
                // Not the range we asked for: start over.
                sink.reset().await?;
                return Err(Failure::Transient(anyhow!(
                    "server returned an unexpected range"
                )));
            }
        }
    } else if status.is_success() {
        // A full response: either a fresh start or the server ignored Range.
        if offset > 0 {
            sink.reset().await?;
        }
        (0, resp.content_length())
    } else if offset > 0 && status == StatusCode::RANGE_NOT_SATISFIABLE {
        sink.reset().await?;
        return Err(Failure::Transient(anyhow!(
            "server rejected the resume range"
        )));
    } else {
        return Err(Failure::from_response(resp, "Download failed").await);
    };
    check(&resp, expected)?;
    if let Some(total) = expected {
        progress.set_total(total);
    }
    progress.set_done(pos);

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
            Ok(chunk) => chunk.map_err(|err| Failure::Transient(err.without_url().into()))?,
        };
        let Some(chunk) = chunk else { break };
        if pos == offset {
            debug!("First data after {:.1}s", started.elapsed().as_secs_f32());
        }
        sink.write(&chunk).await?;
        pos += chunk.len() as u64;
        progress.set_done(pos);
    }

    if let Some(expected) = expected {
        if expected != pos {
            return Err(Failure::Transient(anyhow!(
                "download truncated: received {pos} of {expected} bytes"
            )));
        }
    }
    Ok(pos)
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
