pub mod crypt;
pub mod google;
pub mod mailru;
pub mod s3;
pub mod synology;
pub mod webdav;
pub mod yandex;

use crate::client::{FileSource, MemorySink, PartSink, Sink, Source};
use crate::progress::Progress;
use crate::utils::path::{join_remote, split_remote};
use anyhow::{bail, Result};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use std::future::Future;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RemoteFileInfo {
    pub name: String,
    /// Normalized absolute remote path, e.g. `/Upload/file.zip`.
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    /// Lowercase hex MD5, set only when the provider guarantees it is a content hash.
    pub md5: Option<String>,
    pub modified: Option<DateTime<Utc>>,
}

impl RemoteFileInfo {
    /// A folder known only by its path.
    pub fn dir(path: String) -> Self {
        Self {
            name: base_name(&path).to_string(),
            path,
            is_dir: true,
            size: 0,
            md5: None,
            modified: None,
        }
    }
}

/// The last segment of a `/`-separated path.
pub(crate) fn base_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or_default()
}

/// A cloud storage backend. All paths are absolute, `/`-separated and
/// normalized (see [`crate::utils::path::normalize_remote_dir`]).
#[async_trait]
pub trait StorageProvider: Send + Sync {
    fn name(&self) -> &str;

    /// Verifies credentials and makes sure `remote_dir` is usable (may create it).
    async fn check_access(&self, remote_dir: &str) -> Result<()> {
        self.ensure_dir(remote_dir).await
    }

    /// Creates `remote_dir` with all missing parents. Succeeds if it already exists.
    async fn ensure_dir(&self, remote_dir: &str) -> Result<()>;

    /// Returns `None` if nothing exists at `remote_path`.
    async fn get_file_info(&self, remote_path: &str) -> Result<Option<RemoteFileInfo>>;

    /// Uploads or overwrites a file with the data of `source`. The parent
    /// directory must already exist. Dropping the future cancels the upload.
    async fn upload(
        &self,
        source: &dyn Source,
        remote_path: &str,
        progress: &Progress,
    ) -> Result<()>;

    /// Uploads a local file; see [`StorageProvider::upload`].
    async fn upload_file(
        &self,
        local_path: &Path,
        remote_path: &str,
        progress: &Progress,
    ) -> Result<()> {
        self.upload(&FileSource::new(local_path)?, remote_path, progress)
            .await
    }

    /// Uploads data the caller knows is not in the cloud yet. Providers that
    /// look the file up before writing (Google Drive, which allows equal
    /// names) skip the lookup; the others just upload.
    async fn upload_new(
        &self,
        source: &dyn Source,
        remote_path: &str,
        progress: &Progress,
    ) -> Result<()> {
        self.upload(source, remote_path, progress).await
    }

    /// Uploads a local file that is not in the cloud yet; see
    /// [`StorageProvider::upload_new`].
    async fn upload_new_file(
        &self,
        local_path: &Path,
        remote_path: &str,
        progress: &Progress,
    ) -> Result<()> {
        self.upload_new(&FileSource::new(local_path)?, remote_path, progress)
            .await
    }

    /// Downloads a file into `sink`, resuming after network failures where
    /// the server allows it. Returns the number of bytes written to the sink.
    async fn download_to(
        &self,
        remote_path: &str,
        sink: &mut dyn Sink,
        progress: &Progress,
    ) -> Result<u64>;

    /// Downloads a file, replacing `local_path` only once the download is
    /// complete. Dropping the future cancels the download and removes the
    /// partial data. Returns the size of the file.
    async fn download_file(
        &self,
        remote_path: &str,
        local_path: &Path,
        progress: &Progress,
    ) -> Result<u64> {
        let mut part = PartSink::new(local_path);
        let size = self.download_to(remote_path, &mut part, progress).await?;
        part.commit(local_path).await?;
        Ok(size)
    }

