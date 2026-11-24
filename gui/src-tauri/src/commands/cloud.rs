//! Operations on cloud files: listing, folders, renaming, deleting, links
//! and preview.

use crate::{native, AppState};
use anyhow::Context;
use belka_app::error::AppResult;
use belka_app::preview::{self, Preview, PreviewProgress};
use belka_app::{CloudItem, ShareParams};
use belka_core::storage::Quota;
use belka_core::RemoteFileInfo;
use tauri::State;
use tauri_plugin_clipboard_manager::ClipboardExt;

#[tauri::command]
pub async fn list_dir(
    state: State<'_, AppState>,
    account_id: String,
    path: String,
) -> AppResult<Vec<RemoteFileInfo>> {
    state.list_dir(&account_id, &path).await
}

#[tauri::command]
pub async fn cloud_quota(
    state: State<'_, AppState>,
    account_id: String,
    path: String,
) -> AppResult<Option<Quota>> {
    state.cloud_quota(&account_id, &path).await
}

#[tauri::command]
pub async fn cloud_mkdir(
    state: State<'_, AppState>,
    account_id: String,
    parent: String,
    name: String,
) -> AppResult<String> {
    state.cloud_mkdir(&account_id, &parent, &name).await
}

#[tauri::command]
pub async fn cloud_rename(
    state: State<'_, AppState>,
    account_id: String,
    path: String,
    new_name: String,
    is_dir: bool,
) -> AppResult<String> {
    state.cloud_rename(&account_id, &path, &new_name, is_dir).await
}

#[tauri::command]
pub async fn cloud_delete(
    state: State<'_, AppState>,
    account_id: String,
    items: Vec<CloudItem>,
) -> AppResult<()> {
    state.cloud_delete(&account_id, &items).await
}

#[tauri::command]
pub async fn cloud_share(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    items: Vec<CloudItem>,
    options: Option<ShareParams>,
) -> AppResult<Vec<String>> {
    let links = state.cloud_share(&account_id, &items, options).await?;
    app.clipboard()
        .write_text(links.join("\n"))
        .context("Failed to copy the link to the clipboard")?;
    Ok(links)
}

#[tauri::command]
pub async fn cloud_unshare(
    state: State<'_, AppState>,
    account_id: String,
    items: Vec<CloudItem>,
) -> AppResult<()> {
    state.cloud_unshare(&account_id, &items).await
}

#[tauri::command]
pub async fn preview_file(
    state: State<'_, AppState>,
    account_id: String,
    path: String,
    size: u64,
    modified: Option<String>,
    text: bool,
) -> AppResult<Preview> {
    state
        .preview_file(&account_id, &path, size, modified.as_deref(), text)
        .await
}

#[tauri::command]
pub fn preview_progress(state: State<'_, AppState>) -> PreviewProgress {
    state.preview_progress()
}

#[tauri::command]
pub fn preview_cancel(state: State<'_, AppState>) {
    state.preview_cancel();
}

#[tauri::command]
pub async fn stream_open(
    state: State<'_, AppState>,
    account_id: String,
    path: String,
    size: u64,
) -> AppResult<Option<String>> {
    state.stream_open(&account_id, &path, size).await
}

#[tauri::command]
pub fn preview_open(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> AppResult<()> {
    let path = state.preview_cached(&path)?;
    if cfg!(windows) && preview::runs_when_opened(&path) {
        return Err(anyhow::anyhow!(
            "Files of this type run as programs when opened; download the file to look at it"
        )
        .into());
    }
    Ok(native::open_file(&app, &path.to_string_lossy())?)
}

#[tauri::command]
pub async fn stream_play(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    path: String,
    size: u64,
) -> AppResult<()> {
    let (url, mime, name) = state.stream_open_player(&account_id, &path, size).await?;
    tauri::async_runtime::spawn_blocking(move || native::play_url(&app, &url, mime, &name))
        .await
        .map_err(|err| anyhow::anyhow!("{err}"))??;
    Ok(())
}

/// Android: the pages of a cached PDF preview with their sizes.
#[tauri::command]
pub async fn pdf_info(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> AppResult<native::PdfInfo> {
    let path = state.preview_cached(&path)?;
    let info = tauri::async_runtime::spawn_blocking(move || native::pdf_info(&app, &path))
        .await
        .map_err(|err| anyhow::anyhow!("{err}"))??;
    Ok(info)
}

/// Android: draws page `page` of a cached PDF preview `width` pixels wide
/// and returns the picture, which is kept next to the preview.
#[tauri::command]
pub async fn pdf_page(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
    page: u32,
    width: u32,
) -> AppResult<String> {
    let path = state.preview_cached(&path)?;
    let width = width.clamp(100, 2400);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".p{page}w{width}.png"));
    let out = path.with_file_name(name);
    let picture = out.clone();
    tauri::async_runtime::spawn_blocking(move || native::pdf_page(&app, &path, page, width, &out))
        .await
        .map_err(|err| anyhow::anyhow!("{err}"))??;
    Ok(picture.to_string_lossy().into_owned())
}
