//! Local files and the transfer queue: browsing, folder operations,
//! queuing uploads and downloads, and managing the queue.

use crate::{native, AppState};
use belka_app::error::AppResult;
use belka_app::local;
use belka_app::state::{RemoteItem, RetryFailures};
use belka_app::transfers::{TransferFilter, TransferList};
use tauri::State;

#[tauri::command]
pub fn local_open(app: tauri::AppHandle, path: String) -> AppResult<()> {
    Ok(native::open_file(&app, &path)?)
}

#[tauri::command]
pub fn local_mkdir(state: State<'_, AppState>, parent: String, name: String) -> AppResult<String> {
    state.local_mkdir(&parent, &name)
}

#[tauri::command]
pub fn local_rename(state: State<'_, AppState>, path: String, new_name: String) -> AppResult<String> {
    state.local_rename(&path, &new_name)
}

#[tauri::command]
pub async fn local_delete(state: State<'_, AppState>, paths: Vec<String>) -> AppResult<()> {
    state.local_delete(paths).await
}

#[tauri::command]
pub async fn download(
    state: State<'_, AppState>,
    account_id: String,
    items: Vec<RemoteItem>,
    local_dir: String,
) -> AppResult<usize> {
    state.download(&account_id, items, &local_dir).await
}

#[tauri::command]
pub async fn upload(
    state: State<'_, AppState>,
    account_id: String,
    local_paths: Vec<String>,
    remote_dir: String,
) -> AppResult<usize> {
    state.upload(&account_id, local_paths, &remote_dir).await
}

#[tauri::command]
pub async fn cloud_copy(
    state: State<'_, AppState>,
    account_id: String,
    items: Vec<RemoteItem>,
    target_account_id: String,
    target_dir: String,
) -> AppResult<usize> {
    state
        .cloud_copy(&account_id, items, &target_account_id, &target_dir)
        .await
}

#[tauri::command]
pub fn local_conflicts(
    state: State<'_, AppState>,
    dir: String,
    names: Vec<String>,
) -> Vec<String> {
    state.local_conflicts(&dir, names)
}

#[tauri::command]
pub async fn list_local(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> AppResult<local::LocalListing> {
    let places = local_places(&app);
    state.list_local(path, places).await
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

#[cfg(mobile)]
fn local_places(_app: &tauri::AppHandle) -> Vec<local::Place> {
    local::android_places()
}

#[tauri::command]
pub fn list_transfers(state: State<'_, AppState>, filter: Option<TransferFilter>) -> TransferList {
    state.list_transfers(filter)
}

#[tauri::command]
pub fn cancel_transfer(state: State<'_, AppState>, id: u64) {
    state.cancel_transfer(id);
}

#[tauri::command]
pub fn cancel_all_transfers(state: State<'_, AppState>) {
    state.cancel_all_transfers();
}

#[tauri::command]
pub fn retry_transfer(state: State<'_, AppState>, id: u64) -> AppResult<()> {
    state.retry_transfer(id)
}

#[tauri::command]
pub fn retry_failed_transfers(state: State<'_, AppState>) -> RetryFailures {
    state.retry_failed_transfers()
}

#[tauri::command]
pub fn clear_transfers(state: State<'_, AppState>) {
    state.clear_transfers();
}
