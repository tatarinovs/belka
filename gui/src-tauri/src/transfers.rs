//! The transfer queue: uploads, downloads and copies between clouds run as
//! background tasks, at most
//! [`MAX_PARALLEL`] at a time. The frontend polls [`Transfers::list`] and reads
//! progress counters directly, so running transfers never block on the UI.
//! A queue can hold thousands of files, so the page gets the running ones,
//! the head of the queue and the latest finished ones, plus counts of all.

use crate::error::translate;
use anyhow::{Context, Result};
use belka_core::client::MAX_ATTEMPTS;
use belka_core::storage::copy::copy_file;
use belka_core::sync::{self, LocalFile};
use belka_core::utils::path::split_remote;
use belka_core::{Progress, RemoteFileInfo, StorageProvider};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::async_runtime::{spawn, JoinHandle};
use tokio::sync::Semaphore;

const MAX_PARALLEL: usize = 3;
/// Queued transfers sent to the page; the rest are only counted.
const SHOWN_QUEUED: usize = 50;
/// The most recently finished transfers sent to the page.
const SHOWN_FINISHED: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Upload,
    Download,
    /// From one cloud folder to another, in the same cloud or another one.
    Copy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum Status {
    Queued,
    Running,
    Done,
    Failed { error: String },
    Cancelled,
}

impl Status {
    fn is_active(&self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }
}

/// What to transfer; kept so a failed transfer can be retried.
#[derive(Clone, Debug)]
pub struct Spec {
    pub direction: Direction,
    pub account_id: String,
    pub account_name: String,
    pub local: PathBuf,
    pub remote: String,
    /// Known in advance for downloads (from the listing); 0 if unknown.
    pub size: u64,
    /// The cloud file: for a download the one to fetch, for an upload or a
    /// copy the one already at the target. A file the other side already
    /// holds with the same data is not sent again.
    pub known: Option<RemoteFileInfo>,
    /// For a copy, where it comes from; `account_id` and `remote` are where
    /// it goes, `local` is not used.
    pub from: Option<CopyFrom>,
}

#[derive(Clone, Debug)]
pub struct CopyFrom {
    pub account_id: String,
    pub account_name: String,
    /// The file to copy, as listed.
    pub file: RemoteFileInfo,
}

struct Job {
    id: u64,
    spec: Spec,
    status: Status,
    progress: Progress,
    handle: Option<JoinHandle<()>>,
    /// Order in which transfers finished; 0 while active.
    finished: u64,
    /// Done without sending anything: the other side had the same file.
    unchanged: bool,
}

impl Job {
    fn view(&self) -> TransferView {
        let (remote_dir, name) = split_remote(&self.spec.remote);
        // Kept as the core wrote it, for the log; the page gets Russian.
        let status = match &self.status {
            Status::Failed { error } => Status::Failed {
                error: translate(error),
            },
            status => status.clone(),
        };
        // A copy names both clouds, and the source where a download has
        // the local file.
        let (account_name, local_path) = match &self.spec.from {
            Some(from) => (
                format!("{} → {}", from.account_name, self.spec.account_name),
                format!("{}: {}", from.account_name, from.file.path),
            ),
            None => (
                self.spec.account_name.clone(),
                self.spec.local.display().to_string(),
            ),
        };
        TransferView {
            id: self.id,
            direction: self.spec.direction,
            account_id: self.spec.account_id.clone(),
            account_name,
            name,
            local_path,
            remote_path: self.spec.remote.clone(),
            remote_dir,
            status,
            unchanged: self.unchanged,
            done: self.progress.done(),
            total: self.progress.total(),
            attempt: self.progress.attempt(),
            max_attempts: MAX_ATTEMPTS,
        }
    }
}

#[derive(Serialize)]
pub struct TransferView {
    id: u64,
    direction: Direction,
    account_id: String,
    account_name: String,
    name: String,
    local_path: String,
    remote_path: String,
    remote_dir: String,
    #[serde(flatten)]
    status: Status,
    unchanged: bool,
    done: u64,
    total: u64,
    /// Above 1 while network failures are retried.
    attempt: u32,
    max_attempts: u32,
}