    /// Deletes a file, or a directory with all of its content.
    /// A path that does not exist is not an error.
    async fn delete(&self, remote_path: &str, is_dir: bool) -> Result<()>;

    /// Renames a file or folder within its folder. Fails if `new_name` is taken.
    async fn rename(&self, remote_path: &str, new_name: &str, is_dir: bool) -> Result<()>;

    /// Returns `None` if the file does not exist. Invalid UTF-8 is replaced.
    async fn read_text_file(&self, remote_path: &str) -> Result<Option<String>> {
        match self.get_file_info(remote_path).await? {
            Some(info) if !info.is_dir => {}
            _ => return Ok(None),
        }
        let mut data = MemorySink::default();
        self.download_to(remote_path, &mut data, &Progress::new())
            .await?;
        Ok(Some(String::from_utf8_lossy(&data.0).into_owned()))
    }

    /// Lists the direct children of `remote_dir`; empty if it does not exist.
    async fn list_dir(&self, remote_dir: &str) -> Result<Vec<RemoteFileInfo>>;

    /// Opens read access for anyone with the link and returns the link.
    /// Sharing an item that is already shared returns its link.
    async fn share(&self, remote_path: &str, _is_dir: bool) -> Result<String> {
        bail!("{} cannot share '{remote_path}' by link", self.name())
    }

    /// Shares by a link with exactly these settings, replacing an existing
    /// link. Providers without link settings only accept empty options.
    async fn share_with(
        &self,
        remote_path: &str,
        is_dir: bool,
        options: &ShareOptions,
    ) -> Result<String> {
        plain_links_only(self.name(), options)?;
        self.share(remote_path, is_dir).await
    }

    /// Fails if links with `options` cannot be made here at all; checked
    /// before work done for the sake of the link, such as an upload.
    /// Providers that share override it along with [`StorageProvider::share`].
    fn check_share(&self, _options: &ShareOptions) -> Result<()> {
        bail!("{} cannot share by link", self.name())
    }

    /// Closes access by link. Succeeds if the item was not shared.
    async fn unshare(&self, remote_path: &str, _is_dir: bool) -> Result<()> {
        bail!("{} cannot share '{remote_path}' by link", self.name())
    }
}

/// Settings of a link created by [`StorageProvider::share_with`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShareOptions {
    /// Password asked when the link is opened.
    pub password: Option<String>,
    /// The last day the link works.
    pub expires: Option<NaiveDate>,
}

impl ShareOptions {
    pub fn is_empty(&self) -> bool {
        self.password.is_none() && self.expires.is_none()
    }
}

/// [`StorageProvider::check_share`] of providers whose links have no settings.
pub(crate) fn plain_links_only(provider: &str, options: &ShareOptions) -> Result<()> {
    if !options.is_empty() {
        bail!("{provider} does not support link passwords or expiry dates");
    }
    Ok(())
}

/// Trims a user-entered file or folder name and rejects ones that are empty,
/// `.`/`..` or contain path separators.
pub fn check_name(name: &str) -> Result<&str> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." || trimmed.contains(['/', '\\']) {
        bail!("Invalid name '{name}'");
    }
    Ok(trimmed)
}

/// Validates `new_name` and returns the path `remote_path` gets when renamed.
pub fn rename_target(remote_path: &str, new_name: &str) -> Result<String> {
    let name = check_name(new_name)?;
    let (parent, old) = split_remote(remote_path);
    if old.is_empty() {
        bail!("The root folder cannot be renamed");
    }
    Ok(join_remote(&parent, name))
}

/// What creating a single folder ended with.
pub(crate) enum Mkdir {
    /// Created, or it was there already.
    Done,
    ParentMissing,
}

