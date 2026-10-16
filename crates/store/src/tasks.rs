//! Backup tasks: a local folder copied into a cloud folder on a schedule.
//! A run uploads new and changed files only and never deletes anything in
//! the cloud. Files already backed up are remembered in an index per task,
//! so a run over thousands of unchanged photos is a folder walk.
//!
//! Who runs them: WorkManager on Android (through JNI, also while the page
//! is not running), a timer in the desktop app while it is open, and
//! `belka task run` from a system scheduler. Runs in one process share the
//! registry of [`live`] progress, so the page sees a run started elsewhere.

use crate::accounts::AccountSource;
use anyhow::{bail, Context, Result};
use belka_core::sync::{self, Filter, Index, LocalFile, Observer, Outcome, UploadOptions};
use belka_core::{Progress, StorageProvider};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

const DEFAULT_INTERVAL_HOURS: u32 = 24;
/// Runs kept in a task's history.
const HISTORY: usize = 20;
/// Failed files kept in a run's result.
const KEPT_FAILURES: usize = 20;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    /// Empty for a task that is not saved yet.
    #[serde(default)]
    pub id: String,
    pub name: String,
    /// The local folder.
    pub local: String,
    pub account_id: String,
    /// The cloud folder the contents of `local` go into.
    pub remote: String,
    #[serde(default = "default_interval")]
    pub interval_hours: u32,
    /// Android: wait for an unmetered network.
    #[serde(default = "yes")]
    pub wifi_only: bool,
    /// Android: wait for the charger.
    #[serde(default)]
    pub charging_only: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Patterns of files to take; empty takes all.
    #[serde(default)]
    pub include: Vec<String>,
    /// Patterns of files and folders to leave out.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Delete local files older than this many days once they are in the
    /// cloud, to free space on a phone.
    #[serde(default)]
    pub free_after_days: Option<u32>,
    /// The latest runs, newest first; a run is there from its start, and one
    /// without `finished` is going on or was killed. Kept by the store;
    /// whatever the page sends is ignored.
    #[serde(default)]
    pub history: Vec<RunResult>,
}

fn default_interval() -> u32 {
    DEFAULT_INTERVAL_HOURS
}

fn yes() -> bool {
    true
}

