pub mod crypt;
pub mod google;
pub mod mailru;
pub mod s3;
pub mod webdav;
pub mod yandex;

use crate::client::{FileSource, PartSink, Sink, Source};
use crate::progress::Progress;
use crate::utils::path::{join_remote, split_remote};
use anyhow::{bail, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::path::Path;

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

/// A cloud storage backend. All paths are absolute, `/`-separated and
/// normalized (see [`crate::utils::path::normalize_remote_dir`]).
#[async_trait]
pub trait StorageProvider: Send + Sync {
    fn name(&self) -> &'static str;

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

    /// Returns `None` if the file does not exist.
    async fn read_text_file(&self, remote_path: &str) -> Result<Option<String>>;

    /// Lists the direct children of `remote_dir`; empty if it does not exist.
    async fn list_dir(&self, remote_dir: &str) -> Result<Vec<RemoteFileInfo>>;

    /// Opens read access for anyone with the link and returns the link.
    /// Sharing an item that is already shared returns its link.
    async fn share(&self, remote_path: &str, _is_dir: bool) -> Result<String> {
        bail!("{} cannot share '{remote_path}' by link", self.name())
    }

    /// Closes access by link. Succeeds if the item was not shared.
    async fn unshare(&self, remote_path: &str, _is_dir: bool) -> Result<()> {
        bail!("{} cannot share '{remote_path}' by link", self.name())
    }
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
