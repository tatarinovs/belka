//! Copying a cloud file to another place. Inside one storage the cloud
//! copies it itself where it can ([`StorageProvider::server_copy`]).
//! Between storages the data goes through this computer: read from one
//! cloud in parts while it is written to the other, or, where the source
//! cannot read parts, through a temporary file.

use super::{RemoteFileInfo, StorageProvider};
use crate::client::{Source, SourceReader};
use crate::progress::Progress;
use anyhow::{Context as _, Result};
use bytes::Bytes;
use futures_util::future::BoxFuture;
use std::io;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{ready, Context, Poll};
use std::time::SystemTime;
use tokio::io::{AsyncRead, ReadBuf};
use tracing::debug;

/// The part a cloud file is read in while it is copied: big enough that
/// requests do not dominate, small enough to arrive within one request's
/// time limit on a slow line.
const PART: u64 = 4 * 1024 * 1024;

/// How a file was copied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyMethod {
    /// By the cloud itself; no data went over the network.
    Server,
    /// Read in parts and written at the same time.
    Stream,
    /// Downloaded into a temporary file, then uploaded.
    TempFile,
}

/// Copies `file` of `source` to `to` in `target`, replacing what is there.
/// `same_storage` says both are one storage (the same account), where the
/// cloud may copy by itself. `temp` is where the data waits if neither the
/// cloud nor streaming can do it; it is removed in any case. `progress`
/// follows the bytes; through a temporary file it runs twice.
pub async fn copy_file(
    source: &Arc<dyn StorageProvider>,
    file: &RemoteFileInfo,
    target: &dyn StorageProvider,
    to: &str,
    same_storage: bool,
    temp: &Path,
    progress: &Progress,
) -> Result<CopyMethod> {
    progress.set_total(file.size);
    if same_storage && source.server_copy(file, to).await? {
        progress.set_done(file.size);
        return Ok(CopyMethod::Server);
    }
    let modified = file.modified.map(SystemTime::from);
    // The first part shows whether the source can read parts at all.
    let first = if file.size == 0 {
        Ok(Bytes::new())
    } else {
        source.read_range(&file.path, 0, PART.min(file.size)).await
    };
    match first {
        Ok(first) => {
            let data = RemoteSource {
                provider: source.clone(),
                path: file.path.as_str().into(),
                len: file.size,
                modified,
                first,
            };
            target.upload(&data, to, progress).await?;
            Ok(CopyMethod::Stream)
        }
        Err(err) => {
            debug!(
                "Cannot read '{}' in parts ({err:#}); copying through a temporary file",
                file.path
            );
            let _cleanup = RemoveOnDrop(temp);
            source.download_file(&file.path, temp, progress).await?;
            // The copy keeps the date of the original where the target can.
            if let Some(modified) = modified {
                let dated = std::fs::File::options()
                    .write(true)
                    .open(temp)
                    .and_then(|f| f.set_modified(modified));
                if let Err(err) = dated {
                    debug!("Cannot date '{}': {err}", temp.display());
                }
            }
            target
                .upload_file(temp, to, progress)
                .await
                .with_context(|| format!("Failed to copy '{}'", file.path))?;
            Ok(CopyMethod::TempFile)
        }
    }
}

struct RemoveOnDrop<'a>(&'a Path);

impl Drop for RemoveOnDrop<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0);
    }
}

/// A cloud file as data to upload, read in parts with
/// [`StorageProvider::read_range`] as the upload needs them.
pub struct RemoteSource {
    provider: Arc<dyn StorageProvider>,
    path: Arc<str>,
    len: u64,
    modified: Option<SystemTime>,
    /// The beginning, already read; a retried upload starts from it again.
    first: Bytes,
}

impl Source for RemoteSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn open_at(&self, offset: u64) -> Result<SourceReader> {
        let offset = offset.min(self.len);
        let (buf, pos) = if offset < self.first.len() as u64 {
            (self.first.slice(offset as usize..), self.first.len() as u64)
        } else {
            (Bytes::new(), offset)
        };
        Ok(Box::new(RemoteReader {
            provider: self.provider.clone(),
            path: self.path.clone(),
            pos,
            end: self.len,
            buf,
            pending: Mutex::new(None),
        }))
    }

    fn modified(&self) -> Option<SystemTime> {
        self.modified
    }
}

