//! Streaming of cloud media. The page plays a video or audio file from a
//! small HTTP server on 127.0.0.1 while the file downloads: playback starts
//! at once and seeking fetches only the part needed. The player asks for
//! byte ranges, which are read from the cloud in aligned chunks, kept for a
//! while and read ahead.
//!
//! A URI scheme of the webview would need no port, but Android WebView
//! applies the requested range to the scheme's response itself, as if the
//! response were the whole file, so a scheme cannot serve ranges there.
//! Other apps on the device can reach the port too: the URL carries a random
//! token, and only the file being previewed and the one handed to an
//! external player (VLC and the like, for what the webview cannot play) are
//! served.

use anyhow::{anyhow, bail, Context, Result};
use belka_core::StorageProvider;
use bytes::Bytes;
use std::collections::VecDeque;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::OnceCell;
use tracing::{debug, warn};

/// The unit the cloud is read in.
const CHUNK: u64 = 2 * 1024 * 1024;
/// Chunks read ahead of the one being sent.
const READ_AHEAD: u64 = 2;
/// Chunks kept per stream: the read-ahead plus a little to seek back.
const KEEP: usize = 8;
/// Requests of the player are a line and a few headers.
const MAX_HEAD: usize = 16 * 1024;
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
/// An external player counts as playing this long after its last request,
/// so a pause between requests does not stop the stream.
#[cfg(mobile)]
const PLAYER_IDLE: Duration = Duration::from_secs(60);

/// The files being streamed and the tokens of their URLs.
#[derive(Default)]
struct Slots {
    /// The file shown in the preview.
    preview: Option<(String, Arc<Stream>)>,
    /// The file last handed to an external player; it outlives the preview.
    player: Option<(String, Arc<Stream>)>,
}

impl Slots {
    fn find(&self, token: &str) -> Option<Arc<Stream>> {
        [&self.preview, &self.player]
            .into_iter()
            .flatten()
            .find(|(t, _)| t == token)
            .map(|(_, stream)| stream.clone())
    }
}

type Current = Arc<Mutex<Slots>>;

#[derive(Default)]
pub struct Streams {
    /// The server starts with the first stream and runs while the app does.
    port: OnceCell<u16>,
    current: Current,
}

struct Stream {
    provider: Arc<dyn StorageProvider>,
    path: String,
    size: u64,
    mime: &'static str,
    chunks: Mutex<VecDeque<(u64, Arc<OnceCell<Bytes>>)>>,
    /// Requests being answered.
    connections: AtomicUsize,
    /// When the last request ended.
    last_used: Mutex<Instant>,
}

impl Streams {
    /// Starts streaming `path` for the preview, in place of the file it
    /// streamed before, and returns the URL.
    pub async fn open(
        &self,
        provider: Arc<dyn StorageProvider>,
        path: &str,
        size: u64,
    ) -> Result<String> {
        let (token, url, stream) = self.start(provider, path, size).await?;
        self.current.lock().unwrap().preview = Some((token, stream));
        Ok(url)
    }

    /// Starts streaming `path` for an external player, in place of the file
    /// handed to one before, and returns the URL.
    pub async fn open_player(
        &self,
        provider: Arc<dyn StorageProvider>,
        path: &str,
        size: u64,
    ) -> Result<String> {
        let (token, url, stream) = self.start(provider, path, size).await?;
        self.current.lock().unwrap().player = Some((token, stream));
        Ok(url)
    }

    /// Reads the first chunk, which a player asks for anyway, to make sure
    /// the cloud can read parts of files.
    async fn start(
        &self,
        provider: Arc<dyn StorageProvider>,
        path: &str,
        size: u64,
    ) -> Result<(String, String, Arc<Stream>)> {
        let stream = Arc::new(Stream {
            provider,
            path: path.to_string(),
            size,
            mime: mime_type(path),
            chunks: Mutex::new(VecDeque::new()),
            connections: AtomicUsize::new(0),
            last_used: Mutex::new(Instant::now()),
        });
        if size > 0 {
            stream.chunk(0).await?;
        }
        let port = *self.port.get_or_try_init(|| self.listen()).await?;
        let mut token = [0u8; 16];
        getrandom::fill(&mut token).map_err(|err| anyhow!("No random source: {err}"))?;
        let token: String = token.iter().map(|b| format!("{b:02x}")).collect();
        let url = format!("http://127.0.0.1:{port}/{token}");
        Ok((token, url, stream))
    }

    /// Stops the stream of the preview.
    pub fn close(&self) {
        self.current.lock().unwrap().preview.take();
    }

