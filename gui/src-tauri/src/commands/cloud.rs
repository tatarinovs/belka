//! Operations on cloud files: listing, folders, renaming, deleting, links
//! and preview.

use crate::error::CmdResult;
use crate::{native, preview, AppState};
use anyhow::{Context, Result};
use belka_core::storage::{check_name, rename_target, ShareOptions};
use belka_core::utils::path::{join_remote, normalize_remote_dir};
use belka_core::RemoteFileInfo;
use chrono::NaiveDate;
use serde::Deserialize;
use tauri::State;
use tauri_plugin_clipboard_manager::ClipboardExt;

#[tauri::command]
pub async fn list_dir(
    state: State<'_, AppState>,
    account_id: String,
    path: String,
) -> CmdResult<Vec<RemoteFileInfo>> {
    let provider = state.provider(&account_id)?;
    Ok(provider.list_dir(&normalize_remote_dir(&path)).await?)
}

/// Creates folder `name` in `parent` and returns its path.
#[tauri::command]
pub async fn cloud_mkdir(
    state: State<'_, AppState>,
    account_id: String,
    parent: String,
    name: String,
) -> CmdResult<String> {
    let provider = state.provider(&account_id)?;
    let path = join_remote(&normalize_remote_dir(&parent), check_name(&name)?);
    // Creating a folder that exists succeeds silently in most clouds.
    if provider.get_file_info(&path).await?.is_some() {
        return Err(anyhow::anyhow!("'{}' already exists", name.trim()).into());
    }
    provider.ensure_dir(&path).await?;
    Ok(path)
}

/// Renames a cloud item and returns its new path.
#[tauri::command]
pub async fn cloud_rename(
    state: State<'_, AppState>,
    account_id: String,
    path: String,
    new_name: String,
    is_dir: bool,
) -> CmdResult<String> {
    let provider = state.provider(&account_id)?;
    let target = rename_target(&path, &new_name)?;
    provider.rename(&path, &new_name, is_dir).await?;
    if is_dir {
        state.transfers.forget_dir(&account_id, &path).await;
    }
    Ok(target)
}

#[derive(Deserialize)]
pub struct CloudItem {
    path: String,
    is_dir: bool,
}

/// Deletes cloud items permanently, one by one; stops at the first failure.
#[tauri::command]
pub async fn cloud_delete(
    state: State<'_, AppState>,
    account_id: String,
    items: Vec<CloudItem>,
) -> CmdResult<()> {
    let provider = state.provider(&account_id)?;
    for item in &items {
        provider.delete(&item.path, item.is_dir).await?;
        if item.is_dir {
            state.transfers.forget_dir(&account_id, &item.path).await;
        }
    }
    Ok(())
}

/// Link settings as the page sends them; empty values mean "not set".
#[derive(Deserialize)]
pub struct ShareParams {
    password: Option<String>,
    /// `YYYY-MM-DD`
    expires: Option<String>,
}

impl ShareParams {
    fn parse(self) -> Result<ShareOptions> {
        let expires = match self.expires.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(date) => Some(
                NaiveDate::parse_from_str(date, "%Y-%m-%d")
                    .with_context(|| format!("Invalid date '{date}'"))?,
            ),
        };
        Ok(ShareOptions {
            password: self.password.filter(|p| !p.is_empty()),
            expires,
        })
    }
}

/// Opens access by link to cloud items and copies the links to the
/// clipboard, one per line. With `options` every link gets exactly these
/// settings, replacing an existing one. Returns the links.
#[tauri::command]
pub async fn cloud_share(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    items: Vec<CloudItem>,
    options: Option<ShareParams>,
) -> CmdResult<Vec<String>> {
    let provider = state.provider(&account_id)?;
    let options = options.map(ShareParams::parse).transpose()?;
    let mut links = Vec::with_capacity(items.len());
    for item in &items {
        links.push(match &options {
            Some(options) => {
                provider
                    .share_with(&item.path, item.is_dir, options)
                    .await?
            }
            None => provider.share(&item.path, item.is_dir).await?,
        });
    }
    app.clipboard()
        .write_text(links.join("\n"))
        .context("Failed to copy the link to the clipboard")?;
    Ok(links)
}

/// Closes access by link to cloud items.
#[tauri::command]
pub async fn cloud_unshare(
    state: State<'_, AppState>,
    account_id: String,
    items: Vec<CloudItem>,
) -> CmdResult<()> {
    let provider = state.provider(&account_id)?;
    for item in &items {
        provider.unshare(&item.path, item.is_dir).await?;
    }
    Ok(())
}

/// Downloads a cloud file into the preview cache (once) and returns where it
/// is; with `text`, also its content.
#[tauri::command]
pub async fn preview_file(
    state: State<'_, AppState>,
    account_id: String,
    path: String,
    size: u64,
    modified: Option<String>,
    text: bool,
) -> CmdResult<preview::Preview> {
    let provider = state.provider(&account_id)?;
    let item = preview::Item {
        account_id: &account_id,
        path: &normalize_remote_dir(&path),
        size,
        modified: modified.as_deref(),
    };
    Ok(state.previews.load(&*provider, item, text).await?)
}

#[tauri::command]
pub fn preview_progress(state: State<'_, AppState>) -> preview::PreviewProgress {
    state.previews.progress()
}

#[tauri::command]
pub fn preview_cancel(state: State<'_, AppState>) {
    state.previews.cancel();
    state.streams.close();
}

/// Starts streaming a cloud media file and returns its URL, or `None` if
/// it cannot stream and has to be downloaded.
#[tauri::command]
pub async fn stream_open(
    state: State<'_, AppState>,
    account_id: String,
    path: String,
    size: u64,
) -> CmdResult<Option<String>> {
    let provider = state.provider(&account_id)?;
    let path = normalize_remote_dir(&path);
    match state.streams.open(provider, &path, size).await {
        Ok(url) => Ok(Some(url)),
        Err(err) => {
            tracing::info!("Cannot stream '{path}', downloading instead: {err:#}");
            Ok(None)
        }
    }
}

/// Opens a cached preview in the program the system associates with it.
#[tauri::command]
pub fn preview_open(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> CmdResult<()> {
    let path = state.previews.cached(&path)?;
    if cfg!(windows) && preview::runs_when_opened(&path) {
        return Err(anyhow::anyhow!(
            "Files of this type run as programs when opened; download the file to look at it"
        )
        .into());
    }
    Ok(native::open_file(&app, &path.to_string_lossy())?)
}

/// Hands a cloud video to an external player as a stream: the program set
/// for its type on Windows, the app the user picks on Android.
#[tauri::command]
pub async fn stream_play(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    path: String,
    size: u64,
) -> CmdResult<()> {
    let provider = state.provider(&account_id)?;
    let path = normalize_remote_dir(&path);
    let url = match state.streams.open_player(provider, &path, size).await {
        Ok(url) => url,
        Err(err) => {
            tracing::info!("Cannot stream '{path}' to a player: {err:#}");
            return Err(anyhow::anyhow!("The cloud cannot stream this file").into());
        }
    };
    let name = path.rsplit('/').next().unwrap_or_default().to_string();
    let mime = crate::stream::mime_type(&path);
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
) -> CmdResult<native::PdfInfo> {
    let path = state.previews.cached(&path)?;
    // The plugin answers once the document is read; keep the runtime free.
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
) -> CmdResult<String> {
    let path = state.previews.cached(&path)?;
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
