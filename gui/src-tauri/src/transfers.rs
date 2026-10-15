//! The transfer queue: uploads and downloads run as background tasks, at most
//! [`MAX_PARALLEL`] at a time. The frontend polls [`Transfers::list`] and reads
//! progress counters directly, so running transfers never block on the UI.

use anyhow::{Context, Result};
use cloud_uploader_core::utils::path::split_remote;
use cloud_uploader_core::{Progress, StorageProvider};
use serde::Serialize;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::async_runtime::{spawn, JoinHandle};
use tokio::sync::Semaphore;

const MAX_PARALLEL: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Upload,
    Download,
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
}

struct Job {
    id: u64,
    spec: Spec,
    status: Status,
    progress: Progress,
    handle: Option<JoinHandle<()>>,
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
    done: u64,
    total: u64,
}

pub struct Transfers {
    jobs: Mutex<Vec<Job>>,
    next_id: AtomicU64,
    slots: Arc<Semaphore>,
    /// Remote folders already created, as (account id, path). Folder creation
    /// is serialized so parallel uploads never create the same folder twice
    /// (Google Drive would happily make duplicates).
    ensured: tokio::sync::Mutex<HashSet<(String, String)>>,
}

impl Transfers {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            jobs: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
            slots: Arc::new(Semaphore::new(MAX_PARALLEL)),
            ensured: tokio::sync::Mutex::new(HashSet::new()),
        })
    }

    pub fn enqueue(self: &Arc<Self>, spec: Spec, provider: Arc<dyn StorageProvider>) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let progress = Progress::new();
        progress.set_total(spec.size);
        self.jobs.lock().unwrap().push(Job {
            id,
            spec: spec.clone(),
            status: Status::Queued,
            progress: progress.clone(),
            handle: None,
        });
        self.start(id, spec, progress, provider);
        id
    }

    fn start(
        self: &Arc<Self>,
        id: u64,
        spec: Spec,
        progress: Progress,
        provider: Arc<dyn StorageProvider>,
    ) {
        let this = self.clone();
        let handle = spawn(async move {
            let _slot = this.slots.clone().acquire_owned().await;
            if !this.update(id, Status::Running) {
                return;
            }
            let status = match this.execute(&spec, &*provider, &progress).await {
                Ok(()) => Status::Done,
                Err(err) => {
                    tracing::warn!("Transfer of '{}' failed: {err:#}", spec.remote);
                    Status::Failed {
                        error: format!("{err:#}"),
                    }
                }
            };
            this.update(id, status);
        });
        if let Some(job) = self.jobs.lock().unwrap().iter_mut().find(|j| j.id == id) {
            job.handle = Some(handle);
        }
    }

    /// Moves an active job to `status`; returns false if it is no longer active
    /// (cancelled meanwhile), so a late result never overrides a cancellation.
    fn update(&self, id: u64, status: Status) -> bool {
        let mut jobs = self.jobs.lock().unwrap();
        match jobs.iter_mut().find(|j| j.id == id) {
            Some(job) if job.status.is_active() => {
                job.status = status;
                true
            }
            _ => false,
        }
    }

    async fn execute(
        &self,
        spec: &Spec,
        provider: &dyn StorageProvider,
        progress: &Progress,
    ) -> Result<()> {
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
        }
        Ok(())
    }

    pub async fn ensure_dir(
        &self,
        account_id: &str,
        dir: &str,
        provider: &dyn StorageProvider,
    ) -> Result<()> {
        let key = (account_id.to_string(), dir.to_string());
        let mut ensured = self.ensured.lock().await;
        if !ensured.contains(&key) {
            provider
                .ensure_dir(dir)
                .await
                .with_context(|| format!("Failed to create folder '{dir}'"))?;
            ensured.insert(key);
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

    pub fn list(&self) -> Vec<TransferView> {
        self.jobs
            .lock()
            .unwrap()
            .iter()
            .map(|job| {
                let (remote_dir, name) = split_remote(&job.spec.remote);
                TransferView {
                    id: job.id,
                    direction: job.spec.direction,
                    account_id: job.spec.account_id.clone(),
                    account_name: job.spec.account_name.clone(),
                    name,
                    local_path: job.spec.local.display().to_string(),
                    remote_path: job.spec.remote.clone(),
                    remote_dir,
                    status: job.status.clone(),
                    done: job.progress.done(),
                    total: job.progress.total(),
                }
            })
            .collect()
    }

    /// Stops a queued or running transfer. Dropping the task's future cancels
    /// the provider call, which cleans up partial data.
    pub fn cancel(&self, id: u64) {
        let mut jobs = self.jobs.lock().unwrap();
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id && j.status.is_active()) {
            job.status = Status::Cancelled;
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
    /// caller can resolve the provider, or `None` if there is nothing to retry.
    pub fn retry_spec(&self, id: u64) -> Option<Spec> {
        let jobs = self.jobs.lock().unwrap();
        jobs.iter()
            .find(|j| j.id == id && matches!(j.status, Status::Failed { .. } | Status::Cancelled))
            .map(|j| j.spec.clone())
    }

    pub fn retry(self: &Arc<Self>, id: u64, provider: Arc<dyn StorageProvider>) {
        let (spec, progress) = {
            let mut jobs = self.jobs.lock().unwrap();
            let Some(job) = jobs
                .iter_mut()
                .find(|j| j.id == id && !j.status.is_active() && j.status != Status::Done)
            else {
                return;
            };
            job.status = Status::Queued;
            job.progress.set_done(0);
            (job.spec.clone(), job.progress.clone())
        };
        self.start(id, spec, progress, provider);
    }

    /// Removes every finished transfer (done, failed or cancelled) from the list.
    pub fn clear_finished(&self) {
        self.jobs.lock().unwrap().retain(|j| j.status.is_active());
    }
}