impl Task {
    /// A task to fill in: daily, over Wi‑Fi, hidden files left out.
    pub fn new(local: &str, account_id: &str, remote: &str) -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            local: local.to_string(),
            account_id: account_id.to_string(),
            remote: remote.to_string(),
            interval_hours: DEFAULT_INTERVAL_HOURS,
            wifi_only: true,
            charging_only: false,
            enabled: true,
            include: Vec::new(),
            exclude: Vec::new(),
            free_after_days: None,
            history: Vec::new(),
        }
    }

    pub fn last(&self) -> Option<&RunResult> {
        self.history.first()
    }

    /// Whether the task should run at `now`: enabled, and never run to the
    /// end or the interval has passed since the last such run started. A
    /// killed run does not count, the next one continues its work.
    pub fn is_due(&self, now: DateTime<Utc>) -> bool {
        if !self.enabled {
            return false;
        }
        let finished = self.history.iter().find(|r| r.finished.is_some());
        match finished.and_then(|r| r.started) {
            None => true,
            Some(started) => now - started >= chrono::Duration::hours(self.interval_hours.into()),
        }
    }

    fn filter(&self) -> Filter {
        Filter::new(&self.include, &self.exclude, true)
    }

    /// Names the source and the destination; an index made for another
    /// target is not used.
    fn target(&self) -> String {
        format!("{}\n{}\n{}", self.local, self.account_id, self.remote)
    }

    fn moved_from(&self, old: &Task) -> bool {
        self.target() != old.target()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RunResult {
    pub started: Option<DateTime<Utc>>,
    pub finished: Option<DateTime<Utc>>,
    /// Files uploaded by the run.
    pub uploaded: u64,
    pub bytes: u64,
    /// Files that were in the cloud already.
    #[serde(default)]
    pub unchanged: u64,
    /// Files that failed to upload.
    pub failed: u64,
    /// The first failed files with the reason.
    #[serde(default)]
    pub failures: Vec<(String, String)>,
    /// Local files deleted to free space, and their size.
    #[serde(default)]
    pub freed: u64,
    #[serde(default)]
    pub freed_bytes: u64,
    /// Why the run failed as a whole.
    pub error: Option<String>,
    #[serde(default)]
    pub cancelled: bool,
}

/// A task with what it is doing now, for the page.
#[derive(Serialize)]
pub struct TaskView {
    #[serde(flatten)]
    pub task: Task,
    pub running: Option<LiveView>,
}

/// Serializes changes of the task list: the page and a background run may
/// both write it.
static STORE_LOCK: Mutex<()> = Mutex::new(());

/// Tasks in `backups.json`, indexes in `backup-index/<id>.json`.
pub struct TaskStore {
    dir: PathBuf,
}

impl TaskStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn file(&self) -> PathBuf {
        self.dir.join("backups.json")
    }

    fn index_file(&self, id: &str) -> PathBuf {
        self.dir.join("backup-index").join(format!("{id}.json"))
    }

    pub fn list(&self) -> Result<Vec<Task>> {
        let file = self.file();
        match std::fs::read_to_string(&file) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("Failed to parse '{}'", file.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(err) => Err(err).with_context(|| format!("Failed to read '{}'", file.display())),
        }
    }

    pub fn views(&self) -> Result<Vec<TaskView>> {
        Ok(self
            .list()?
            .into_iter()
            .map(|task| TaskView {
                running: live(&task.id),
                task,
            })
            .collect())
    }

    pub fn task(&self, id: &str) -> Result<Task> {
        self.list()?
            .into_iter()
            .find(|t| t.id == id)
            .context("Backup task not found")
    }

    /// A task by its name (any case) or id.
    pub fn find(&self, name_or_id: &str) -> Result<Task> {
        let tasks = self.list()?;
        let wanted = name_or_id.trim().to_lowercase();
        tasks
            .iter()
            .find(|t| t.id == name_or_id.trim() || t.name.to_lowercase() == wanted)
            .cloned()
            .with_context(|| format!("No backup task '{}'", name_or_id.trim()))
    }

    fn write(&self, tasks: &[Task]) -> Result<()> {
        crate::write_json(&self.file(), &tasks)
    }

    /// Adds a task (an empty id) or replaces one, keeping its history.
    /// A task that now copies another folder, or into another place,
    /// starts with an empty history and index.
    pub fn save(&self, mut task: Task) -> Result<Task> {
        task.local = task.local.trim().trim_end_matches(['/', '\\']).to_string();
        task.remote = belka_core::utils::path::normalize_remote_dir(&task.remote);
        task.name = task.name.trim().to_string();
        let clean = |list: &mut Vec<String>| {
            list.iter_mut().for_each(|p| *p = p.trim().to_string());
            list.retain(|p| !p.is_empty());
        };
        clean(&mut task.include);
        clean(&mut task.exclude);
        if task.local.is_empty() {
            bail!("Choose the folder to back up");
        }
        if task.account_id.is_empty() {
            bail!("Choose the cloud to back up to");
        }
        if task.name.is_empty() {
            task.name = Path::new(&task.local)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| task.local.clone());
        }
        task.interval_hours = task.interval_hours.max(1);
        task.free_after_days = task.free_after_days.filter(|d| *d > 0);

        let _guard = STORE_LOCK.lock().unwrap();
        let mut tasks = self.list()?;
        match tasks
            .iter_mut()
            .find(|t| !task.id.is_empty() && t.id == task.id)
        {
            Some(old) => {
                task.history = if task.moved_from(old) {
                    Vec::new()
                } else {
                    std::mem::take(&mut old.history)
                };
                *old = task.clone();
            }
            None => {
                task.id = new_id();
                task.history = Vec::new();
                tasks.push(task.clone());
            }
        }
        self.write(&tasks)?;
        Ok(task)
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        let _guard = STORE_LOCK.lock().unwrap();
        let mut tasks = self.list()?;
        tasks.retain(|t| t.id != id);
        self.write(&tasks)?;
        Index::forget(&self.index_file(id));
        Ok(())
    }

    /// Puts a run into the history when it starts, so a run the system
    /// kills is still seen, and replaces it with the result at the end.
    fn record(&self, id: &str, result: &RunResult) -> Result<()> {
        let _guard = STORE_LOCK.lock().unwrap();
        let mut tasks = self.list()?;
        if let Some(task) = tasks.iter_mut().find(|t| t.id == id) {
            match task
                .history
                .iter_mut()
                .find(|r| r.started == result.started)
            {
                Some(entry) => *entry = result.clone(),
                None => {
                    task.history.insert(0, result.clone());
                    task.history.truncate(HISTORY);
                }
            }
            self.write(&tasks)?;
        }
        Ok(())
    }

    /// Names of the tasks that back up into the account.
    pub fn using_account(&self, account_id: &str) -> Vec<String> {
        self.list()
            .unwrap_or_default()
            .into_iter()
            .filter(|t| t.account_id == account_id)
            .map(|t| t.name)
            .collect()
    }
}

fn new_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("b{nanos:x}")
}

// ---- runs in progress ----

