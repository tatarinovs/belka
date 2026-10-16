//! Accounts: adding (with a password, a token or a browser sign-in),
//! changing, ordering, removing, export and import.

use super::file_path;
use crate::error::CmdResult;
use crate::AppState;
use anyhow::Context;
use belka_core::oauth::{self, OAuthApp, Service};
use belka_core::storage::crypt::CryptProvider;
use belka_core::{ProviderConfig, StorageProvider};
use belka_store::accounts::{
    Account, AccountSource, Backup, CryptFolder, CryptLocation, Imported, CRYPT_KIND,
};
use serde::Serialize;
use std::sync::Arc;
use tauri::State;
use tauri_plugin_fs::{FsExt, OpenOptions};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::oneshot;

#[tauri::command]
pub fn list_accounts(state: State<'_, AppState>) -> CmdResult<Vec<Account>> {
    let store = state.accounts.lock().unwrap();
    let mut accounts = store.list()?;
    // Encrypted folders added before the location was kept in the list.
    for account in &mut accounts {
        if account.kind == CRYPT_KIND && account.crypt.is_none() {
            account.crypt = store.crypt_folder(&account.id).ok().map(|f| CryptLocation {
                base: f.base,
                root: f.root,
            });
        }
    }
    Ok(accounts)
}

/// Builds the provider and checks that the credentials work.
async fn connect(config: &ProviderConfig) -> anyhow::Result<Arc<dyn StorageProvider>> {
    let provider: Arc<dyn StorageProvider> = config.build()?.into();
    provider
        .list_dir("/")
        .await
        .with_context(|| format!("Cannot connect to {}", provider.name()))?;
    Ok(provider)
}

/// Checks that the credentials work before saving the account.
#[tauri::command]
pub async fn add_account(
    state: State<'_, AppState>,
    name: String,
    config: ProviderConfig,
) -> CmdResult<Account> {
    let provider = connect(&config).await?;
    let account = state.accounts.lock().unwrap().add(&name, &config)?;
    state
        .providers
        .lock()
        .unwrap()
        .insert(account.id.clone(), provider);
    Ok(account)
}

#[derive(Serialize)]
pub struct OAuthApps {
    /// This build has an OAuth app of its own for the service.
    builtin: bool,
    /// The user's own OAuth app, saved after the last sign-in with it.
    custom: Option<OAuthApp>,
}

#[tauri::command]
pub fn oauth_apps(state: State<'_, AppState>, service: Service) -> OAuthApps {
    OAuthApps {
        builtin: service.builtin_app().is_some(),
        custom: state.accounts.lock().unwrap().load_oauth_app(service),
    }
}

/// Signs in through the browser and saves the account, or with `account_id`
/// replaces the credentials of that account. Without `custom_app`
/// the built-in OAuth app is used. An empty `name` keeps the name of the
/// replaced account, or becomes the e-mail or login of a new one.
#[tauri::command]
pub async fn oauth_sign_in(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    service: Service,
    custom_app: Option<OAuthApp>,
    name: String,
    account_id: Option<String>,
) -> CmdResult<Account> {
    let (cancel, cancelled) = oneshot::channel();
    // A sign-in still waiting in another browser tab is abandoned.
    state.sign_in_cancel.lock().unwrap().replace(cancel);
    let oauth_app = match &custom_app {
        Some(custom) => custom.clone(),
        None => service
            .builtin_app()
            .context("This build has no built-in OAuth app; enter your own")?,
    };
    let open = |url: &str| {
        app.opener()
            .open_url(url, None::<&str>)
            .map_err(|err| anyhow::anyhow!("{err}"))
    };
    let signed_in = tokio::select! {
        result = oauth::sign_in(service, &oauth_app, open) => result?,
        _ = cancelled => return Err(anyhow::anyhow!("Sign-in cancelled").into()),
    };

    let provider = connect(&signed_in.config).await?;
    let current = account_id
        .as_deref()
        .map(|id| state.account(id))
        .transpose()?;
    let name = Some(name.trim().to_string())
        .filter(|n| !n.is_empty())
        .or_else(|| current.as_ref().map(|a| a.name.clone()))
        .or(signed_in.label)
        .unwrap_or_else(|| service.title().to_string());
    let account = {
        let store = state.accounts.lock().unwrap();
        match &current {
            Some(current) => store.update(&current.id, &name, Some(&signed_in.config))?,
            None => store.add(&name, &signed_in.config)?,
        }
    };
    state.replace_provider(&account.id, provider);
    // Only the user's own app is remembered: the built-in one always
    // comes from the binary, so an update can replace it.
    if let Some(custom) = &custom_app {
        if let Err(err) = state
            .accounts
            .lock()
            .unwrap()
            .save_oauth_app(service, custom)
        {
            tracing::warn!("{err:#}");
        }
    }
    Ok(account)
}