#[derive(Default, Serialize)]
pub struct Counts {
    queued: usize,
    running: usize,
    done: usize,
    failed: usize,
    cancelled: usize,
}

#[derive(Serialize)]
pub struct TransferList {
    items: Vec<TransferView>,
    counts: Counts,
    /// A filtered slice for display; `items` still feeds completion events.
    view_items: Option<Vec<TransferView>>,
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TransferFilter {
    #[default]
    All,
    Active,
    Failed,
    Done,
    Cancelled,
}

impl TransferFilter {
    fn accepts(self, status: &Status) -> bool {
        match self {
            Self::All => true,
            Self::Active => status.is_active(),
            Self::Failed => matches!(status, Status::Failed { .. }),
            Self::Done => matches!(status, Status::Done),
            Self::Cancelled => matches!(status, Status::Cancelled),
        }
    }
}

#[derive(Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    /// Queued and running transfers.
    active: usize,
    /// Of them uploads.
    uploads: usize,
    /// A running file, to name in the notification.
    name: String,
    /// Progress of the running files.
    done_bytes: u64,
    total_bytes: u64,
    /// The file an external player is streaming, or "".
    pub player: String,
}

pub struct Transfers {
    jobs: Mutex<Vec<Job>>,
    next_id: AtomicU64,
    finished_seq: AtomicU64,
    slots: Arc<Semaphore>,
    /// Remote folders already created, as (account id, path). Folder creation
    /// is serialized so parallel uploads never create the same folder twice
    /// (Google Drive would happily make duplicates).
    ensured: tokio::sync::Mutex<HashSet<(String, String)>>,
    /// Where copies from a cloud that cannot read parts of files wait
    /// between download and upload. Emptied on start.
    temp_dir: PathBuf,
    temp_seq: AtomicU64,
}

impl Transfers {
    pub fn new(temp_dir: PathBuf) -> Arc<Self> {
        // Left by copies that were running when the app closed.
        let _ = std::fs::remove_dir_all(&temp_dir);
        Arc::new(Self {
            jobs: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
            finished_seq: AtomicU64::new(1),
            slots: Arc::new(Semaphore::new(MAX_PARALLEL)),
            ensured: tokio::sync::Mutex::new(HashSet::new()),
            temp_dir,
            temp_seq: AtomicU64::new(1),
        })
    }

    pub fn enqueue(self: &Arc<Self>, spec: Spec, provider: Arc<dyn StorageProvider>) -> u64 {
        self.enqueue_with(spec, provider, None)
    }

    /// Queues a copy: `target` is the provider of `spec.account_id`,
    /// `source` the one of `spec.from`.
    pub fn enqueue_copy(
        self: &Arc<Self>,
        spec: Spec,
        target: Arc<dyn StorageProvider>,
        source: Arc<dyn StorageProvider>,
    ) -> u64 {
        self.enqueue_with(spec, target, Some(source))
    }

