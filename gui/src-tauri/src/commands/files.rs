//! Local files and the transfer queue: browsing, folder operations,
//! queuing uploads and downloads, and managing the queue.

use crate::error::CmdResult;
use crate::transfers::{CopyFrom, Direction, Spec, TransferFilter, TransferList};
use crate::{local, native, AppState};
use anyhow::{bail, Context, Result};
use belka_core::sync::{self, Filter, RemoteListing};
use belka_core::utils::path::{
    join_remote, normalize_remote_dir, sanitize_file_name, split_remote, unique_path,
};
use belka_core::{RemoteFileInfo, StorageProvider};
use chrono::{DateTime, Utc};
use futures_util::stream::{self, StreamExt};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tauri::State;

/// Opens a local file in the program the system associates with it.
#[tauri::command]
pub fn local_open(app: tauri::AppHandle, path: String) -> CmdResult<()> {
    Ok(native::open_file(&app, &path)?)
}

#[tauri::command]
pub fn local_mkdir(parent: String, name: String) -> CmdResult<String> {
    Ok(local::mkdir(&parent, &name)?)
}

#[tauri::command]
pub fn local_rename(path: String, new_name: String) -> CmdResult<String> {
    Ok(local::rename(&path, &new_name)?)
}

#[tauri::command]
pub async fn local_delete(paths: Vec<String>) -> CmdResult<()> {
    let deleted = tauri::async_runtime::spawn_blocking(move || local::delete(&paths))
        .await
        .map_err(anyhow::Error::from)?;
    Ok(deleted?)
}

/// A cloud item as the page has it from the listing.
#[derive(Deserialize)]
pub struct RemoteItem {
    path: String,
    is_dir: bool,
    size: u64,
    #[serde(default)]
    md5: Option<String>,
    #[serde(default)]
    modified: Option<DateTime<Utc>>,
}

impl RemoteItem {
    fn into_info(self) -> RemoteFileInfo {
        let path = normalize_remote_dir(&self.path);
        RemoteFileInfo {
            name: split_remote(&path).1,
            path,
            is_dir: self.is_dir,
            size: self.size,
            md5: self.md5,
            modified: self.modified,
        }
    }
}

/// Queues downloads of files and whole folders into `local_dir`.
/// Returns the number of queued files.
#[tauri::command]
pub async fn download(
    state: State<'_, AppState>,
    account_id: String,
    items: Vec<RemoteItem>,
    local_dir: String,
) -> CmdResult<usize> {
    let provider = state.provider(&account_id)?;
    let account_name = state.account(&account_id)?.name;
    let files = expand_download(&*provider, items, Path::new(&local_dir)).await?;
    let count = files.len();
    for (remote, local) in files {
        let spec = Spec {
            direction: Direction::Download,
            account_id: account_id.clone(),
            account_name: account_name.clone(),
            local,
            remote: remote.path.clone(),
            size: remote.size,
            known: Some(remote),
            from: None,
        };
        state.transfers.enqueue(spec, provider.clone());
    }
    Ok(count)
}

/// Lists every file under the selected items, mapping cloud names to safe
/// local names. Folders are created locally right away, so empty ones survive.
/// Folders of one level are listed in parallel; the names are assigned in
/// listing order all the same, so `a (2).txt` is always the same file.
async fn expand_download(
    provider: &dyn StorageProvider,
    items: Vec<RemoteItem>,
    local_dir: &Path,
) -> Result<Vec<(RemoteFileInfo, PathBuf)>> {
    let mut files = Vec::new();
    let mut taken = HashSet::new();
    let mut level = Vec::new();
    for item in items {
        let item = item.into_info();
        let local = unique_path(local_dir.join(sanitize_file_name(&item.name)), &mut taken);
        if item.is_dir {
            level.push((item.path, local));
        } else {
            files.push((item, local));
        }
    }
    while !level.is_empty() {
        for (_, local) in &level {
            tokio::fs::create_dir_all(local)
                .await
                .with_context(|| format!("Failed to create '{}'", local.display()))?;
        }
        // Owned paths: a stream over references trips the `Send` checks.
        let remotes: Vec<String> = level.iter().map(|(remote, _)| remote.clone()).collect();
        let listings: Vec<Result<Vec<RemoteFileInfo>>> = stream::iter(remotes)
            .map(|remote| async move { provider.list_dir(&remote).await })
            .buffered(sync::DEFAULT_PARALLEL)
            .collect()
            .await;
        let mut next = Vec::new();
        for ((_, local), listing) in level.iter().zip(listings) {
            for entry in listing? {
                let path = unique_path(local.join(sanitize_file_name(&entry.name)), &mut taken);
                if entry.is_dir {
                    next.push((entry.path, path));
                } else {
                    files.push((entry, path));
                }
            }
        }
        level = next;
    }
    Ok(files)
}

