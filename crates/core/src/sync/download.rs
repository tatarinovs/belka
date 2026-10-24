use super::{local_has, Summary};
use crate::client::PartSink;
use crate::progress::Progress;
use crate::storage::{RemoteFileInfo, StorageProvider};
use crate::utils::path::{normalize_remote_dir, sanitize_file_name, unique_path};
use crate::utils::{format_bytes, format_rate};
use anyhow::{bail, Context, Result};
use futures_util::stream::{self, StreamExt};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};
use tracing::{error, info, warn};

#[derive(Clone, Debug)]
pub struct DownloadOptions {
    pub parallel: usize,
    /// Download even what looks unchanged locally.
    pub overwrite: bool,
    pub dry_run: bool,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            parallel: super::DEFAULT_PARALLEL,
            overwrite: false,
            dry_run: false,
        }
    }
}

struct FileJob {
    remote: RemoteFileInfo,
    local: PathBuf,
}

/// Downloads the cloud folder (or file) `remote_path` into the local folder
/// `local_dir`: a folder's contents land directly in `local_dir`, a file is
/// saved in it under its own name. Files that are there and unchanged are
/// skipped, so an interrupted download continues where it stopped. Every
/// downloaded file gets the cloud's date.
pub async fn download(
    provider: &dyn StorageProvider,
    remote_path: &str,
    local_dir: &Path,
    opts: &DownloadOptions,
) -> Result<Summary> {
    let remote_path = normalize_remote_dir(remote_path);
    if local_dir.is_file() {
        bail!(
            "'{}' is a file; a folder to download into is needed",
            local_dir.display()
        );
    }
    let root = if remote_path == "/" {
        RemoteFileInfo::dir(remote_path.clone())
    } else {
        provider
            .get_file_info(&remote_path)
            .await
            .with_context(|| format!("Cannot access '{remote_path}' on {}", provider.name()))?
            .with_context(|| format!("'{remote_path}' does not exist on {}", provider.name()))?
    };

    let mut summary = Summary::default();
    let (dirs, jobs) = if root.is_dir {
        info!("Scanning '{remote_path}'...");
        scan_remote(provider, &root.path, local_dir, opts.parallel, &mut summary).await
    } else {
        let local = local_dir.join(local_name(&root.name));
        (
            Vec::new(),
            vec![FileJob {
                remote: root,
                local,
            }],
        )
    };

    if !opts.dry_run {
        for dir in std::iter::once(local_dir.to_path_buf()).chain(dirs) {
            if let Err(err) = std::fs::create_dir_all(&dir) {
                error!("  [!] Cannot create folder '{}': {err}", dir.display());
                summary.fail(&dir.display().to_string(), err.to_string());
            }
        }
    }

    let total: u64 = jobs.iter().map(|j| j.remote.size).sum();
    info!(
        "Found {} file(s), {} total; downloading into '{}' with {} parallel job(s)",
        jobs.len(),
        format_bytes(total),
        local_dir.display(),
        opts.parallel
    );
    summary.checked = jobs.len() as u64;

    let mut results = stream::iter(jobs)
        .map(|job| async move {
            let result = download_one(provider, &job, opts).await;
            (job, result)
        })
        .buffer_unordered(opts.parallel.max(1));
    while let Some((job, result)) = results.next().await {
        match result {
            Ok(Some(bytes)) => {
                summary.transferred += 1;
                summary.bytes += bytes;
            }
            Ok(None) => summary.unchanged += 1,
            Err(err) => {
                error!("  [!] Failed to download '{}': {err:#}", job.remote.path);
                summary.fail(&job.remote.path, format!("{err:#}"));
            }
        }
    }
    Ok(summary)
}

