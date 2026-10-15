//! Downloads a cloud folder (or a single file) into a local folder: the
//! reverse of the upload mode. Files that are already there and unchanged
//! are skipped, so an interrupted restore continues where it stopped.

use super::Stats;
use crate::format::{format_bytes, format_rate};
use crate::hash::compute_md5_file;
use crate::storage::{RemoteFileInfo, StorageProvider};
use crate::utils::path::{normalize_remote_dir, sanitize_file_name};
use anyhow::{bail, Context, Result};
use belka_core::Progress;
use futures_util::stream::{self, StreamExt};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};
use tracing::{error, info, warn};

pub struct RestoreOptions {
    pub overwrite: bool,
    pub dry_run: bool,
    pub jobs: usize,
}

struct FileJob {
    remote: RemoteFileInfo,
    local: PathBuf,
}

enum Outcome {
    Downloaded(u64),
    Skipped,
    Failed,
}

/// Restores `remote_path` into the folder `local_dir`. A remote folder's
/// contents land directly in `local_dir`; a remote file is saved in it
/// under its own name.
pub async fn run_restore(
    provider: &dyn StorageProvider,
    remote_path: &str,
    local_dir: &Path,
    opts: &RestoreOptions,
) -> Result<Stats> {
    let remote_path = normalize_remote_dir(remote_path);
    if local_dir.is_file() {
        bail!(
            "'{}' is a file; restore needs a folder to download into",
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

    let mut stats = Stats::default();
    let (dirs, jobs) = if root.is_dir {
        info!("Scanning '{remote_path}'...");
        scan(provider, &root.path, local_dir, opts.jobs, &mut stats).await
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
                stats.failed += 1;
            }
        }
    }

    let total_bytes: u64 = jobs.iter().map(|j| j.remote.size).sum();
    info!(
        "Found {} file(s), {} total; restoring into '{}' with {} parallel job(s)",
        jobs.len(),
        format_bytes(total_bytes),
        local_dir.display(),
        opts.jobs
    );

    let started = Instant::now();
    let mut results = stream::iter(jobs)
        .map(|job| process_file(provider, job, opts))
        .buffer_unordered(opts.jobs.max(1));
    while let Some(outcome) = results.next().await {
        stats.processed += 1;
        match outcome {
            Outcome::Downloaded(bytes) => {
                stats.uploaded += 1;
                stats.bytes_uploaded += bytes;
            }
            Outcome::Skipped => stats.skipped += 1,
            Outcome::Failed => stats.failed += 1,
        }
    }

    info!("--------------------------------------------------");
    info!(
        "Files: {} processed, {} downloaded ({}), {} unchanged, {} failed",
        stats.processed,
        stats.uploaded,
        format_bytes(stats.bytes_uploaded),
        stats.skipped,
        stats.failed
    );
    if !opts.dry_run && stats.bytes_uploaded > 0 {
        info!(
            "Elapsed: {:.0}s, average {}",
            started.elapsed().as_secs_f64(),
            format_rate(stats.bytes_uploaded, started.elapsed())
        );
    }
    Ok(stats)
}