/// Queues uploads of local files and folders into `remote_dir`.
/// Returns the number of queued files.
#[tauri::command]
pub async fn upload(
    state: State<'_, AppState>,
    account_id: String,
    local_paths: Vec<String>,
    remote_dir: String,
) -> CmdResult<usize> {
    let provider = state.provider(&account_id)?;
    let account_name = state.account(&account_id)?.name;
    let remote_dir = normalize_remote_dir(&remote_dir);
    let paths: Vec<PathBuf> = local_paths.into_iter().map(PathBuf::from).collect();
    let target = remote_dir.clone();
    let (files, empty_dirs) =
        tauri::async_runtime::spawn_blocking(move || expand_upload(&paths, &target))
            .await
            .context("Failed to scan local files")??;

    for dir in &empty_dirs {
        state
            .transfers
            .ensure_dir(&account_id, dir, &*provider)
            .await?;
    }
    let targets: Vec<&str> = files.iter().map(|(_, remote)| remote.as_str()).collect();
    let mut existing = existing_targets(&*provider, &remote_dir, &targets).await;
    let count = files.len();
    for (local, remote) in files {
        let spec = Spec {
            direction: Direction::Upload,
            account_id: account_id.clone(),
            account_name: account_name.clone(),
            local,
            known: existing.remove(&remote),
            remote,
            size: 0,
            from: None,
        };
        state.transfers.enqueue(spec, provider.clone());
    }
    Ok(count)
}

/// The cloud files already at the targets of `files`, so that identical
/// ones are not uploaded again. Only the folders under names `remote_dir`
/// already has are listed: anything under a new name is new. Nothing is
/// known if `remote_dir` cannot be listed.
async fn existing_targets(
    provider: &dyn StorageProvider,
    remote_dir: &str,
    targets: &[&str],
) -> HashMap<String, RemoteFileInfo> {
    let top = match provider.list_dir(remote_dir).await {
        Ok(top) => top,
        Err(err) => {
            tracing::debug!("Cannot list '{remote_dir}' ({err:#}); uploading everything");
            return HashMap::new();
        }
    };
    let taken: HashSet<String> = top.iter().map(|item| item.path.clone()).collect();
    let mut found: HashMap<String, RemoteFileInfo> = top
        .into_iter()
        .filter(|item| !item.is_dir)
        .map(|item| (item.path.clone(), item))
        .collect();
    // `a/b/c.txt` for `/Backup/a/b/c.txt` under `/Backup`.
    let relative = |remote: &str| {
        remote
            .strip_prefix(remote_dir.trim_end_matches('/'))
            .unwrap_or(remote)
            .trim_start_matches('/')
            .to_string()
    };
    let nested: Vec<String> = targets
        .iter()
        .map(|remote| relative(remote))
        .filter(|rel| {
            rel.split_once('/')
                .is_some_and(|(top, _)| taken.contains(&join_remote(remote_dir, top)))
        })
        .collect();
    if nested.is_empty() {
        return found;
    }
    let listing = RemoteListing::new(
        provider,
        remote_dir,
        nested.iter().map(String::as_str),
        sync::DEFAULT_PARALLEL,
    )
    .await;
    for rel in &nested {
        let path = join_remote(remote_dir, rel);
        if let Some(item) = listing.listed(&path).filter(|item| !item.is_dir) {
            found.insert(path, item.clone());
        }
    }
    found
}