/// Creates `dir` and whatever parents are missing, with one request per
/// folder created: `mkdir` is tried on the folder itself first and goes
/// up only while the server reports a missing parent.
pub(crate) async fn create_dirs<F, Fut>(dir: &str, mut mkdir: F) -> Result<()>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<Mkdir>>,
{
    let mut pending = vec![dir.to_string()];
    while let Some(current) = pending.last().cloned() {
        match mkdir(current.clone()).await? {
            Mkdir::Done => {
                pending.pop();
            }
            Mkdir::ParentMissing => {
                let (parent, _) = split_remote(&current);
                if parent == "/" || pending.len() > 256 {
                    bail!(
                        "Failed to create folder '{current}': the server reports a missing parent"
                    );
                }
                pending.push(parent);
            }
        }
    }
    Ok(())
}

/// An access token reused until shortly before it expires.
#[derive(Default)]
pub(crate) struct TokenCache(tokio::sync::Mutex<Option<(String, Instant)>>);

impl TokenCache {
    /// The cached token, or a new one from `fetch`, which returns the token
    /// and its lifetime in seconds. Concurrent callers wait for one fetch.
    pub async fn get<F, Fut>(&self, fetch: F) -> Result<String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(String, Option<u64>)>>,
    {
        let mut cached = self.0.lock().await;
        if let Some((token, expires_at)) = cached.as_ref() {
            if *expires_at > Instant::now() + Duration::from_secs(300) {
                return Ok(token.clone());
            }
        }
        let (token, lifetime) = fetch().await?;
        let expires_at = Instant::now() + Duration::from_secs(lifetime.unwrap_or(3600));
        *cached = Some((token.clone(), expires_at));
        Ok(token)
    }
}

pub(crate) fn parse_rfc3339(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Parses an HTTP-date such as `Tue, 15 Nov 1994 12:45:26 GMT`.
pub(crate) fn parse_http_date(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc2822(s.trim())
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Returns the value lowercased if it looks like an MD5 hex digest.
pub(crate) fn as_md5(s: &str) -> Option<String> {
    let s = s.trim().trim_matches('"');
    (s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit())).then(|| s.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_target_stays_in_the_folder() {
        assert_eq!(rename_target("/a/b.txt", " c.txt ").unwrap(), "/a/c.txt");
        assert_eq!(rename_target("/top", "new").unwrap(), "/new");
        for bad in ["", "  ", ".", "..", "x/y", r"x\y"] {
            assert!(rename_target("/a/b", bad).is_err(), "{bad:?}");
        }
        assert!(rename_target("/", "x").is_err());
    }

    #[tokio::test]
    async fn create_dirs_goes_up_only_while_parents_are_missing() {
        let existing = std::sync::Mutex::new(vec!["/".to_string(), "/a".to_string()]);
        let tried = std::sync::Mutex::new(Vec::new());
        let mkdir = |path: String| {
            tried.lock().unwrap().push(path.clone());
            let mut existing = existing.lock().unwrap();
            let outcome = if existing.contains(&split_remote(&path).0) {
                existing.push(path);
                Mkdir::Done
            } else {
                Mkdir::ParentMissing
            };
            async move { Ok(outcome) }
        };
        create_dirs("/a/b/c", mkdir).await.unwrap();
        assert_eq!(*tried.lock().unwrap(), ["/a/b/c", "/a/b", "/a/b/c"]);

        tried.lock().unwrap().clear();
        create_dirs("/a/b/d", mkdir).await.unwrap();
        assert_eq!(*tried.lock().unwrap(), ["/a/b/d"]);
    }

    #[test]
    fn parses_dates_and_md5() {
        assert!(parse_http_date("Tue, 15 Nov 1994 12:45:26 GMT").is_some());
        assert!(parse_rfc3339("2009-10-12T17:50:30.000Z").is_some());
        assert_eq!(
            as_md5("\"D41D8CD98F00B204E9800998ECF8427E\"").as_deref(),
            Some("d41d8cd98f00b204e9800998ecf8427e")
        );
        assert_eq!(as_md5("abc-2"), None);
    }
}
