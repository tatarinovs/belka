//! Copying trees between a local folder and a cloud folder. Used by the CLI
//! (`upload`, `restore`) and by the GUI backups, so they decide the same way
//! what is new, what is unchanged and what is extra.
//!
//! * [`scan`] lists a local tree through a [`Filter`];
//! * [`upload`] sends the files the cloud lacks, optionally remembering what
//!   is backed up in an [`Index`] so that unchanged files are not even read;
//! * [`delete_extra`] removes from the cloud what is no longer local;
//! * [`download`] is the reverse of `upload`.

mod download;
mod extra;
mod glob;
mod index;
mod upload;

pub use download::{download, DownloadOptions};
pub use extra::delete_extra;
pub use glob::Pattern;
pub use index::Index;
pub use upload::{upload, Observer, Outcome, UploadOptions};

use crate::storage::{RemoteFileInfo, StorageProvider};
use crate::utils::hash::compute_md5_file;
use crate::utils::path::{join_remote, split_remote};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use futures_util::stream::{self, StreamExt};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Files transferred at the same time unless the caller says otherwise.
pub const DEFAULT_PARALLEL: usize = 3;
/// Failures kept in a [`Summary`] with their reasons; the rest are counted.
const KEPT_FAILURES: usize = 50;

/// Which local files take part. Folders are walked unless excluded.
#[derive(Clone, Debug, Default)]
pub struct Filter {
    include: Vec<Pattern>,
    exclude: Vec<Pattern>,
    /// Leaves out names starting with `.`: on Android these are thumbnails
    /// and photos in the trash (`.trashed-…`), elsewhere caches and settings.
    pub skip_hidden: bool,
}

impl Filter {
    pub fn new(include: &[String], exclude: &[String], skip_hidden: bool) -> Self {
        let patterns = |list: &[String]| {
            list.iter()
                .filter(|p| !p.trim().is_empty())
                .map(|p| Pattern::new(p))
                .collect()
        };
        Self {
            include: patterns(include),
            exclude: patterns(exclude),
            skip_hidden,
        }
    }

    /// Everything, hidden files included.
    pub fn all() -> Self {
        Self::default()
    }

    /// Whether a file (or with `is_dir`, a folder to walk) takes part.
    /// `relative` is `/`-separated and relative to the root.
    pub fn accepts(&self, relative: &str, is_dir: bool) -> bool {
        if self.skip_hidden && relative.split('/').any(|n| n.starts_with('.')) {
            return false;
        }
        if self.exclude.iter().any(|p| p.matches(relative)) {
            return false;
        }
        is_dir || self.include.is_empty() || self.include.iter().any(|p| p.matches_name(relative))
    }
}

#[derive(Clone, Debug)]
pub struct LocalFile {
    pub path: PathBuf,
    /// `/`-separated, relative to the scanned root.
    pub relative: String,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

impl LocalFile {
    fn from_metadata(path: PathBuf, relative: String, meta: &std::fs::Metadata) -> Self {
        Self {
            path,
            relative,
            size: meta.len(),
            modified: meta.modified().ok(),
        }
    }

    /// Size and modification time in seconds: what the [`Index`] compares.
    pub fn stamp(&self) -> (u64, i64) {
        let secs = self
            .modified
            .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs() as i64);
        (self.size, secs)
    }
}

/// A scanned local tree.
#[derive(Debug, Default)]
pub struct LocalTree {
    pub files: Vec<LocalFile>,
    /// Every folder walked, relative and `/`-separated, parents first.
    pub dirs: Vec<String>,
    /// Entries that could not be read, with the reason.
    pub errors: Vec<String>,
    /// Folders reached through a symlink or junction, relative: they are not
    /// walked, and what the cloud has under their names is not extra.
    pub links: Vec<String>,
    /// The root was a single file rather than a folder.
    pub single_file: bool,
}

/// Lists `root` through `filter`. A file as `root` gives a tree of that one
/// file. Symlinked files are followed, symlinked folders are not, so a link
/// cannot make the walk endless.
pub fn scan(root: &Path, filter: &Filter) -> Result<LocalTree> {
    let meta =
        std::fs::metadata(root).with_context(|| format!("Cannot access '{}'", root.display()))?;
    let mut tree = LocalTree::default();
    if meta.is_file() {
        let name = root
            .file_name()
            .and_then(|n| n.to_str())
            .with_context(|| format!("'{}' has no valid UTF-8 file name", root.display()))?;
        tree.files.push(LocalFile::from_metadata(
            root.to_path_buf(),
            name.to_string(),
            &meta,
        ));
        tree.single_file = true;
        return Ok(tree);
    }

    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, relative)) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) if relative.is_empty() => {
                return Err(err).with_context(|| format!("Failed to read '{}'", dir.display()))
            }
            Err(err) => {
                tree.errors.push(format!("{}: {err}", dir.display()));
                continue;
            }
        };
        let mut subdirs = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    tree.errors.push(format!("{}: {err}", dir.display()));
                    continue;
                }
            };
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                tree.errors.push(format!(
                    "{}: the name is not valid UTF-8",
                    entry.path().display()
                ));
                continue;
            };
            let child = if relative.is_empty() {
                name
            } else {
                format!("{relative}/{name}")
            };
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                if filter.accepts(&child, true) {
                    subdirs.push((entry.path(), child));
                }
            } else if filter.accepts(&child, false) {
                match std::fs::metadata(entry.path()) {
                    Ok(meta) if meta.is_file() => {
                        tree.files
                            .push(LocalFile::from_metadata(entry.path(), child, &meta));
                    }
                    Ok(meta) if meta.is_dir() => tree.links.push(child),
                    Ok(_) => {}
                    Err(err) => tree
                        .errors
                        .push(format!("{}: {err}", entry.path().display())),
                }
            }
        }
        for (_, child) in &subdirs {
            tree.dirs.push(child.clone());
        }
        // Reversed so the stack walks folders in name order.
        subdirs.sort_by(|a, b| b.1.cmp(&a.1));
        stack.extend(subdirs);
    }
    tree.dirs.sort();
    tree.links.sort();
    tree.files.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(tree)
}