/// Local files with their remote paths, plus remote folders that contain no files.
type UploadPlan = (Vec<(PathBuf, String)>, Vec<String>);

/// Builds the upload plan for the selected files and folders, walking
/// folders the way backups do.
fn expand_upload(paths: &[PathBuf], remote_dir: &str) -> Result<UploadPlan> {
    let mut files = Vec::new();
    let mut empty_dirs = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .with_context(|| format!("Invalid path '{}'", path.display()))?
            .to_string_lossy();
        let tree = sync::scan(path, &Filter::all())?;
        if !tree.errors.is_empty() {
            bail!(
                "Cannot upload an incomplete folder scan: {}",
                tree.errors
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; ")
            );
        }
        if tree.single_file {
            files.extend(
                tree.files
                    .into_iter()
                    .map(|f| (f.path, join_remote(remote_dir, &name))),
            );
            continue;
        }
        let base = join_remote(remote_dir, &name);
        let nonempty: HashSet<String> = tree
            .files
            .iter()
            .map(|f| f.relative.as_str())
            .chain(tree.dirs.iter().map(String::as_str))
            .filter_map(|path| path.rsplit_once('/').map(|(parent, _)| parent.to_string()))
            .collect();
        empty_dirs.extend(
            tree.dirs
                .iter()
                .filter(|d| !nonempty.contains(*d))
                .map(|d| join_remote(&base, d)),
        );
        if tree.files.is_empty() && tree.dirs.is_empty() {
            empty_dirs.push(base.clone());
        }
        files.extend(
            tree.files
                .into_iter()
                .map(|f| (f.path, join_remote(&base, &f.relative))),
        );
    }
    if files.is_empty() && empty_dirs.is_empty() {
        bail!("Nothing to upload");
    }
    Ok((files, empty_dirs))
}

/// Queues copies of cloud files and whole folders of account `account_id`
/// into folder `target_dir` of account `target_account_id`, which may be
/// the same one. Returns the number of queued files.
#[tauri::command]
pub async fn cloud_copy(
    state: State<'_, AppState>,
    account_id: String,
    items: Vec<RemoteItem>,
    target_account_id: String,
    target_dir: String,
) -> CmdResult<usize> {
    let source = state.provider(&account_id)?;
    let target = state.provider(&target_account_id)?;
    let source_name = state.account(&account_id)?.name;
    let target_name = state.account(&target_account_id)?.name;
    let target_dir = normalize_remote_dir(&target_dir);
    let items: Vec<RemoteFileInfo> = items.into_iter().map(RemoteItem::into_info).collect();
    if account_id == target_account_id {
        check_copy_target(&items, &target_dir)?;
    }

    let (files, empty_dirs) = expand_copy(&*source, items, &target_dir).await?;
    for dir in &empty_dirs {
        state
            .transfers
            .ensure_dir(&target_account_id, dir, &*target)
            .await?;
    }
    let targets: Vec<&str> = files.iter().map(|(_, to)| to.as_str()).collect();
    let mut existing = existing_targets(&*target, &target_dir, &targets).await;
    let count = files.len();
    for (file, to) in files {
        let spec = Spec {
            direction: Direction::Copy,
            account_id: target_account_id.clone(),
            account_name: target_name.clone(),
            local: PathBuf::new(),
            known: existing.remove(&to),
            remote: to,
            size: file.size,
            from: Some(CopyFrom {
                account_id: account_id.clone(),
                account_name: source_name.clone(),
                file,
            }),
        };
        state
            .transfers
            .enqueue_copy(spec, target.clone(), source.clone());
    }
    Ok(count)
}

/// Inside one cloud: nothing goes into the folder it is in already (it
/// would replace itself), and no folder into itself.
fn check_copy_target(items: &[RemoteFileInfo], target_dir: &str) -> Result<()> {
    for item in items {
        let prefix = format!("{}/", item.path.trim_end_matches('/'));
        if item.is_dir && (target_dir == item.path || target_dir.starts_with(&prefix)) {
            bail!("A folder cannot be copied into itself");
        }
        if split_remote(&item.path).0 == target_dir {
            bail!("'{}' is in this folder already", item.name);
        }
    }
    Ok(())
}

