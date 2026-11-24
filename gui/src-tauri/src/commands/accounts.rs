//! Accounts: adding (with a password, a token or a browser sign-in),
//! changing, ordering, removing, export and import.

use super::file_path;
use crate::AppState;
use anyhow::Context;
use belka_app::error::AppResult;
use belka_app::OAuthApps;
use belka_core::oauth::{OAuthApp, Service};
use belka_core::ProviderConfig;
use belka_store::accounts::{Account, CryptFolder, Imported};
use tauri::State;
use tauri_plugin_fs::{FsExt, OpenOptions};
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
pub fn list_accounts(state: State<'_, AppState>) -> AppResult<Vec<Account>> {
    state.list_accounts()
}

#[tauri::command]
pub async fn add_account(
    state: State<'_, AppState>,
    name: String,
    config: ProviderConfig,
) -> AppResult<Account> {
    state.add_account(&name, config).await
}

#[tauri::command]
pub fn oauth_apps(state: State<'_, AppState>, service: Service) -> OAuthApps {
    state.oauth_apps(service)
}

#[tauri::command]
pub async fn oauth_sign_in(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    service: Service,
    custom_app: Option<OAuthApp>,
    name: String,
    account_id: Option<String>,
) -> AppResult<Account> {
    let open = |url: &str| {
        app.opener()
            .open_url(url, None::<&str>)
            .map_err(|err| anyhow::anyhow!("{err}"))
    };
    state
        .oauth_sign_in(service, custom_app, &name, account_id.as_deref(), open)
        .await
}

#[tauri::command]
pub fn oauth_cancel(state: State<'_, AppState>) {
    state.oauth_cancel();
}

#[tauri::command]
pub async fn add_crypt_folder(
    state: State<'_, AppState>,
    name: String,
    folder: CryptFolder,
) -> AppResult<Account> {
    state.add_crypt_folder(&name, folder).await
}

#[tauri::command]
pub fn account_config(state: State<'_, AppState>, id: String) -> AppResult<Option<ProviderConfig>> {
    state.account_config(&id)
}

#[tauri::command]
pub async fn update_account(
    state: State<'_, AppState>,
    id: String,
    name: String,
    config: Option<ProviderConfig>,
) -> AppResult<Account> {
    state.update_account(&id, &name, config).await
}

#[tauri::command]
pub fn reorder_accounts(state: State<'_, AppState>, ids: Vec<String>) -> AppResult<()> {
    state.reorder_accounts(&ids)
}

#[tauri::command]
pub fn remove_account(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.remove_account(&id)
}

#[tauri::command]
pub async fn export_accounts(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
    password: String,
) -> AppResult<usize> {
    let (count, file) = state.export_accounts_bytes(password).await?;
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    app.fs()
        .open(file_path(&path), options)
        .and_then(|mut out| std::io::Write::write_all(&mut out, &file))
        .with_context(|| format!("Failed to write '{path}'"))?;
    Ok(count)
}

#[tauri::command]
pub async fn import_accounts(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
    password: String,
) -> AppResult<Imported> {
    let file = app
        .fs()
        .read(file_path(&path))
        .with_context(|| format!("Failed to read '{path}'"))?;
    state.import_accounts_bytes(file, password).await
}