    fn enqueue_with(
        self: &Arc<Self>,
        spec: Spec,
        provider: Arc<dyn StorageProvider>,
        source: Option<Arc<dyn StorageProvider>>,
    ) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let progress = Progress::new();
        progress.set_total(spec.size);
        self.jobs.lock().unwrap().push(Job {
            id,
            spec: spec.clone(),
            status: Status::Queued,
            progress: progress.clone(),
            handle: None,
            finished: 0,
            unchanged: false,
        });
        self.start(id, spec, progress, provider, source);
        id
    }

    fn start(
        self: &Arc<Self>,
        id: u64,
        spec: Spec,
        progress: Progress,
        provider: Arc<dyn StorageProvider>,
        source: Option<Arc<dyn StorageProvider>>,
    ) {
        let this = self.clone();
        let handle = spawn(async move {
            let _slot = this.slots.clone().acquire_owned().await;
            if !this.update(id, Status::Running, false) {
                return;
            }
            let result = this
                .execute(&spec, &*provider, source.as_ref(), &progress)
                .await;
            let (status, unchanged) = match result {
                Ok(sent) => (Status::Done, !sent),
                Err(err) => {
                    tracing::warn!("Transfer of '{}' failed: {err:#}", spec.remote);
                    let error = format!("{err:#}");
                    (Status::Failed { error }, false)
                }
            };
            this.update(id, status, unchanged);
        });
        if let Some(job) = self.jobs.lock().unwrap().iter_mut().find(|j| j.id == id) {
            job.handle = Some(handle);
        }
    }

    /// Moves an active job to `status`; returns false if it is no longer active
    /// (cancelled meanwhile), so a late result never overrides a cancellation.
    fn update(&self, id: u64, status: Status, unchanged: bool) -> bool {
        let mut jobs = self.jobs.lock().unwrap();
        match jobs.iter_mut().find(|j| j.id == id) {
            Some(job) if job.status.is_active() => {
                if !status.is_active() {
                    job.finished = self.finished_seq.fetch_add(1, Ordering::Relaxed);
                }
                job.status = status;
                job.unchanged = unchanged;
                true
            }
            _ => false,
        }
    }

    /// Runs a transfer. Returns false when nothing had to be sent: the
    /// other side already had the same file.
    async fn execute(
        &self,
        spec: &Spec,
        provider: &dyn StorageProvider,
        source: Option<&Arc<dyn StorageProvider>>,
        progress: &Progress,
    ) -> Result<bool> {
        tracing::debug!(
            "{:?} '{}' <-> '{}' ({})",
            spec.direction,
            spec.remote,
            spec.local.display(),
            spec.account_name
        );
        if self.is_unchanged(spec, provider).await {
            tracing::info!("  [=] Unchanged: {}", spec.remote);
            // Same size on both sides.
            let size = spec.known.as_ref().map_or(0, |k| k.size);
            progress.set_total(size);
            progress.set_done(size);
            return Ok(false);
        }
        match spec.direction {
            Direction::Download => {
                if let Some(dir) = spec.local.parent() {
                    tokio::fs::create_dir_all(dir)
                        .await
                        .with_context(|| format!("Failed to create '{}'", dir.display()))?;
                }
                provider
                    .download_file(&spec.remote, &spec.local, progress)
                    .await?;
            }
            Direction::Upload => {
                let (dir, _) = split_remote(&spec.remote);
                self.ensure_dir(&spec.account_id, &dir, provider).await?;
                provider
                    .upload_file(&spec.local, &spec.remote, progress)
                    .await?;
            }
            Direction::Copy => {
                let (Some(from), Some(source)) = (&spec.from, source) else {
                    anyhow::bail!("The source of the copy is unknown");
                };
                let (dir, _) = split_remote(&spec.remote);
                self.ensure_dir(&spec.account_id, &dir, provider).await?;
                tokio::fs::create_dir_all(&self.temp_dir)
                    .await
                    .with_context(|| format!("Failed to create '{}'", self.temp_dir.display()))?;
                let temp = self.temp_dir.join(format!(
                    "copy-{}",
                    self.temp_seq.fetch_add(1, Ordering::Relaxed)
                ));
                let same_storage = from.account_id == spec.account_id;
                let method = copy_file(
                    source,
                    &from.file,
                    provider,
                    &spec.remote,
                    same_storage,
                    &temp,
                    progress,
                )
                .await?;
                tracing::info!("  [+] Copied {} ({method:?})", spec.remote);
            }
        }
        Ok(true)
    }

    /// Whether the target already holds the file, compared the way backups
    /// compare: size, then MD5 or the date. A failed comparison means "send".
    async fn is_unchanged(&self, spec: &Spec, provider: &dyn StorageProvider) -> bool {
        let Some(remote) = &spec.known else {
            return false;
        };
        let result = match spec.direction {
            Direction::Download => sync::local_has(remote, &spec.local).await,
            Direction::Upload => {
                let Ok(meta) = tokio::fs::metadata(&spec.local).await else {
                    return false;
                };
                let local = LocalFile {
                    path: spec.local.clone(),
                    relative: String::new(),
                    size: meta.len(),
                    modified: meta.modified().ok(),
                };
                sync::cloud_has(provider, remote, &local, false).await
            }
            Direction::Copy => match &spec.from {
                Some(from) => Ok(same_content(&from.file, remote)),
                None => return false,
            },
        };
        result.unwrap_or_else(|err| {
            tracing::debug!("Cannot compare '{}': {err:#}", spec.remote);
            false
        })
    }

    pub async fn ensure_dir(
        &self,
        account_id: &str,
        dir: &str,
        provider: &dyn StorageProvider,
    ) -> Result<()> {
        let mut ensured = self.ensured.lock().await;
        if ensured.contains(&(account_id.to_string(), dir.to_string())) {
            return Ok(());
        }
        provider
            .ensure_dir(dir)
            .await
            .with_context(|| format!("Failed to create folder '{dir}'"))?;
        // Its parents exist now as well.
        let mut path = dir.to_string();
        loop {
            let parent = split_remote(&path).0;
            let top = parent == path;
            ensured.insert((account_id.to_string(), path));
            if top {
                break;
            }
            path = parent;
        }
        Ok(())
    }

    /// Forgets created folders at or below `dir`, after it was renamed or deleted.
    pub async fn forget_dir(&self, account_id: &str, dir: &str) {
        let prefix = format!("{}/", dir.trim_end_matches('/'));
        self.ensured.lock().await.retain(|(account, path)| {
            account != account_id || (path != dir && !path.starts_with(&prefix))
        });
    }

    pub fn list(&self) -> TransferList {
        self.list_filtered(TransferFilter::All)
    }

    pub fn list_filtered(&self, filter: TransferFilter) -> TransferList {
        let jobs = self.jobs.lock().unwrap();
        let mut counts = Counts::default();
        for job in jobs.iter() {
            *match job.status {
                Status::Queued => &mut counts.queued,
                Status::Running => &mut counts.running,
                Status::Done => &mut counts.done,
                Status::Failed { .. } => &mut counts.failed,
                Status::Cancelled => &mut counts.cancelled,
            } += 1;
        }
        let mut finished: Vec<u64> = jobs.iter().map(|j| j.finished).filter(|&f| f > 0).collect();
        let oldest_shown = if finished.len() > SHOWN_FINISHED {
            *finished
                .select_nth_unstable_by(SHOWN_FINISHED - 1, |a, b| b.cmp(a))
                .1
        } else {
            0
        };
        let mut queued = 0;
        let shown: Vec<&Job> = jobs
            .iter()
            .filter(|job| match job.status {
                Status::Running => true,
                Status::Queued => {
                    queued += 1;
                    queued <= SHOWN_QUEUED
                }
                _ => job.finished >= oldest_shown,
            })
            .collect();
        let view_items = match filter {
            TransferFilter::All => None,
            TransferFilter::Active => Some(
                shown
                    .iter()
                    .filter(|j| j.status.is_active())
                    .map(|j| j.view())
                    .collect(),
            ),
            _ => {
                let mut matches: Vec<&Job> =
                    jobs.iter().filter(|j| filter.accepts(&j.status)).collect();
                matches.sort_unstable_by_key(|j| std::cmp::Reverse(j.finished));
                Some(
                    matches
                        .into_iter()
                        .take(SHOWN_FINISHED)
                        .map(Job::view)
                        .collect(),
                )
            }
        };
        let items = shown.into_iter().map(Job::view).collect();
        TransferList {
            items,
            counts,
            view_items,
        }
    }

    /// Includes failures outside the slice sent to the page. Cancelled jobs
    /// are retried only explicitly, never by "retry all errors".
    pub fn failed_ids(&self) -> Vec<u64> {
        self.jobs
            .lock()
            .unwrap()
            .iter()
            .filter(|j| matches!(j.status, Status::Failed { .. }))
            .map(|j| j.id)
            .collect()
    }

    /// A summary of the active transfers for the Android progress notification.
    #[cfg_attr(desktop, allow(dead_code))]
    pub fn activity(&self) -> Activity {
        let jobs = self.jobs.lock().unwrap();
        let mut activity = Activity::default();
        for job in jobs.iter().filter(|j| j.status.is_active()) {
            activity.active += 1;
            if job.spec.direction != Direction::Download {
                activity.uploads += 1;
            }
            if job.status == Status::Running {
                activity.done_bytes += job.progress.done();
                activity.total_bytes += job.progress.total();
                if activity.name.is_empty() {
                    activity.name = split_remote(&job.spec.remote).1;
                }
            }
        }
        activity
    }

    /// Stops a queued or running transfer. Dropping the task's future cancels
    /// the provider call, which cleans up partial data.
    pub fn cancel(&self, id: u64) {
        let mut jobs = self.jobs.lock().unwrap();
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id && j.status.is_active()) {
            job.status = Status::Cancelled;
            job.finished = self.finished_seq.fetch_add(1, Ordering::Relaxed);
            if let Some(handle) = job.handle.take() {
                handle.abort();
            }
        }
    }

    pub fn cancel_all(&self) {
        let ids: Vec<u64> = self
            .jobs
            .lock()
            .unwrap()
            .iter()
            .filter(|j| j.status.is_active())
            .map(|j| j.id)
            .collect();
        for id in ids {
            self.cancel(id);
        }
    }

    /// Requeues a failed or cancelled transfer. Returns its spec so the
    /// caller can resolve the providers, or `None` if there is nothing to retry.
    pub fn retry_spec(&self, id: u64) -> Option<Spec> {
        let jobs = self.jobs.lock().unwrap();
        jobs.iter()
            .find(|j| j.id == id && matches!(j.status, Status::Failed { .. } | Status::Cancelled))
            .map(|j| j.spec.clone())
    }

    /// `source` is the provider of the copy's source; `None` for others.
    pub fn retry(
        self: &Arc<Self>,
        id: u64,
        provider: Arc<dyn StorageProvider>,
        source: Option<Arc<dyn StorageProvider>>,
    ) -> bool {
        let (spec, progress) = {
            let mut jobs = self.jobs.lock().unwrap();
            let Some(job) = jobs
                .iter_mut()
                .find(|j| j.id == id && !j.status.is_active() && j.status != Status::Done)
            else {
                return false;
            };
            job.status = Status::Queued;
            job.finished = 0;
            job.unchanged = false;
            job.progress.set_done(0);
            job.progress.set_attempt(1);
            (job.spec.clone(), job.progress.clone())
        };
        self.start(id, spec, progress, provider, source);
        true
    }

    /// Removes every finished transfer (done, failed or cancelled) from the list.
    pub fn clear_finished(&self) {
        self.jobs.lock().unwrap().retain(|j| j.status.is_active());
    }
}

