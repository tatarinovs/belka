//! Local files and the transfer queue: browsing, folder operations,
//! queuing uploads and downloads, and managing the queue.

use crate::error::CmdResult;
use crate::transfers::{Direction, Spec, TransferList};
use crate::{local, native, AppState};
use anyhow::{bail, Context, Result};
use belka_core::sync::{self, Filter};
use belka_core::utils::path::{join_remote, normalize_remote_dir, sanitize_file_name, unique_path};
use belka_core::StorageProvider;
use serde::Deserialize;
use std::collections::{HashSet, VecDeque};
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

#[derive(Deserialize)]
pub struct RemoteItem {
    path: String,
    is_dir: bool,
    size: u64,
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
    for (remote, local, size) in &files {
        let spec = Spec {
            direction: Direction::Download,
            account_id: account_id.clone(),
            account_name: account_name.clone(),
            local: local.clone(),
            remote: remote.clone(),
            size: *size,
        };
        state.transfers.enqueue(spec, provider.clone());
    }
    Ok(files.len())
}

/// Lists every file under the selected items, mapping cloud names to safe
/// local names. Folders are created locally right away, so empty ones survive.
async fn expand_download(
    provider: &dyn StorageProvider,
    items: Vec<RemoteItem>,
    local_dir: &Path,
) -> Result<Vec<(String, PathBuf, u64)>> {
    let mut files = Vec::new();
    let mut taken = HashSet::new();
    let mut dirs = VecDeque::new();
    for item in items {
        let name = item.path.rsplit('/').next().unwrap_or_default();
        let local = unique_path(local_dir.join(sanitize_file_name(name)), &mut taken);
        if item.is_dir {
            dirs.push_back((normalize_remote_dir(&item.path), local));
        } else {
            files.push((normalize_remote_dir(&item.path), local, item.size));
        }
    }
    while let Some((remote, local)) = dirs.pop_front() {
        std::fs::create_dir_all(&local)
            .with_context(|| format!("Failed to create '{}'", local.display()))?;
        for entry in provider.list_dir(&remote).await? {
            let path = unique_path(local.join(sanitize_file_name(&entry.name)), &mut taken);
            if entry.is_dir {
                dirs.push_back((entry.path, path));
            } else {
                files.push((entry.path, path, entry.size));
            }
        }
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
    let (files, empty_dirs) =
        tauri::async_runtime::spawn_blocking(move || expand_upload(&paths, &remote_dir))
            .await
            .context("Failed to scan local files")??;

    for dir in &empty_dirs {
        state
            .transfers
            .ensure_dir(&account_id, dir, &*provider)
            .await?;
    }
    for (local, remote) in &files {
        let spec = Spec {
            direction: Direction::Upload,
            account_id: account_id.clone(),
            account_name: account_name.clone(),
            local: local.clone(),
            remote: remote.clone(),
            size: 0,
        };
        state.transfers.enqueue(spec, provider.clone());
    }
    Ok(files.len())
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
        if tree.single_file {
            files.extend(
                tree.files
                    .into_iter()
                    .map(|f| (f.path, join_remote(remote_dir, &name))),
            );
            continue;
        }
        let base = join_remote(remote_dir, &name);
        let is_empty = |dir: &str| {
            let prefix = format!("{dir}/");
            !tree.files.iter().any(|f| f.relative.starts_with(&prefix))
                && !tree.dirs.iter().any(|d| d.starts_with(&prefix))
        };
        empty_dirs.extend(
            tree.dirs
                .iter()
                .filter(|d| is_empty(d))
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
pub fn list_transfers(state: State<'_, AppState>) -> TransferList {
    state.transfers.list()
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
    state.transfers.retry(id, provider);
    Ok(())
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
}