/// Whether the cloud file already holds the local one, for uploads:
/// 1. different size -> no;
/// 2. the listing has a real MD5 -> compare hashes;
/// 3. the same date (2s tolerance) -> yes: uploads keep the original's
///    date, so another file with the same name, size and date is unlikely;
/// 4. the provider computes an MD5 on request -> compare hashes;
/// 5. otherwise the local file must not be newer (2s tolerance). This
///    matters for multi-volume archives whose parts all have the same size.
///
/// `strict` skips step 3, for when the local file is to be deleted.
pub async fn cloud_has(
    provider: &dyn StorageProvider,
    remote: &RemoteFileInfo,
    local: &LocalFile,
    strict: bool,
) -> Result<bool> {
    if remote.is_dir || remote.size != local.size {
        return Ok(false);
    }
    let local_time: Option<DateTime<Utc>> = local.modified.map(Into::into);
    let same_date = matches!((local_time, remote.modified),
        (Some(l), Some(r)) if (l - r).num_seconds().abs() <= 2);
    let remote_md5 = match &remote.md5 {
        Some(md5) => Some(md5.clone()),
        None if same_date && !strict => return Ok(true),
        None => provider.content_md5(&remote.path).await?,
    };
    if let Some(remote_md5) = remote_md5 {
        let local_md5 = compute_md5_file(&local.path).await?;
        if !local_md5.eq_ignore_ascii_case(&remote_md5) {
            tracing::info!("  [~] Same size but different MD5: {}", remote.path);
            return Ok(false);
        }
        return Ok(true);
    }
    if let (Some(local_time), Some(remote_time)) = (local_time, remote.modified) {
        if local_time > remote_time + chrono::Duration::seconds(2) {
            tracing::info!(
                "  [~] Same size but local file is newer: {} (local {}, remote {})",
                remote.path,
                local_time.format("%Y-%m-%d %H:%M:%S UTC"),
                remote_time.format("%Y-%m-%d %H:%M:%S UTC")
            );
            return Ok(false);
        }
    }
    Ok(true)
}

/// Whether the local file already holds the cloud one, for downloads: the
/// same size and either the same MD5 or the same date (2s tolerance).
pub async fn local_has(remote: &RemoteFileInfo, local: &Path) -> Result<bool> {
    let meta = match tokio::fs::metadata(local).await {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    if !meta.is_file() || meta.len() != remote.size {
        return Ok(false);
    }
    if let Some(remote_md5) = remote.md5.as_deref() {
        return Ok(compute_md5_file(local)
            .await?
            .eq_ignore_ascii_case(remote_md5));
    }
    let (Some(remote_time), Ok(local_time)) = (remote.modified, meta.modified()) else {
        return Ok(false);
    };
    let local_time: DateTime<Utc> = local_time.into();
    Ok((local_time - remote_time).num_seconds().abs() <= 2)
}

/// What the cloud folders that files go to already hold. Each is listed once,
/// which takes far fewer requests than asking about every file; a folder that
/// cannot be listed (often because it does not exist yet) is asked about file
/// by file.
pub struct RemoteListing {
    listed: HashSet<String>,
    items: HashMap<String, RemoteFileInfo>,
}

impl RemoteListing {
    pub fn empty() -> Self {
        Self {
            listed: HashSet::new(),
            items: HashMap::new(),
        }
    }

    /// Lists the folders that the files `relatives` under `root` are in.
    pub async fn new<'a>(
        provider: &dyn StorageProvider,
        root: &str,
        relatives: impl Iterator<Item = &'a str>,
        parallel: usize,
    ) -> Self {
        let dirs: HashSet<String> = relatives
            .map(|relative| split_remote(&join_remote(root, relative)).0)
            .collect();
        let mut listings = stream::iter(dirs)
            .map(|dir| async move {
                let result = provider.list_dir(&dir).await;
                (dir, result)
            })
            .buffer_unordered(parallel.max(1));
        let mut listing = Self::empty();
        while let Some((dir, result)) = listings.next().await {
            match result {
                Ok(items) => {
                    listing
                        .items
                        .extend(items.into_iter().map(|item| (item.path.clone(), item)));
                    listing.listed.insert(dir);
                }
                Err(err) => tracing::debug!("Cannot list '{dir}' ({err:#}); asking file by file"),
            }
        }
        listing
    }

    /// The cloud file at `path`, from the listing or asked directly.
    pub async fn info(
        &self,
        provider: &dyn StorageProvider,
        path: &str,
    ) -> Result<Option<RemoteFileInfo>> {
        if self.listed.contains(&split_remote(path).0) {
            Ok(self.items.get(path).cloned())
        } else {
            provider.get_file_info(path).await
        }
    }
}

/// What a transfer ended with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Files considered after the index: the ones that might have changed.
    pub checked: u64,
    pub transferred: u64,
    pub bytes: u64,
    /// Files that were already there: by the index or by comparing.
    pub unchanged: u64,
    pub failed: u64,
    /// The first failures: (file, reason).
    pub failures: Vec<(String, String)>,
}

impl Summary {
    pub(crate) fn fail(&mut self, file: &str, reason: String) {
        self.failed += 1;
        if self.failures.len() < KEPT_FAILURES {
            self.failures.push((file.to_string(), reason));
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