/// Whether two cloud files hold the same data: the same size, and the same
/// MD5 where both clouds keep one, else the same date (copies keep it).
fn same_content(a: &RemoteFileInfo, b: &RemoteFileInfo) -> bool {
    if a.is_dir || b.is_dir || a.size != b.size {
        return false;
    }
    match (&a.md5, &b.md5, a.modified, b.modified) {
        (Some(x), Some(y), _, _) => x.eq_ignore_ascii_case(y),
        (_, _, Some(x), Some(y)) => (x - y).num_seconds().abs() <= 2,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(id: u64, status: Status, finished: u64) -> Job {
        Job {
            id,
            spec: Spec {
                direction: Direction::Upload,
                account_id: "a".into(),
                account_name: "A".into(),
                local: PathBuf::from("x"),
                remote: format!("/f{id}"),
                size: 0,
                known: None,
                from: None,
            },
            status,
            progress: Progress::new(),
            handle: None,
            finished,
            unchanged: false,
        }
    }

    #[test]
    fn copies_compare_by_md5_then_date() {
        let file = |size: u64, md5: Option<&str>, secs: Option<i64>| RemoteFileInfo {
            name: "a".into(),
            path: "/a".into(),
            is_dir: false,
            size,
            md5: md5.map(str::to_string),
            modified: secs.and_then(|s| chrono::DateTime::from_timestamp(s, 0)),
        };
        assert!(same_content(
            &file(5, Some("AB"), None),
            &file(5, Some("ab"), None)
        ));
        assert!(!same_content(
            &file(5, Some("ab"), Some(0)),
            &file(5, Some("cd"), Some(0))
        ));
        assert!(same_content(
            &file(5, None, Some(100)),
            &file(5, Some("ab"), Some(101))
        ));
        assert!(!same_content(
            &file(5, None, Some(100)),
            &file(6, None, Some(100))
        ));
        assert!(!same_content(&file(5, None, None), &file(5, None, None)));
    }

    #[test]
    fn filtered_errors_include_old_failures_and_bulk_retry_skips_cancelled() {
        let tmp = tempfile::tempdir().unwrap();
        let transfers = Transfers::new(tmp.path().join("copy"));
        {
            let mut jobs = transfers.jobs.lock().unwrap();
            jobs.extend((1..=150).map(|id| {
                job(
                    id,
                    Status::Failed {
                        error: "offline".into(),
                    },
                    id,
                )
            }));
            jobs.push(job(151, Status::Cancelled, 151));
            jobs.extend((152..=351).map(|id| job(id, Status::Done, id)));
            jobs.push(job(352, Status::Running, 0));
            jobs.push(job(353, Status::Queued, 0));
        }
        let normal = transfers.list();
        assert!(normal
            .items
            .iter()
            .all(|j| !matches!(j.status, Status::Failed { .. })));
        let filtered = transfers.list_filtered(TransferFilter::Failed);
        assert_eq!(filtered.counts.failed, 150);
        assert_eq!(filtered.counts.done, 200);
        let errors = filtered.view_items.unwrap();
        assert_eq!(errors.len(), SHOWN_FINISHED);
        assert_eq!(errors[0].id, 150);
        assert_eq!(errors.last().unwrap().id, 51);
        assert!(errors
            .iter()
            .all(|j| matches!(j.status, Status::Failed { .. })));
        // The ordinary slice still carries active jobs and recent completions.
        assert!(filtered.items.iter().any(|j| j.id == 352));
        assert!(filtered.items.iter().any(|j| j.id == 351));
        let retry = transfers.failed_ids();
        assert_eq!(retry, (1..=150).collect::<Vec<_>>());
        assert!(transfers.retry_spec(352).is_none());
        let active = transfers
            .list_filtered(TransferFilter::Active)
            .view_items
            .unwrap();
        assert_eq!(active.iter().map(|j| j.id).collect::<Vec<_>>(), [352, 353]);
        let cancelled = transfers
            .list_filtered(TransferFilter::Cancelled)
            .view_items
            .unwrap();
        assert_eq!(cancelled.iter().map(|j| j.id).collect::<Vec<_>>(), [151]);
    }

    #[test]
    fn list_sends_the_head_of_a_long_queue() {
        let transfers = Transfers::new(std::env::temp_dir().join("belka-test-copy"));
        {
            let mut jobs = transfers.jobs.lock().unwrap();
            // 150 finished in reverse order, so the latest are the first ones.
            jobs.extend((1..=150).map(|id| job(id, Status::Done, 151 - id)));
            jobs.push(job(151, Status::Running, 0));
            jobs.extend((152..=400).map(|id| job(id, Status::Queued, 0)));
        }
        let list = transfers.list();
        assert_eq!(
            (list.counts.done, list.counts.running, list.counts.queued),
            (150, 1, 249)
        );
        let ids: Vec<u64> = list.items.iter().map(|t| t.id).collect();
        assert_eq!(ids.len(), SHOWN_FINISHED + 1 + SHOWN_QUEUED);
        assert_eq!(ids[0], 1);
        assert_eq!(ids[SHOWN_FINISHED - 1], SHOWN_FINISHED as u64);
        assert_eq!(ids[SHOWN_FINISHED], 151);
        assert_eq!(*ids.last().unwrap(), 151 + SHOWN_QUEUED as u64);
    }
}