/// What a run is doing, polled by the page and the Android notification.
#[derive(Default)]
struct Live {
    scanning: bool,
    files_total: u64,
    files_done: u64,
    bytes_total: u64,
    /// Bytes of the files finished so far.
    bytes_finished: u64,
    /// Files being uploaded, by their local path.
    uploading: Vec<(PathBuf, Progress)>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LiveView {
    pub scanning: bool,
    pub files_total: u64,
    pub files_done: u64,
    pub bytes_total: u64,
    pub bytes_done: u64,
    /// The file being uploaded.
    pub current: String,
}

struct Running {
    live: Arc<Mutex<Live>>,
    cancel: watch::Sender<bool>,
}

static RUNNING: Mutex<Option<HashMap<String, Running>>> = Mutex::new(None);

fn with_running<T>(f: impl FnOnce(&mut HashMap<String, Running>) -> T) -> T {
    f(RUNNING.lock().unwrap().get_or_insert_with(HashMap::new))
}

pub fn live(id: &str) -> Option<LiveView> {
    with_running(|running| {
        let live = running.get(id)?.live.lock().unwrap();
        let uploading: u64 = live.uploading.iter().map(|(_, p)| p.done()).sum();
        Some(LiveView {
            scanning: live.scanning,
            files_total: live.files_total,
            files_done: live.files_done,
            bytes_total: live.bytes_total,
            bytes_done: live.bytes_finished + uploading,
            current: live
                .uploading
                .first()
                .and_then(|(path, _)| path.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
        })
    })
}

pub fn is_running(id: &str) -> bool {
    with_running(|running| running.contains_key(id))
}

/// Stops a run; what it uploaded so far stays remembered.
pub fn cancel(id: &str) {
    with_running(|running| {
        if let Some(run) = running.get(id) {
            let _ = run.cancel.send(true);
        }
    });
}

/// Registers a run of the task; `None` if one is going on already.
struct RunGuard {
    id: String,
    live: Arc<Mutex<Live>>,
    cancelled: watch::Receiver<bool>,
}

impl RunGuard {
    fn start(id: &str) -> Option<Self> {
        with_running(|running| {
            if running.contains_key(id) {
                return None;
            }
            let (cancel, cancelled) = watch::channel(false);
            let live = Arc::new(Mutex::new(Live {
                scanning: true,
                ..Live::default()
            }));
            running.insert(
                id.to_string(),
                Running {
                    live: live.clone(),
                    cancel,
                },
            );
            Some(Self {
                id: id.to_string(),
                live,
                cancelled,
            })
        })
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        with_running(|running| running.remove(&self.id));
    }
}

/// Feeds a transfer's progress into the registry.
struct LiveObserver<'a>(&'a Mutex<Live>);

impl Observer for LiveObserver<'_> {
    fn started(&self, file: &LocalFile, progress: &Progress) {
        self.0
            .lock()
            .unwrap()
            .uploading
            .push((file.path.clone(), progress.clone()));
    }

    fn finished(&self, file: &LocalFile, _outcome: &Outcome) {
        let mut live = self.0.lock().unwrap();
        live.uploading.retain(|(path, _)| path != &file.path);
        live.files_done += 1;
        live.bytes_finished += file.size;
    }
}

// ---- a run ----

/// What a run did, with the local files it deleted to free space (Android
/// asks the media scanner to forget them).
pub struct Run {
    pub result: RunResult,
    pub freed: Vec<PathBuf>,
}

/// Gives the provider of an account.
pub type ProviderFn<'a> = dyn Fn(&str) -> Result<Arc<dyn StorageProvider>> + Sync + 'a;

/// Runs a task to the end and records the result.
pub async fn run(store: &TaskStore, id: &str, provider: &ProviderFn<'_>) -> Result<Run> {
    let task = store.task(id)?;
    let Some(mut guard) = RunGuard::start(id) else {
        bail!("The backup «{}» is running already", task.name);
    };
    let mut result = RunResult {
        started: Some(Utc::now()),
        ..RunResult::default()
    };
    store.record(id, &result)?;
    let index = Mutex::new(Index::load(store.index_file(id), &task.target()));
    let mut freed = Vec::new();

    let work = async {
        let provider = provider(&task.account_id)?;
        back_up(
            &task,
            &*provider,
            &index,
            &guard.live,
            &mut result,
            &mut freed,
        )
        .await
    };
    let outcome = tokio::select! {
        outcome = work => outcome,
        _ = guard.cancelled.wait_for(|c| *c) => {
            result.cancelled = true;
            Ok(())
        }
    };
    if let Err(err) = outcome {
        result.error = Some(format!("{err:#}"));
    }
    if let Err(err) = index.lock().unwrap().save() {
        tracing::warn!("{err:#}");
    }
    result.finished = Some(Utc::now());
    store.record(id, &result)?;
    tracing::info!(
        "Backup «{}»: {} uploaded, {} failed, {} freed{}",
        task.name,
        result.uploaded,
        result.failed,
        result.freed,
        result
            .error
            .as_deref()
            .map(|e| format!(": {e}"))
            .unwrap_or_default()
    );
    Ok(Run { result, freed })
}