/// Lists the remote tree level by level, several folders at a time.
/// Returns the local folders to create and the files to download.
async fn scan(
    provider: &dyn StorageProvider,
    remote_root: &str,
    local_root: &Path,
    parallel: usize,
    stats: &mut Stats,
) -> (Vec<PathBuf>, Vec<FileJob>) {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
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
                    stats.failed += 1;
                    continue;
                }
            };
            for item in items {
                let path = local.join(local_name(&item.name));
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

/// A remote name made safe for the local file system: a name such as `..`,
/// or one with characters Windows forbids, must not escape or break the path.
fn local_name(name: &str) -> String {
    let safe = sanitize_file_name(name);
    if safe != name {
        warn!("  [~] '{name}' is saved as '{safe}'");
    }
    safe
}

async fn process_file(
    provider: &dyn StorageProvider,
    job: FileJob,
    opts: &RestoreOptions,
) -> Outcome {
    if !opts.overwrite {
        match is_up_to_date(&job).await {
            Ok(true) => {
                info!("  [=] Unchanged: {}", job.local.display());
                return Outcome::Skipped;
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
        return Outcome::Downloaded(job.remote.size);
    }

    info!(
        "  [v] Downloading {} ({})...",
        job.remote.path,
        format_bytes(job.remote.size)
    );
    let started = Instant::now();
    match provider
        .download_file(&job.remote.path, &job.local, &Progress::new())
        .await
    {
        Ok(size) => {
            // The cloud's date, so that the next upload sees the file as unchanged.
            if let Some(modified) = job.remote.modified {
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
            Outcome::Downloaded(size)
        }
        Err(err) => {
            error!("  [!] Failed to download '{}': {err:#}", job.remote.path);
            Outcome::Failed
        }
    }
}

fn set_modified(path: &Path, time: SystemTime) -> std::io::Result<()> {
    std::fs::File::options()
        .write(true)
        .open(path)?
        .set_modified(time)
}

/// The local copy matches if it has the same size and either the same MD5
/// (when the cloud provides one) or the same date (2s tolerance).
async fn is_up_to_date(job: &FileJob) -> Result<bool> {
    let meta = match tokio::fs::metadata(&job.local).await {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    if !meta.is_file() || meta.len() != job.remote.size {
        return Ok(false);
    }
    if let Some(remote_md5) = job.remote.md5.as_deref() {
        let local_md5 = compute_md5_file(&job.local).await?;
        return Ok(local_md5.eq_ignore_ascii_case(remote_md5));
    }
    let (Some(remote), Ok(local)) = (job.remote.modified, meta.modified()) else {
        return Ok(false);
    };
    let local: chrono::DateTime<chrono::Utc> = local.into();
    Ok((local - remote).num_seconds().abs() <= 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use belka_core::client::{Sink, Source};
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// A cloud with files held in memory; folders are implied by paths.
    #[derive(Default)]
    struct FakeCloud {
        files: HashMap<String, Vec<u8>>,
        downloads: Mutex<Vec<String>>,
    }

    impl FakeCloud {
        fn info(&self, path: &str) -> Option<RemoteFileInfo> {
            if let Some(data) = self.files.get(path) {
                return Some(RemoteFileInfo {
                    name: path.rsplit('/').next().unwrap().into(),
                    path: path.into(),
                    is_dir: false,
                    size: data.len() as u64,
                    md5: None,
                    modified: Some(chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap()),
                });
            }
            let prefix = format!("{path}/");
            self.files
                .keys()
                .any(|p| p.starts_with(&prefix))
                .then(|| RemoteFileInfo::dir(path.into()))
        }
    }

    #[async_trait]
    impl StorageProvider for FakeCloud {
        fn name(&self) -> &str {
            "fake"
        }
        async fn ensure_dir(&self, _remote_dir: &str) -> Result<()> {
            unimplemented!()
        }
        async fn get_file_info(&self, remote_path: &str) -> Result<Option<RemoteFileInfo>> {
            Ok(self.info(remote_path))
        }
        async fn upload(&self, _s: &dyn Source, _p: &str, _pr: &Progress) -> Result<()> {
            unimplemented!()
        }
        async fn download_to(
            &self,
            remote_path: &str,
            sink: &mut dyn Sink,
            _progress: &Progress,
        ) -> Result<u64> {
            self.downloads.lock().unwrap().push(remote_path.into());
            let data = self.files[remote_path].clone();
            sink.reset().await?;
            sink.write(&data).await?;
            Ok(data.len() as u64)
        }
        async fn delete(&self, _p: &str, _d: bool) -> Result<()> {
            unimplemented!()
        }
        async fn rename(&self, _p: &str, _n: &str, _d: bool) -> Result<()> {
            unimplemented!()
        }
        async fn list_dir(&self, remote_dir: &str) -> Result<Vec<RemoteFileInfo>> {
            let prefix = format!("{}/", remote_dir.trim_end_matches('/'));
            let mut names: Vec<String> = self
                .files
                .keys()
                .filter_map(|p| p.strip_prefix(&prefix))
                .map(|rest| rest.split('/').next().unwrap().to_string())
                .collect();
            names.sort();
            names.dedup();
            Ok(names
                .iter()
                .filter_map(|n| self.info(&format!("{prefix}{n}")))
                .collect())
        }
    }

    fn cloud() -> FakeCloud {
        let mut files = HashMap::new();
        files.insert("/B/a.txt".to_string(), b"aaa".to_vec());
        files.insert("/B/sub/deep/b.txt".to_string(), b"bb".to_vec());
        files.insert("/B/sub/c:d.txt".to_string(), b"c".to_vec());
        FakeCloud {
            files,
            ..Default::default()
        }
    }

    fn opts() -> RestoreOptions {
        RestoreOptions {
            overwrite: false,
            dry_run: false,
            jobs: 2,
        }
    }

    #[tokio::test]
    async fn restores_tree_and_skips_unchanged_on_second_run() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("restored");
        let cloud = cloud();

        let stats = run_restore(&cloud, "/B", &target, &opts()).await.unwrap();
        assert_eq!((stats.uploaded, stats.failed), (3, 0));
        assert_eq!(std::fs::read(target.join("a.txt")).unwrap(), b"aaa");
        assert_eq!(std::fs::read(target.join("sub/deep/b.txt")).unwrap(), b"bb");
        assert_eq!(std::fs::read(target.join("sub/c_d.txt")).unwrap(), b"c");

        let stats = run_restore(&cloud, "/B", &target, &opts()).await.unwrap();
        assert_eq!((stats.uploaded, stats.skipped), (0, 3));
        assert_eq!(cloud.downloads.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn restores_a_single_file_and_honors_dry_run() {
        let dir = tempfile::tempdir().unwrap();
        let cloud = cloud();
        let dry = RestoreOptions {
            dry_run: true,
            ..opts()
        };
        let stats = run_restore(&cloud, "/B/a.txt", dir.path(), &dry)
            .await
            .unwrap();
        assert_eq!(stats.uploaded, 1);
        assert!(!dir.path().join("a.txt").exists());

        run_restore(&cloud, "/B/a.txt", dir.path(), &opts())
            .await
            .unwrap();
        assert_eq!(std::fs::read(dir.path().join("a.txt")).unwrap(), b"aaa");
        assert!(run_restore(&cloud, "/missing", dir.path(), &opts())
            .await
            .is_err());
    }
}