    /// The name of the file an external player is streaming, while it is:
    /// Android keeps the app running for it under a notification.
    #[cfg(mobile)]
    pub fn player_activity(&self) -> Option<String> {
        let slots = self.current.lock().unwrap();
        let (_, stream) = slots.player.as_ref()?;
        let playing = stream.connections.load(Ordering::Relaxed) > 0
            || stream.last_used.lock().unwrap().elapsed() < PLAYER_IDLE;
        playing.then(|| {
            let name = stream.path.rsplit('/').next().unwrap_or_default();
            name.to_string()
        })
    }

    async fn listen(&self) -> Result<u16> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .context("Failed to start the media server")?;
        let port = listener.local_addr()?.port();
        let current = self.current.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((socket, _)) => {
                        let current = current.clone();
                        tauri::async_runtime::spawn(async move {
                            // The player drops connections it no longer needs.
                            if let Err(err) = serve(socket, &current).await {
                                debug!("Media request ended: {err:#}");
                            }
                        });
                    }
                    Err(err) => {
                        warn!("Media server cannot accept a connection: {err}");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        });
        Ok(port)
    }
}

/// Answers one request and closes the connection.
async fn serve(mut socket: TcpStream, current: &Current) -> Result<()> {
    let head = tokio::time::timeout(HEAD_TIMEOUT, read_head(&mut socket))
        .await
        .context("no request")??;
    let Some(request) = parse_head(&head) else {
        return reply(&mut socket, "400 Bad Request").await;
    };
    let stream = current.lock().unwrap().find(request.token);
    let Some(stream) = stream else {
        return reply(&mut socket, "404 Not Found").await;
    };
    let _request = InUse::new(&stream);
    if request.method != "GET" && request.method != "HEAD" {
        return reply(&mut socket, "405 Method Not Allowed").await;
    }
    let size = stream.size;
    let (status, range) = match request.range {
        None => ("200 OK", size.checked_sub(1).map(|last| (0, last))),
        Some(value) => match parse_range(value, size) {
            Some(range) => ("206 Partial Content", Some(range)),
            None => {
                let head = format!(
                    "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{size}\r\n\
                     Content-Length: 0\r\nConnection: close\r\n\r\n"
                );
                return Ok(socket.write_all(head.as_bytes()).await?);
            }
        },
    };
    let len = range.map_or(0, |(first, last)| last - first + 1);
    debug!("Media {} {:?}: {status}", request.method, request.range);
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {}\r\nContent-Length: {len}\r\n\
         Accept-Ranges: bytes\r\nCache-Control: no-store\r\nConnection: close\r\n",
        stream.mime
    );
    if let (Some((first, last)), true) = (range, request.range.is_some()) {
        head.push_str(&format!("Content-Range: bytes {first}-{last}/{size}\r\n"));
    }
    head.push_str("\r\n");
    socket.write_all(head.as_bytes()).await?;
    if let (Some((first, last)), "GET") = (range, request.method) {
        stream.send(&mut socket, first, last).await?;
    }
    Ok(())
}

/// Counts a request being answered, for [`Streams::player_activity`].
struct InUse<'a>(&'a Stream);

impl<'a> InUse<'a> {
    fn new(stream: &'a Stream) -> Self {
        stream.connections.fetch_add(1, Ordering::Relaxed);
        Self(stream)
    }
}

impl Drop for InUse<'_> {
    fn drop(&mut self) {
        *self.0.last_used.lock().unwrap() = Instant::now();
        self.0.connections.fetch_sub(1, Ordering::Relaxed);
    }
}

async fn reply(socket: &mut TcpStream, status: &str) -> Result<()> {
    let head = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    Ok(socket.write_all(head.as_bytes()).await?)
}

/// Reads the request up to the empty line after the headers.
async fn read_head(socket: &mut TcpStream) -> Result<String> {
    let mut head = Vec::new();
    let mut buf = [0u8; 2048];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        if head.len() > MAX_HEAD {
            bail!("request too large");
        }
        let n = socket.read(&mut buf).await?;
        if n == 0 {
            bail!("connection closed");
        }
        head.extend_from_slice(&buf[..n]);
    }
    Ok(String::from_utf8_lossy(&head).into_owned())
}

struct Head<'a> {
    method: &'a str,
    token: &'a str,
    range: Option<&'a str>,
}

fn parse_head(head: &str) -> Option<Head<'_>> {
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next()?.split(' ');
    let method = request_line.next()?;
    let token = request_line.next()?.strip_prefix('/')?.split('?').next()?;
    let range = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("range"))
        .map(|(_, value)| value.trim());
    Some(Head {
        method,
        token,
        range,
    })
}