/// Lists every file under the selected cloud items with the path it gets
/// under `target_dir`, plus the target folders that stay empty. Names stay
/// as they are; equal ones (Google Drive allows them) get numbers.
async fn expand_copy(
    provider: &dyn StorageProvider,
    items: Vec<RemoteFileInfo>,
    target_dir: &str,
) -> Result<(Vec<(RemoteFileInfo, String)>, Vec<String>)> {
    let mut files = Vec::new();
    let mut empty = Vec::new();
    let mut taken = HashSet::new();
    let mut level = Vec::new();
    for item in items {
        let to = unique_remote(join_remote(target_dir, &item.name), &mut taken);
        if item.is_dir {
            level.push((item.path, to));
        } else {
            files.push((item, to));
        }
    }
    while !level.is_empty() {
        // Owned paths: a stream over references trips the `Send` checks.
        let remotes: Vec<String> = level.iter().map(|(remote, _)| remote.clone()).collect();
        let listings: Vec<Result<Vec<RemoteFileInfo>>> = stream::iter(remotes)
            .map(|remote| async move { provider.list_dir(&remote).await })
            .buffered(sync::DEFAULT_PARALLEL)
            .collect()
            .await;
        let mut next = Vec::new();
        for ((_, to_dir), listing) in level.iter().zip(listings) {
            let listing = listing?;
            if listing.is_empty() {
                empty.push(to_dir.clone());
            }
            for entry in listing {
                let to = unique_remote(join_remote(to_dir, &entry.name), &mut taken);
                if entry.is_dir {
                    next.push((entry.path, to));
                } else {
                    files.push((entry, to));
                }
            }
        }
        level = next;
    }
    Ok((files, empty))
}

/// `path`, or `name (2).ext` and so on if an earlier call took it.
fn unique_remote(path: String, taken: &mut HashSet<String>) -> String {
    if taken.insert(path.clone()) {
        return path;
    }
    let (dir, name) = split_remote(&path);
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name.as_str(), ""),
    };
    (2..)
        .map(|n| join_remote(&dir, &format!("{stem} ({n}){ext}")))
        .find(|candidate| taken.insert(candidate.clone()))
        .expect("an unused name exists")
}

/// Which of `names` already exist in the local folder `dir`.
#[tauri::command]
pub fn local_conflicts(dir: String, names: Vec<String>) -> Vec<String> {
    names
        .into_iter()
        .filter(|name| Path::new(&dir).join(sanitize_file_name(name)).exists())
        .collect()
}

#[tauri::command]
pub async fn list_local(app: tauri::AppHandle, path: String) -> CmdResult<local::LocalListing> {
    let places = local_places(&app);
    // Slow or disconnected drives must not block the UI thread.
    let listing = tauri::async_runtime::spawn_blocking(move || local::list(&path, &places))
        .await
        .map_err(anyhow::Error::from)?;
    Ok(listing?)
}

#[cfg(desktop)]
fn local_places(app: &tauri::AppHandle) -> Vec<local::Place> {
    let paths = tauri::Manager::path(app);
    [
        ("Рабочий стол", paths.desktop_dir()),
        ("Загрузки", paths.download_dir()),
        ("Документы", paths.document_dir()),
        ("Домашняя папка", paths.home_dir()),
    ]
    .into_iter()
    .filter_map(|(name, path)| path.ok().map(|path| local::Place { name, path }))
    .collect()
}

/// The shared storage folders every Android device has.
#[cfg(mobile)]
fn local_places(_app: &tauri::AppHandle) -> Vec<local::Place> {
    let root = PathBuf::from(local::ANDROID_STORAGE);
    [
        ("Загрузки", "Download"),
        ("Документы", "Documents"),
        ("Камера", "DCIM"),
        ("Изображения", "Pictures"),
    ]
    .into_iter()
    .map(|(name, dir)| local::Place {
        name,
        path: root.join(dir),
    })
    .collect()
}