/// Lists the cloud tree level by level, several folders at a time. Returns
/// the local folders to create and the files to download.
async fn scan_remote(
    provider: &dyn StorageProvider,
    remote_root: &str,
    local_root: &Path,
    parallel: usize,
    summary: &mut Summary,
) -> (Vec<PathBuf>, Vec<FileJob>) {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    // Names that are equal once made safe, or differ only in case.
    let mut taken = HashSet::new();
    let mut level = vec![(remote_root.to_string(), local_root.to_path_buf())];
    while !level.is_empty() {
        let mut listings = stream::iter(std::mem::take(&mut level))
            .map(|(remote, local)| async move {
                let result = provider.list_dir(&remote).await;
                (remote, local, result)
            })
            .buffer_unordered(parallel.max(1));
        while let Some((remote, local, result)) = listings.next().await {
            let items = match result {
                Ok(items) => items,
                Err(err) => {
                    error!("  [!] Cannot list '{remote}': {err:#}");
                    summary.fail(&remote, format!("{err:#}"));
                    continue;
                }
            };
            for item in items {
                let path = unique_path(local.join(local_name(&item.name)), &mut taken);
                if item.is_dir {
                    dirs.push(path.clone());
                    level.push((item.path, path));
                } else {
                    files.push(FileJob {
                        remote: item,
                        local: path,
                    });
                }
            }
        }
    }
    dirs.sort();
    files.sort_by(|a, b| a.remote.path.cmp(&b.remote.path));
    (dirs, files)
}

/// A cloud name made safe for the local file system: a name such as `..`,
/// or one with characters Windows forbids, must not escape or break the path.
fn local_name(name: &str) -> String {
    let safe = sanitize_file_name(name);
    if safe != name {
        warn!("  [~] '{name}' is saved as '{safe}'");
    }
    safe
}

/// `Some(bytes)` if downloaded, `None` if the local copy is up to date.
async fn download_one(
    provider: &dyn StorageProvider,
    job: &FileJob,
    opts: &DownloadOptions,
) -> Result<Option<u64>> {
    let mut remote = job.remote.clone();
    // Listings with upload times instead of the original dates (S3): the
    // original is asked when it is needed, to compare or to set.
    let mut dated = !provider.listed_dates_are_upload_times();
    if !dated && remote.md5.is_none() && !opts.overwrite {
        original_date(provider, &mut remote).await;
        dated = true;
    }
    if !opts.overwrite {
        match local_has(&remote, &job.local).await {
            Ok(true) => {
                info!("  [=] Unchanged: {}", job.local.display());
                return Ok(None);
            }
            Ok(false) => {}
            Err(err) => warn!(
                "  [~] Cannot check local '{}' ({err:#}); downloading anyway",
                job.local.display()
            ),
        }
    }
    if opts.dry_run {
        info!(
            "  [dry-run] Would download {} ({})",
            job.remote.path,
            format_bytes(job.remote.size)
        );
        return Ok(Some(job.remote.size));
    }
    info!(
        "  [v] Downloading {} ({})...",
        job.remote.path,
        format_bytes(job.remote.size)
    );
    let started = Instant::now();
    let mut part = PartSink::new(&job.local);
    let size = provider
        .download_to(&job.remote.path, &mut part, &Progress::new())
        .await?;
    // Whatever came instead of the file (an error page, a changed file)
    // must not replace the local copy. A listed 0 may also mean "unknown"
    // (WebDAV servers without getcontentlength).
    if job.remote.size > 0 && size != job.remote.size {
        bail!(
            "Received {size} bytes instead of {}; the file may have changed in the cloud",
            job.remote.size
        );
    }
    part.commit(&job.local).await?;
    if !dated {
        original_date(provider, &mut remote).await;
    }
    // The cloud's date, so that the next upload sees the file as unchanged.
    if let Some(modified) = remote.modified {
        if let Err(err) = set_modified(&job.local, modified.into()) {
            warn!(
                "  [~] Cannot set the date of '{}': {err}",
                job.local.display()
            );
        }
    }
    info!(
        "  [+] Downloaded {} in {:.1}s ({})",
        job.local.display(),
        started.elapsed().as_secs_f64(),
        format_rate(size, started.elapsed())
    );
    Ok(Some(size))
}

/// Replaces the listed date of `remote` with the one the cloud keeps for
/// the file itself, if it can be asked.
async fn original_date(provider: &dyn StorageProvider, remote: &mut RemoteFileInfo) {
    match provider.get_file_info(&remote.path).await {
        Ok(Some(info)) => remote.modified = info.modified.or(remote.modified),
        Ok(None) => {}
        Err(err) => warn!("  [~] Cannot read the date of '{}': {err:#}", remote.path),
    }
}

pub(crate) fn set_modified(path: &Path, time: SystemTime) -> std::io::Result<()> {
    std::fs::File::options()
        .write(true)
        .open(path)?
        .set_modified(time)
}