impl Stream {
    /// Sends bytes `first..=last`, reading ahead of the player.
    async fn send(self: &Arc<Self>, socket: &mut TcpStream, first: u64, last: u64) -> Result<()> {
        let mut pos = first;
        while pos <= last {
            let index = pos / CHUNK;
            self.read_ahead(index);
            let chunk = self.chunk(index).await?;
            let from = (pos - index * CHUNK) as usize;
            let to = ((last - index * CHUNK) as usize + 1).min(chunk.len());
            if from >= to {
                bail!("'{}' is shorter than its size", self.path);
            }
            socket.write_all(&chunk[from..to]).await?;
            pos += (to - from) as u64;
        }
        Ok(())
    }

    fn read_ahead(self: &Arc<Self>, index: u64) {
        for ahead in index + 1..=index + READ_AHEAD {
            if ahead * CHUNK >= self.size {
                break;
            }
            let stream = self.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(err) = stream.chunk(ahead).await {
                    warn!("Reading ahead '{}' failed: {err:#}", stream.path);
                }
            });
        }
    }

    /// Chunk `index`, read once however many requests want it at a time.
    async fn chunk(&self, index: u64) -> Result<Bytes> {
        let cell = {
            let mut chunks = self.chunks.lock().unwrap();
            match chunks.iter().position(|(i, _)| *i == index) {
                Some(pos) => {
                    // The most recently used chunk goes to the back.
                    let entry = chunks.remove(pos).unwrap();
                    chunks.push_back(entry.clone());
                    entry.1
                }
                None => {
                    let cell = Arc::new(OnceCell::new());
                    chunks.push_back((index, cell.clone()));
                    if chunks.len() > KEEP {
                        chunks.pop_front();
                    }
                    cell
                }
            }
        };
        let start = index * CHUNK;
        let len = CHUNK.min(self.size.saturating_sub(start));
        cell.get_or_try_init(|| self.provider.read_range(&self.path, start, len))
            .await
            .cloned()
    }
}

/// Parses `bytes=<first>-[<last>]` or `bytes=-<suffix>` into an inclusive
/// range within `size`. Only the first of several ranges is served.
fn parse_range(value: &str, size: u64) -> Option<(u64, u64)> {
    let spec = value.trim().strip_prefix("bytes=")?.split(',').next()?.trim();
    let (first, last) = spec.split_once('-')?;
    let (first, last) = if first.is_empty() {
        let suffix: u64 = last.parse().ok()?;
        (size.checked_sub(suffix.min(size))?, size.checked_sub(1)?)
    } else {
        let first: u64 = first.parse().ok()?;
        let last = match last {
            "" => u64::MAX,
            last => last.parse().ok()?,
        };
        (first, last.min(size.checked_sub(1)?))
    };
    (first <= last).then_some((first, last))
}

/// The types the preview or an external player plays; see `previewKind` in
/// the page. Android picks the apps to offer by them.
pub fn mime_type(path: &str) -> &'static str {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_lowercase());
    match ext.as_deref().unwrap_or_default() {
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "ogv" => "video/ogg",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "ts" | "m2ts" | "mts" => "video/mp2t",
        "wmv" => "video/x-ms-wmv",
        "flv" => "video/x-flv",
        "mpg" | "mpeg" | "vob" => "video/mpeg",
        "3gp" => "video/3gpp",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "wav" => "audio/wav",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ranges() {
        assert_eq!(parse_range("bytes=0-", 100), Some((0, 99)));
        assert_eq!(parse_range("bytes=10-19", 100), Some((10, 19)));
        assert_eq!(parse_range("bytes=90-200", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-30", 100), Some((70, 99)));
        assert_eq!(parse_range("bytes=-300", 100), Some((0, 99)));
        assert_eq!(parse_range("bytes=5-9, 20-29", 100), Some((5, 9)));
        assert_eq!(parse_range("bytes=100-", 100), None);
        assert_eq!(parse_range("bytes=0-", 0), None);
        assert_eq!(parse_range("items=0-1", 100), None);
    }

    #[test]
    fn parses_requests() {
        let head = parse_head("GET /abc?x=1 HTTP/1.1\r\nHost: 127.0.0.1\r\nRANGE: bytes=5-\r\n\r\n")
            .unwrap();
        assert_eq!(head.method, "GET");
        assert_eq!(head.token, "abc");
        assert_eq!(head.range, Some("bytes=5-"));
        assert!(parse_head("GET /abc HTTP/1.1\r\n\r\n").unwrap().range.is_none());
        assert!(parse_head("nonsense\r\n\r\n").is_none());
    }
}