#[tauri::command]
pub fn oauth_cancel(state: State<'_, AppState>) {
    if let Some(cancel) = state.sign_in_cancel.lock().unwrap().take() {
        let _ = cancel.send(());
    }
}

/// Adds an encrypted folder in account `folder.base`. An empty folder or
/// one encrypted with the same password is accepted.
#[tauri::command]
pub async fn add_crypt_folder(
    state: State<'_, AppState>,
    name: String,
    folder: CryptFolder,
) -> CmdResult<Account> {
    if state.account(&folder.base)?.kind == CRYPT_KIND {
        return Err(
            anyhow::anyhow!("An encrypted folder cannot be placed inside another one").into(),
        );
    }
    let base = state.provider(&folder.base)?;
    let (root, config) = (folder.root.clone(), folder.config.clone());
    // Key derivation is deliberately slow.
    let provider =
        tauri::async_runtime::spawn_blocking(move || CryptProvider::new(base, &root, &config))
            .await
            .map_err(anyhow::Error::from)??;
    provider.verify_password().await?;
    let account = state.accounts.lock().unwrap().add_crypt(&name, &folder)?;
    state
        .providers
        .lock()
        .unwrap()
        .insert(account.id.clone(), Arc::new(provider));
    Ok(account)
}

/// The stored credentials of an account for its settings form; `None` for
/// an encrypted folder, whose password is not shown.
#[tauri::command]
pub fn account_config(state: State<'_, AppState>, id: String) -> CmdResult<Option<ProviderConfig>> {
    if state.account(&id)?.kind == CRYPT_KIND {
        return Ok(None);
    }
    Ok(Some(state.accounts.lock().unwrap().credentials(&id)?))
}

/// Renames an account and, with `config`, replaces its credentials once
/// they are checked.
#[tauri::command]
pub async fn update_account(
    state: State<'_, AppState>,
    id: String,
    name: String,
    config: Option<ProviderConfig>,
) -> CmdResult<Account> {
    let provider = match &config {
        Some(config) => Some(connect(config).await?),
        None => None,
    };
    let account = state
        .accounts
        .lock()
        .unwrap()
        .update(&id, &name, config.as_ref())?;
    if let Some(provider) = provider {
        state.replace_provider(&id, provider);
    }
    Ok(account)
}

/// Saves the order in which accounts are listed.
#[tauri::command]
pub fn reorder_accounts(state: State<'_, AppState>, ids: Vec<String>) -> CmdResult<()> {
    Ok(state.accounts.lock().unwrap().reorder(&ids)?)
}

/// Refuses to remove an account that encrypted folders are stored in or
/// that backups go to.
#[tauri::command]
pub fn remove_account(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let quoted = |names: Vec<String>| {
        names
            .iter()
            .map(|n| format!("«{n}»"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let store = state.accounts.lock().unwrap();
    let folders = store.crypt_folders_in(&id);
    if !folders.is_empty() {
        let names = quoted(folders);
        return Err(anyhow::anyhow!(
            "Encrypted folders {names} are stored in this account; remove them first"
        )
        .into());
    }
    let tasks = state.tasks.using_account(&id);
    if !tasks.is_empty() {
        let names = quoted(tasks);
        return Err(
            anyhow::anyhow!("Backups {names} go to this account; remove them first").into(),
        );
    }
    state.providers.lock().unwrap().remove(&id);
    Ok(store.remove(&id)?)
}

/// Saves all accounts with their credentials into file `path`, encrypted
/// with `password`.
#[tauri::command]
pub async fn export_accounts(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
    password: String,
) -> CmdResult<usize> {
    let backup = state.accounts.lock().unwrap().export()?;
    let count = backup.account_count();
    // Key derivation is deliberately slow.
    let file = tauri::async_runtime::spawn_blocking(move || backup.seal(&password))
        .await
        .map_err(anyhow::Error::from)??;
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    app.fs()
        .open(file_path(&path), options)
        .and_then(|mut out| std::io::Write::write_all(&mut out, &file))
        .with_context(|| format!("Failed to write '{path}'"))?;
    Ok(count)
}

/// Adds the accounts from a file made by [`export_accounts`].
#[tauri::command]
pub async fn import_accounts(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
    password: String,
) -> CmdResult<Imported> {
    let file = app
        .fs()
        .read(file_path(&path))
        .with_context(|| format!("Failed to read '{path}'"))?;
    let backup = tauri::async_runtime::spawn_blocking(move || Backup::open(&file, &password))
        .await
        .map_err(anyhow::Error::from)??;
    Ok(state.accounts.lock().unwrap().import(backup)?)
}