/// Reads one part at a time, only when the upload asks for more: an upload
/// in parts (S3) reads up to the end of its part and stops, and reading
/// ahead would fetch data nobody sends.
struct RemoteReader {
    provider: Arc<dyn StorageProvider>,
    path: Arc<str>,
    /// Where the next part starts.
    pos: u64,
    end: u64,
    /// What is left of the part read last.
    buf: Bytes,
    /// The part being read. In a mutex only so that the reader is `Sync`,
    /// as uploads need; `poll_read` owns the reader and never waits on it.
    pending: Mutex<Option<BoxFuture<'static, Result<Bytes>>>>,
}

impl AsyncRead for RemoteReader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        out: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            if !this.buf.is_empty() {
                let n = this.buf.len().min(out.remaining());
                out.put_slice(&this.buf.split_to(n));
                return Poll::Ready(Ok(()));
            }
            if this.pos >= this.end {
                return Poll::Ready(Ok(()));
            }
            let pending = this.pending.get_mut().unwrap();
            let part = pending.get_or_insert_with(|| {
                let (provider, path) = (this.provider.clone(), this.path.clone());
                let (start, len) = (this.pos, PART.min(this.end - this.pos));
                Box::pin(async move { provider.read_range(&path, start, len).await })
            });
            let result = ready!(part.as_mut().poll(cx));
            *pending = None;
            match result {
                Ok(data) if data.is_empty() => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        format!("the cloud sent no data of '{}' at {}", this.path, this.pos),
                    )))
                }
                Ok(mut data) => {
                    // Never more than announced.
                    data.truncate((this.end - this.pos).min(data.len() as u64) as usize);
                    this.pos += data.len() as u64;
                    this.buf = data;
                }
                Err(err) => return Poll::Ready(Err(io::Error::other(format!("{err:#}")))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Sink;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::AsyncReadExt;

    /// One file of `size` bytes, readable in parts; counts the reads.
    struct Ranged {
        data: Vec<u8>,
        reads: AtomicUsize,
    }

    #[async_trait]
    impl StorageProvider for Ranged {
        fn name(&self) -> &str {
            "ranged"
        }
        async fn ensure_dir(&self, _: &str) -> Result<()> {
            Ok(())
        }
        async fn get_file_info(&self, _: &str) -> Result<Option<RemoteFileInfo>> {
            Ok(None)
        }
        async fn upload(&self, _: &dyn Source, _: &str, _: &Progress) -> Result<()> {
            unimplemented!()
        }
        async fn download_to(&self, _: &str, _: &mut dyn Sink, _: &Progress) -> Result<u64> {
            unimplemented!()
        }
        async fn read_range(&self, _: &str, start: u64, len: u64) -> Result<Bytes> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            let start = start as usize;
            let end = (start + len as usize).min(self.data.len());
            Ok(Bytes::copy_from_slice(&self.data[start..end]))
        }
        async fn delete(&self, _: &str, _: bool) -> Result<()> {
            Ok(())
        }
        async fn rename(&self, _: &str, _: &str, _: bool) -> Result<()> {
            Ok(())
        }
        async fn list_dir(&self, _: &str) -> Result<Vec<RemoteFileInfo>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn remote_source_reads_parts_on_demand() {
        let size = (PART * 2 + 10) as usize;
        let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let cloud = Arc::new(Ranged {
            data: data.clone(),
            reads: AtomicUsize::new(0),
        });
        let first = cloud.read_range("/f", 0, PART).await.unwrap();
        let source = RemoteSource {
            provider: cloud.clone(),
            path: "/f".into(),
            len: size as u64,
            modified: None,
            first,
        };

        let mut all = Vec::new();
        source
            .open_at(0)
            .unwrap()
            .read_to_end(&mut all)
            .await
            .unwrap();
        assert_eq!(all, data);
        // The first part was not read again.
        assert_eq!(cloud.reads.load(Ordering::Relaxed), 3);

        // A part of an upload in parts reads only what it takes.
        let before = cloud.reads.load(Ordering::Relaxed);
        let mut part = Vec::new();
        let offset = PART + 5;
        source
            .open_at(offset)
            .unwrap()
            .take(100)
            .read_to_end(&mut part)
            .await
            .unwrap();
        assert_eq!(part, data[offset as usize..offset as usize + 100]);
        assert_eq!(cloud.reads.load(Ordering::Relaxed), before + 1);
    }
}