async fn back_up(
    task: &Task,
    provider: &dyn StorageProvider,
    index: &Mutex<Index>,
    live: &Mutex<Live>,
    result: &mut RunResult,
    freed: &mut Vec<PathBuf>,
) -> Result<()> {
    let root = PathBuf::from(&task.local);
    let filter = task.filter();
    let tree = tokio::task::spawn_blocking(move || sync::scan(&root, &filter))
        .await
        .context("Failed to scan local files")??;
    for error in tree.errors.iter().take(5) {
        tracing::warn!("Skipped: {error}");
    }
    {
        let index = index.lock().unwrap();
        let mut live = live.lock().unwrap();
        let pending: Vec<&LocalFile> = tree.files.iter().filter(|f| !index.is_current(f)).collect();
        live.scanning = false;
        live.files_total = pending.len() as u64;
        live.bytes_total = pending.iter().map(|f| f.size).sum();
    }

    let summary = sync::upload(
        provider,
        &tree,
        &task.remote,
        Some(index),
        // Several phones may back up into one folder, and two cameras
        // easily name photos alike.
        &UploadOptions {
            keep_both: true,
            ..UploadOptions::default()
        },
        &LiveObserver(live),
    )
    .await?;
    result.uploaded = summary.transferred;
    result.bytes = summary.bytes;
    result.unchanged = summary.unchanged;
    result.failed = summary.failed;
    result.failures = summary.failures.into_iter().take(KEPT_FAILURES).collect();
    index.lock().unwrap().retain_scanned(&tree);

    if let Some(days) = task.free_after_days {
        let cutoff = SystemTime::now() - Duration::from_secs(u64::from(days) * 86_400);
        let old: Vec<&LocalFile> = {
            let index = index.lock().unwrap();
            tree.files
                .iter()
                .filter(|f| index.is_current(f) && f.modified.is_some_and(|m| m < cutoff))
                .collect()
        };
        free_space(provider, &task.remote, &old, index, result, freed).await;
    }
    Ok(())
}

/// Deletes local files that the cloud holds: the same size and the same MD5
/// where the cloud can tell one. The cloud is asked again, the index alone
/// is not proof enough to delete anything.
async fn free_space(
    provider: &dyn StorageProvider,
    remote_root: &str,
    files: &[&LocalFile],
    index: &Mutex<Index>,
    result: &mut RunResult,
    freed: &mut Vec<PathBuf>,
) {
    if files.is_empty() {
        return;
    }
    // A file may be in the cloud under another name, see `keep_both`.
    let cloud_paths: Vec<String> = {
        let index = index.lock().unwrap();
        files
            .iter()
            .map(|f| index.cloud_path(&f.relative).to_string())
            .collect()
    };
    let listing = sync::RemoteListing::new(
        provider,
        remote_root,
        cloud_paths.iter().map(String::as_str),
        sync::DEFAULT_PARALLEL,
    )
    .await;
    for (file, cloud_path) in files.iter().zip(&cloud_paths) {
        let target = belka_core::utils::path::join_remote(remote_root, cloud_path);
        let held = match listing.info(provider, &target).await {
            Ok(Some(remote)) => sync::cloud_has(provider, &remote, file, true).await,
            Ok(None) => Ok(false),
            Err(err) => Err(err),
        };
        match held {
            Ok(true) => match std::fs::remove_file(&file.path) {
                Ok(()) => {
                    tracing::info!("  [-] Freed {}", file.path.display());
                    result.freed += 1;
                    result.freed_bytes += file.size;
                    index.lock().unwrap().remove(&file.relative);
                    freed.push(file.path.clone());
                }
                Err(err) => tracing::warn!("Cannot delete '{}': {err}", file.path.display()),
            },
            Ok(_) => tracing::warn!(
                "'{}' is not in the cloud as expected; kept on the device",
                file.relative
            ),
            Err(err) => tracing::warn!("Cannot check '{}': {err:#}; kept", file.relative),
        }
    }
}

/// Runs a task on its own runtime, for a run outside the app's runtime: a
/// background job that may start in a process where the page never ran,
/// or the CLI. Accounts come from `accounts`.
pub fn run_blocking(
    store: &TaskStore,
    accounts: &(dyn AccountSource + Sync),
    id: &str,
) -> Result<Run> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to start the async runtime")?;
    runtime.block_on(run(store, id, &|account| accounts.provider(account)))
}

#[cfg(test)]
mod tests;
