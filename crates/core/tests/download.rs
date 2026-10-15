//! Resumable downloads against a scripted local HTTP server.

use belka_core::client::{build_http_client, download, file_body};
use belka_core::Progress;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// What the server does for one request.
enum Reply {
    /// 200 with the whole body; with `cut`, the connection drops after that many bytes.
    Full {
        cut: Option<usize>,
    },
    /// 206 from the requested offset if a Range header was sent, else 200.
    Ranged,
    /// Sends the headers and `send` bytes, then stalls forever.
    Stall {
        send: usize,
    },
    Status(u16),
}

struct Server {
    url: String,
    hits: Arc<AtomicUsize>,
}

/// Serves `content`; the n-th request (0-based) is answered by `script(n)`.
async fn serve(content: &'static [u8], script: fn(usize) -> Reply) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let n = counter.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(handle(stream, content, script(n)));
        }
    });
    Server { url, hits }
}

async fn read_head(stream: &mut TcpStream) -> String {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).await.unwrap() == 0 {
            break;
        }
        head.push(byte[0]);
    }
    String::from_utf8_lossy(&head).to_ascii_lowercase()
}

async fn handle(mut stream: TcpStream, content: &'static [u8], reply: Reply) {
    let head = read_head(&mut stream).await;
    let range_start = head
        .lines()
        .find_map(|l| l.strip_prefix("range: bytes="))
        .and_then(|r| r.trim_end_matches('-').parse::<usize>().ok());

    let len = content.len();
    let (status, body, extra, cut) = match reply {
        Reply::Full { cut } => ("200 OK".to_string(), content, String::new(), cut),
        Reply::Ranged => match range_start {
            Some(start) => (
                "206 Partial Content".to_string(),
                &content[start..],
                format!("Content-Range: bytes {start}-{}/{len}\r\n", len - 1),
                None,
            ),
            None => ("200 OK".to_string(), content, String::new(), None),
        },
        Reply::Stall { send } => ("200 OK".to_string(), content, String::new(), Some(send)),
        Reply::Status(code) => (format!("{code} Nope"), &b""[..], String::new(), None),
    };
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await.unwrap();
    let sent = cut.unwrap_or(body.len()).min(body.len());
    let _ = stream.write_all(&body[..sent]).await;
    let _ = stream.flush().await;
    if matches!(reply, Reply::Stall { .. }) {
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

const CONTENT: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";

async fn fetch(url: &str, dest: &Path, progress: &Progress) -> anyhow::Result<u64> {
    let client = build_http_client().unwrap();
    download("test", dest, progress, || async { Ok(client.get(url)) }).await
}

fn part_of(dest: &Path) -> std::path::PathBuf {
    dest.with_file_name("out.bin.part")
}

#[tokio::test]
async fn downloads_and_reports_progress() {
    let server = serve(CONTENT, |_| Reply::Full { cut: None }).await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("out.bin");
    let progress = Progress::new();

    let size = fetch(&server.url, &dest, &progress).await.unwrap();

    assert_eq!(size, CONTENT.len() as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), CONTENT);
    assert_eq!((progress.done(), progress.total()), (size, size));
    assert!(!part_of(&dest).exists());
}

#[tokio::test]
async fn resumes_after_a_dropped_connection() {
    let server = serve(CONTENT, |n| match n {
        0 => Reply::Full { cut: Some(30) },
        _ => Reply::Ranged,
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("out.bin");

    fetch(&server.url, &dest, &Progress::new()).await.unwrap();

    assert_eq!(std::fs::read(&dest).unwrap(), CONTENT);
    assert_eq!(server.hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn restarts_when_the_server_ignores_range() {
    let server = serve(CONTENT, |n| match n {
        0 => Reply::Full { cut: Some(30) },
        _ => Reply::Full { cut: None },
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("out.bin");

    fetch(&server.url, &dest, &Progress::new()).await.unwrap();

    assert_eq!(std::fs::read(&dest).unwrap(), CONTENT);
}

#[tokio::test]
async fn failure_keeps_the_existing_file() {
    let server = serve(CONTENT, |_| Reply::Status(404)).await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("out.bin");
    std::fs::write(&dest, b"old").unwrap();

    let err = fetch(&server.url, &dest, &Progress::new())
        .await
        .unwrap_err();

    assert!(format!("{err:#}").contains("404"), "{err:#}");
    assert_eq!(std::fs::read(&dest).unwrap(), b"old");
    assert!(!part_of(&dest).exists());
}

#[tokio::test]
async fn cancelling_removes_partial_data() {
    let server = serve(CONTENT, |_| Reply::Stall { send: 20 }).await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("out.bin");
    let progress = Progress::new();

    let result = tokio::time::timeout(
        Duration::from_millis(500),
        fetch(&server.url, &dest, &progress),
    )
    .await;

    assert!(result.is_err(), "download should still be running");
    assert_eq!(progress.done(), 20);
    assert!(!part_of(&dest).exists());
    assert!(!dest.exists());
}

#[tokio::test]
async fn upload_body_reports_progress() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/up", listener.local_addr().unwrap());
    let received = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let head = read_head(&mut stream).await;
        let len: usize = head
            .lines()
            .find_map(|l| l.strip_prefix("content-length: "))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).await.unwrap();
        stream
            .write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
        body
    });

    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("in.bin");
    let data: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
    std::fs::write(&local, &data).unwrap();
    let progress = Progress::new();

    let (body, len) = file_body(&local, &progress).unwrap();
    let resp = build_http_client()
        .unwrap()
        .put(&url)
        .header("Content-Length", len)
        .body(body)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 201);
    assert_eq!(received.await.unwrap(), data);
    assert_eq!((progress.done(), progress.total()), (len, len));
}