#[tauri::command]
pub fn list_transfers(state: State<'_, AppState>, filter: Option<TransferFilter>) -> TransferList {
    match filter.unwrap_or_default() {
        TransferFilter::All => state.transfers.list(),
        filter => state.transfers.list_filtered(filter),
    }
}

#[tauri::command]
pub fn cancel_transfer(state: State<'_, AppState>, id: u64) {
    state.transfers.cancel(id);
}

#[tauri::command]
pub fn cancel_all_transfers(state: State<'_, AppState>) {
    state.transfers.cancel_all();
}

#[tauri::command]
pub fn retry_transfer(state: State<'_, AppState>, id: u64) -> CmdResult<()> {
    let Some(spec) = state.transfers.retry_spec(id) else {
        return Ok(());
    };
    let provider = state.provider(&spec.account_id)?;
    let source = spec
        .from
        .as_ref()
        .map(|from| state.provider(&from.account_id))
        .transpose()?;
    state.transfers.retry(id, provider, source);
    Ok(())
}

#[derive(serde::Serialize)]
pub struct RetryFailures {
    retried: usize,
    errors: Vec<String>,
}

#[tauri::command]
pub fn retry_failed_transfers(state: State<'_, AppState>) -> RetryFailures {
    let mut result = RetryFailures {
        retried: 0,
        errors: Vec::new(),
    };
    for id in state.transfers.failed_ids() {
        let Some(spec) = state.transfers.retry_spec(id) else {
            continue;
        };
        let providers = (|| -> anyhow::Result<_> {
            let target = state.provider(&spec.account_id)?;
            let source = spec
                .from
                .as_ref()
                .map(|from| state.provider(&from.account_id))
                .transpose()?;
            Ok((target, source))
        })();
        match providers {
            Ok((target, source)) => {
                if state.transfers.retry(id, target, source) {
                    result.retried += 1;
                }
            }
            Err(err) => result
                .errors
                .push(crate::error::translate(&format!("{err:#}"))),
        }
    }
    result
}

#[tauri::command]
pub fn clear_transfers(state: State<'_, AppState>) {
    state.transfers.clear_finished();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_upload_walks_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("photos");
        std::fs::create_dir_all(root.join("2024")).unwrap();
        std::fs::create_dir_all(root.join("empty")).unwrap();
        std::fs::write(root.join("2024").join("a.jpg"), b"x").unwrap();
        std::fs::write(tmp.path().join("note.txt"), b"y").unwrap();

        let (mut files, empty) =
            expand_upload(&[root.clone(), tmp.path().join("note.txt")], "/Backup").unwrap();
        files.sort_by(|a, b| a.1.cmp(&b.1));
        let remotes: Vec<&str> = files.iter().map(|(_, r)| r.as_str()).collect();
        assert_eq!(remotes, ["/Backup/note.txt", "/Backup/photos/2024/a.jpg"]);
        assert_eq!(empty, ["/Backup/photos/empty"]);
    }

    #[test]
    fn copies_stay_out_of_themselves() {
        let item = |path: &str, is_dir: bool| RemoteFileInfo {
            name: split_remote(path).1,
            path: path.into(),
            is_dir,
            size: 0,
            md5: None,
            modified: None,
        };
        let photos = [item("/Photos", true)];
        assert!(check_copy_target(&photos, "/Photos").is_err());
        assert!(check_copy_target(&photos, "/Photos/2024").is_err());
        assert!(
            check_copy_target(&photos, "/").is_err(),
            "already in the root"
        );
        assert!(check_copy_target(&photos, "/Photos 2").is_ok());
        assert!(check_copy_target(&[item("/a/x.txt", false)], "/a").is_err());
        assert!(check_copy_target(&[item("/a/x.txt", false)], "/b").is_ok());

        let mut taken = HashSet::new();
        assert_eq!(unique_remote("/d/a.txt".into(), &mut taken), "/d/a.txt");
        assert_eq!(unique_remote("/d/a.txt".into(), &mut taken), "/d/a (2).txt");
    }
}
