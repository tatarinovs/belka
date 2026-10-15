mod accounts;
mod local;
mod preview;
mod transfers;

use accounts::{Account, AccountStore, CryptFolder, CryptLocation, CRYPT_KIND};
use anyhow::{bail, Context, Result};
use belka_core::oauth::{self, OAuthApp, Service};
use belka_core::storage::crypt::CryptProvider;
use belka_core::storage::{check_name, rename_target, ShareOptions};
use belka_core::utils::path::{join_remote, normalize_remote_dir, sanitize_file_name};
use belka_core::{ProviderConfig, RemoteFileInfo, StorageProvider};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_window_state::StateFlags;
use tokio::sync::oneshot;
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::prelude::*;
use transfers::{Direction, Spec, TransferList, Transfers};

struct AppState {
    accounts: Mutex<AccountStore>,
    /// Providers are built lazily and reused, so tokens and connections are kept.
    providers: Mutex<HashMap<String, Arc<dyn StorageProvider>>>,
    transfers: Arc<Transfers>,
    /// Aborts the browser sign-in in progress.
    sign_in_cancel: Mutex<Option<oneshot::Sender<()>>>,
    previews: preview::Previews,
}

impl AppState {
    fn provider(&self, account_id: &str) -> Result<Arc<dyn StorageProvider>> {
        if let Some(p) = self.providers.lock().unwrap().get(account_id) {
            return Ok(p.clone());
        }
        let provider: Arc<dyn StorageProvider> = if self.account(account_id)?.kind == CRYPT_KIND {
            let folder = self.accounts.lock().unwrap().crypt_folder(account_id)?;
            let base = self.provider(&folder.base)?;
            Arc::new(CryptProvider::new(base, &folder.root, &folder.config)?)
        } else {
            let config = self.accounts.lock().unwrap().credentials(account_id)?;
            config.build()?.into()
        };
        self.providers
            .lock()
            .unwrap()
            .insert(account_id.to_string(), provider.clone());
        Ok(provider)
    }

    fn account(&self, account_id: &str) -> Result<Account> {
        self.accounts
            .lock()
            .unwrap()
            .list()?
            .into_iter()
            .find(|a| a.id == account_id)
            .context("Account not found")
    }

    fn account_name(&self, account_id: &str) -> Result<String> {
        self.account(account_id).map(|a| a.name)
    }

    /// Uses `provider` for the account from now on. Encrypted folders in it
    /// hold the old one, so they are rebuilt on next use.
    fn replace_provider(&self, account_id: &str, provider: Arc<dyn StorageProvider>) {
        let dependent: Vec<String> = {
            let store = self.accounts.lock().unwrap();
            store
                .list()
                .unwrap_or_default()
                .into_iter()
                .filter(|a| a.kind == CRYPT_KIND)
                .filter(|a| {
                    store
                        .crypt_folder(&a.id)
                        .is_ok_and(|f| f.base == account_id)
                })
                .map(|a| a.id)
                .collect()
        };
        let mut providers = self.providers.lock().unwrap();
        for id in dependent {
            providers.remove(&id);
        }
        providers.insert(account_id.to_string(), provider);
    }
}

/// Commands return errors as strings with the full context chain.
type CmdResult<T> = std::result::Result<T, String>;

/// Also logged: the page shows the message, but the log keeps it.
fn to_msg(err: anyhow::Error) -> String {
    let msg = format!("{err:#}");
    tracing::warn!("{msg}");
    msg
}

#[tauri::command]
fn list_accounts(state: State<'_, AppState>) -> CmdResult<Vec<Account>> {
    let store = state.accounts.lock().unwrap();
    let mut accounts = store.list().map_err(to_msg)?;
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

/// Checks that the credentials work before saving the account.
#[tauri::command]
async fn add_account(
    state: State<'_, AppState>,
    name: String,
    config: ProviderConfig,
) -> CmdResult<Account> {
    async {
        let provider: Arc<dyn StorageProvider> = config.build()?.into();
        provider
            .list_dir("/")
            .await
            .with_context(|| format!("Cannot connect to {}", provider.name()))?;
        let account = state.accounts.lock().unwrap().add(&name, &config)?;
        state
            .providers
            .lock()
            .unwrap()
            .insert(account.id.clone(), provider);
        Ok(account)
    }
    .await
    .map_err(to_msg)
}

#[derive(Serialize)]
struct OAuthApps {
    /// This build has an OAuth app of its own for the service.
    builtin: bool,
    /// The user's own OAuth app, saved after the last sign-in with it.
    custom: Option<OAuthApp>,
}

#[tauri::command]
fn oauth_apps(service: Service) -> OAuthApps {
    OAuthApps {
        builtin: service.builtin_app().is_some(),
        custom: accounts::load_oauth_app(service),
    }
}

/// Signs in through the browser and saves the account, or with `account_id`
/// replaces the credentials of that account. Without `custom_app`
/// the built-in OAuth app is used. An empty `name` keeps the name of the
/// replaced account, or becomes the e-mail or login of a new one.
#[tauri::command]
async fn oauth_sign_in(
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
    async {
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
            _ = cancelled => bail!("Sign-in cancelled"),
        };

        let provider: Arc<dyn StorageProvider> = signed_in.config.build()?.into();
        provider
            .list_dir("/")
            .await
            .with_context(|| format!("Cannot connect to {}", provider.name()))?;
        let current = account_id
            .as_deref()
            .map(|id| state.account(id))
            .transpose()?;
        let name = Some(name.trim().to_string())
            .filter(|n| !n.is_empty())
            .or_else(|| current.as_ref().map(|a| a.name.clone()))
            .or(signed_in.label)
            .unwrap_or_else(|| service.title().to_string());
        let account = match &current {
            Some(current) => state.accounts.lock().unwrap().update(
                &current.id,
                &name,
                Some(&signed_in.config),
            )?,
            None => state
                .accounts
                .lock()
                .unwrap()
                .add(&name, &signed_in.config)?,
        };
        state.replace_provider(&account.id, provider);
        // Only the user's own app is remembered: the built-in one always
        // comes from the binary, so an update can replace it.
        if let Some(custom) = &custom_app {
            if let Err(err) = accounts::save_oauth_app(service, custom) {
                tracing::warn!("{err:#}");
            }
        }
        Ok(account)
    }
    .await
    .map_err(to_msg)
}

#[tauri::command]
fn oauth_cancel(state: State<'_, AppState>) {
    if let Some(cancel) = state.sign_in_cancel.lock().unwrap().take() {
        let _ = cancel.send(());
    }
}

/// Adds an encrypted folder in account `folder.base`. An empty folder or
/// one encrypted with the same password is accepted.
#[tauri::command]
async fn add_crypt_folder(
    state: State<'_, AppState>,
    name: String,
    folder: CryptFolder,
) -> CmdResult<Account> {
    async {
        if state.account(&folder.base)?.kind == CRYPT_KIND {
            bail!("An encrypted folder cannot be placed inside another one");
        }
        let base = state.provider(&folder.base)?;
        let (root, config) = (folder.root.clone(), folder.config.clone());
        // Key derivation is deliberately slow.
        let provider =
            tauri::async_runtime::spawn_blocking(move || CryptProvider::new(base, &root, &config))
                .await??;
        provider.verify_password().await?;
        let account = state.accounts.lock().unwrap().add_crypt(&name, &folder)?;
        state
            .providers
            .lock()
            .unwrap()
            .insert(account.id.clone(), Arc::new(provider));
        Ok(account)
    }
    .await
    .map_err(to_msg)
}

/// The stored credentials of an account for its settings form; `None` for
/// an encrypted folder, whose password is not shown.
#[tauri::command]
fn account_config(state: State<'_, AppState>, id: String) -> CmdResult<Option<ProviderConfig>> {
    let read = || {
        if state.account(&id)?.kind == CRYPT_KIND {
            return Ok(None);
        }
        state.accounts.lock().unwrap().credentials(&id).map(Some)
    };
    read().map_err(to_msg)
}

/// Renames an account and, with `config`, replaces its credentials once
/// they are checked.
#[tauri::command]
async fn update_account(
    state: State<'_, AppState>,
    id: String,
    name: String,
    config: Option<ProviderConfig>,
) -> CmdResult<Account> {
    async {
        let provider = match &config {
            Some(config) => {
                let provider: Arc<dyn StorageProvider> = config.build()?.into();
                provider
                    .list_dir("/")
                    .await
                    .with_context(|| format!("Cannot connect to {}", provider.name()))?;
                Some(provider)
            }
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
    .await
    .map_err(to_msg)
}

/// Saves the order in which accounts are listed.
#[tauri::command]
fn reorder_accounts(state: State<'_, AppState>, ids: Vec<String>) -> CmdResult<()> {
    state.accounts.lock().unwrap().reorder(&ids).map_err(to_msg)
}

/// Refuses to remove an account that encrypted folders are stored in.
#[tauri::command]
fn remove_account(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let remove = || {
        let store = state.accounts.lock().unwrap();
        let dependent: Vec<String> = store
            .list()?
            .into_iter()
            .filter(|a| a.kind == CRYPT_KIND)
            .filter(|a| store.crypt_folder(&a.id).is_ok_and(|f| f.base == id))
            .map(|a| format!("«{}»", a.name))
            .collect();
        if !dependent.is_empty() {
            bail!(
                "Encrypted folders {} are stored in this account; remove them first",
                dependent.join(", ")
            );
        }
        state.providers.lock().unwrap().remove(&id);
        store.remove(&id)
    };
    remove().map_err(to_msg)
}

/// Saves all accounts with their credentials into file `path`, encrypted
/// with `password`.
#[tauri::command]
async fn export_accounts(
    state: State<'_, AppState>,
    path: String,
    password: String,
) -> CmdResult<usize> {
    async {
        let backup = state.accounts.lock().unwrap().export()?;
        let count = backup.account_count();
        let data = serde_json::to_vec(&backup)?;
        // Key derivation is deliberately slow.
        let file =
            tauri::async_runtime::spawn_blocking(move || belka_core::crypt::seal(&password, &data))
                .await??;
        std::fs::write(&path, file).with_context(|| format!("Failed to write '{path}'"))?;
        Ok(count)
    }
    .await
    .map_err(to_msg)
}

/// Adds the accounts from a file made by [`export_accounts`].
#[tauri::command]
async fn import_accounts(
    state: State<'_, AppState>,
    path: String,
    password: String,
) -> CmdResult<accounts::Imported> {
    async {
        let file = std::fs::read(&path).with_context(|| format!("Failed to read '{path}'"))?;
        let data = tauri::async_runtime::spawn_blocking(move || {
            belka_core::crypt::unseal(&password, &file)
        })
        .await??;
        let backup = serde_json::from_slice(&data).context("The file is damaged")?;
        state.accounts.lock().unwrap().import(backup)
    }
    .await
    .map_err(to_msg)
}

#[tauri::command]
async fn list_dir(
    state: State<'_, AppState>,
    account_id: String,
    path: String,
) -> CmdResult<Vec<RemoteFileInfo>> {
    async {
        let provider = state.provider(&account_id)?;
        provider.list_dir(&normalize_remote_dir(&path)).await
    }
    .await
    .map_err(to_msg)
}

/// Creates folder `name` in `parent` and returns its path.
#[tauri::command]
async fn cloud_mkdir(
    state: State<'_, AppState>,
    account_id: String,
    parent: String,
    name: String,
) -> CmdResult<String> {
    async {
        let provider = state.provider(&account_id)?;
        let path = join_remote(&normalize_remote_dir(&parent), check_name(&name)?);
        // Creating a folder that exists succeeds silently in most clouds.
        if provider.get_file_info(&path).await?.is_some() {
            bail!("'{}' already exists", name.trim());
        }
        provider.ensure_dir(&path).await?;
        Ok(path)
    }
    .await
    .map_err(to_msg)
}

/// Renames a cloud item and returns its new path.
#[tauri::command]
async fn cloud_rename(
    state: State<'_, AppState>,
    account_id: String,
    path: String,
    new_name: String,
    is_dir: bool,
) -> CmdResult<String> {
    async {
        let provider = state.provider(&account_id)?;
        let target = rename_target(&path, &new_name)?;
        provider.rename(&path, &new_name, is_dir).await?;
        if is_dir {
            state.transfers.forget_dir(&account_id, &path).await;
        }
        Ok(target)
    }
    .await
    .map_err(to_msg)
}

#[derive(Deserialize)]
struct CloudItem {
    path: String,
    is_dir: bool,
}

/// Deletes cloud items permanently, one by one; stops at the first failure.
#[tauri::command]
async fn cloud_delete(
    state: State<'_, AppState>,
    account_id: String,
    items: Vec<CloudItem>,
) -> CmdResult<()> {
    async {
        let provider = state.provider(&account_id)?;
        for item in &items {
            provider.delete(&item.path, item.is_dir).await?;
            if item.is_dir {
                state.transfers.forget_dir(&account_id, &item.path).await;
            }
        }
        Ok(())
    }
    .await
    .map_err(to_msg)
}

/// Link settings as the page sends them; empty values mean "not set".
#[derive(Deserialize)]
struct ShareParams {
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
async fn cloud_share(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    items: Vec<CloudItem>,
    options: Option<ShareParams>,
) -> CmdResult<Vec<String>> {
    async {
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
    .await
    .map_err(to_msg)
}

/// Closes access by link to cloud items.
#[tauri::command]
async fn cloud_unshare(
    state: State<'_, AppState>,
    account_id: String,
    items: Vec<CloudItem>,
) -> CmdResult<()> {
    async {
        let provider = state.provider(&account_id)?;
        for item in &items {
            provider.unshare(&item.path, item.is_dir).await?;
        }
        Ok(())
    }
    .await
    .map_err(to_msg)
}

/// Downloads a cloud file into the preview cache (once) and returns where it
/// is; with `text`, also its content.
#[tauri::command]
async fn preview_file(
    state: State<'_, AppState>,
    account_id: String,
    path: String,
    size: u64,
    modified: Option<String>,
    text: bool,
) -> CmdResult<preview::Preview> {
    async {
        let provider = state.provider(&account_id)?;
        let item = preview::Item {
            account_id: &account_id,
            path: &normalize_remote_dir(&path),
            size,
            modified: modified.as_deref(),
        };
        state.previews.load(&*provider, item, text).await
    }
    .await
    .map_err(to_msg)
}

#[tauri::command]
fn preview_progress(state: State<'_, AppState>) -> preview::PreviewProgress {
    state.previews.progress()
}

#[tauri::command]
fn preview_cancel(state: State<'_, AppState>) {
    state.previews.cancel();
}

/// Opens a cached preview in the program the system associates with it.
#[tauri::command]
fn preview_open(app: tauri::AppHandle, state: State<'_, AppState>, path: String) -> CmdResult<()> {
    let path = state.previews.cached(&path).map_err(to_msg)?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|err| err.to_string())
}

/// Opens a local file in the program the system associates with it.
#[tauri::command]
fn local_open(app: tauri::AppHandle, path: String) -> CmdResult<()> {
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn local_mkdir(parent: String, name: String) -> CmdResult<String> {
    local::mkdir(&parent, &name).map_err(to_msg)
}

#[tauri::command]
fn local_rename(path: String, new_name: String) -> CmdResult<String> {
    local::rename(&path, &new_name).map_err(to_msg)
}

#[tauri::command]
async fn local_delete(paths: Vec<String>) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || local::delete(&paths))
        .await
        .map_err(|err| err.to_string())?
        .map_err(to_msg)
}

#[derive(Deserialize)]
struct RemoteItem {
    path: String,
    is_dir: bool,
    size: u64,
}

/// Queues downloads of files and whole folders into `local_dir`.
/// Returns the number of queued files.
#[tauri::command]
async fn download(
    state: State<'_, AppState>,
    account_id: String,
    items: Vec<RemoteItem>,
    local_dir: String,
) -> CmdResult<usize> {
    async {
        let provider = state.provider(&account_id)?;
        let account_name = state.account_name(&account_id)?;
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
    .await
    .map_err(to_msg)
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

/// Clouds such as Google Drive allow equal names in one folder, and
/// sanitizing can make different names equal; such files get " (2)" etc.
fn unique_path(path: PathBuf, taken: &mut HashSet<String>) -> PathBuf {
    // Windows and macOS file systems are case-insensitive.
    let key = |p: &Path| p.to_string_lossy().to_lowercase();
    if taken.insert(key(&path)) {
        return path;
    }
    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (2..)
        .map(|n| path.with_file_name(format!("{stem} ({n}){ext}")))
        .find(|candidate| taken.insert(key(candidate)))
        .expect("an unused name exists")
}

/// Queues uploads of local files and folders into `remote_dir`.
/// Returns the number of queued files.
#[tauri::command]
async fn upload(
    state: State<'_, AppState>,
    account_id: String,
    local_paths: Vec<String>,
    remote_dir: String,
) -> CmdResult<usize> {
    async {
        let provider = state.provider(&account_id)?;
        let account_name = state.account_name(&account_id)?;
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
    .await
    .map_err(to_msg)
}

/// Local files with their remote paths, plus remote folders that contain no files.
type UploadPlan = (Vec<(PathBuf, String)>, Vec<String>);

/// Builds the upload plan for the selected files and folders.
/// Symlinked folders are skipped to avoid cycles.
fn expand_upload(paths: &[PathBuf], remote_dir: &str) -> Result<UploadPlan> {
    let mut files = Vec::new();
    let mut empty_dirs = Vec::new();
    let mut stack: Vec<(PathBuf, String)> = Vec::new();

    for path in paths {
        let name = path
            .file_name()
            .with_context(|| format!("Invalid path '{}'", path.display()))?
            .to_string_lossy();
        let remote = join_remote(remote_dir, &name);
        let meta = std::fs::metadata(path)
            .with_context(|| format!("Cannot access '{}'", path.display()))?;
        if meta.is_dir() {
            stack.push((path.clone(), remote));
        } else {
            files.push((path.clone(), remote));
        }
    }

    while let Some((dir, remote)) = stack.pop() {
        let mut has_entries = false;
        let entries = std::fs::read_dir(&dir)
            .with_context(|| format!("Failed to read folder '{}'", dir.display()))?;
        for entry in entries {
            let entry =
                entry.with_context(|| format!("Failed to read folder '{}'", dir.display()))?;
            let path = entry.path();
            let child = join_remote(&remote, &entry.file_name().to_string_lossy());
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                stack.push((path, child));
                has_entries = true;
            } else if file_type.is_file()
                || (file_type.is_symlink() && std::fs::metadata(&path).is_ok_and(|m| m.is_file()))
            {
                files.push((path, child));
                has_entries = true;
            }
        }
        if !has_entries {
            empty_dirs.push(remote);
        }
    }
    if files.is_empty() && empty_dirs.is_empty() {
        bail!("Nothing to upload");
    }
    Ok((files, empty_dirs))
}

/// Which of `names` already exist in the local folder `dir`.
#[tauri::command]
fn local_conflicts(dir: String, names: Vec<String>) -> Vec<String> {
    names
        .into_iter()
        .filter(|name| Path::new(&dir).join(sanitize_file_name(name)).exists())
        .collect()
}

#[tauri::command]
async fn list_local(app: tauri::AppHandle, path: String) -> CmdResult<local::LocalListing> {
    let paths = app.path();
    let places: Vec<local::Place> = [
        ("Рабочий стол", paths.desktop_dir()),
        ("Загрузки", paths.download_dir()),
        ("Документы", paths.document_dir()),
        ("Домашняя папка", paths.home_dir()),
    ]
    .into_iter()
    .filter_map(|(name, path)| path.ok().map(|path| local::Place { name, path }))
    .collect();
    // Slow or disconnected drives must not block the UI thread.
    tauri::async_runtime::spawn_blocking(move || local::list(&path, &places))
        .await
        .map_err(|err| err.to_string())?
        .map_err(to_msg)
}

#[tauri::command]
fn list_transfers(state: State<'_, AppState>) -> TransferList {
    state.transfers.list()
}

#[tauri::command]
fn cancel_transfer(state: State<'_, AppState>, id: u64) {
    state.transfers.cancel(id);
}

#[tauri::command]
fn cancel_all_transfers(state: State<'_, AppState>) {
    state.transfers.cancel_all();
}

#[tauri::command]
fn retry_transfer(state: State<'_, AppState>, id: u64) -> CmdResult<()> {
    let Some(spec) = state.transfers.retry_spec(id) else {
        return Ok(());
    };
    let provider = state.provider(&spec.account_id).map_err(to_msg)?;
    state.transfers.retry(id, provider);
    Ok(())
}

#[tauri::command]
fn clear_transfers(state: State<'_, AppState>) {
    state.transfers.clear_finished();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // BELKA_LOG takes levels per target, e.g. "debug,reqwest=trace".
    let filter = std::env::var("BELKA_LOG")
        .ok()
        .and_then(|value| value.parse::<Targets>().ok())
        .unwrap_or_else(|| {
            Targets::new()
                .with_default(LevelFilter::WARN)
                .with_target("belka_core", LevelFilter::INFO)
                .with_target("belka_gui_lib", LevelFilter::INFO)
        });
    tracing_subscriber::fmt()
        .with_max_level(LevelFilter::TRACE)
        .with_target(false)
        .finish()
        .with(filter)
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        // Position, size and maximized state survive restarts; visibility is
        // left to the page so the window still appears only once drawn.
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(StateFlags::all() - StateFlags::VISIBLE)
                .build(),
        )
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let preview_dir = app.path().app_cache_dir()?.join("preview");
            app.manage(AppState {
                accounts: Mutex::new(AccountStore::new(config_dir)),
                providers: Mutex::new(HashMap::new()),
                transfers: Transfers::new(),
                sign_in_cancel: Mutex::new(None),
                previews: preview::Previews::new(preview_dir),
            });
            // The page shows the hidden window once it is drawn; if it fails
            // to load, the window still appears.
            if let Some(window) = app.get_webview_window("main") {
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    if !window.is_visible().unwrap_or(true) {
                        let _ = window.show();
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_accounts,
            add_account,
            add_crypt_folder,
            oauth_apps,
            oauth_sign_in,
            oauth_cancel,
            account_config,
            update_account,
            reorder_accounts,
            remove_account,
            export_accounts,
            import_accounts,
            list_dir,
            download,
            upload,
            local_conflicts,
            list_local,
            cloud_mkdir,
            cloud_rename,
            cloud_delete,
            cloud_share,
            cloud_unshare,
            preview_file,
            preview_progress,
            preview_cancel,
            preview_open,
            local_open,
            local_mkdir,
            local_rename,
            local_delete,
            list_transfers,
            cancel_transfer,
            cancel_all_transfers,
            retry_transfer,
            clear_transfers
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_path_numbers_duplicates_case_insensitively() {
        let mut taken = HashSet::new();
        let dir = Path::new("dl");
        assert_eq!(
            unique_path(dir.join("a.txt"), &mut taken),
            dir.join("a.txt")
        );
        assert_eq!(
            unique_path(dir.join("A.txt"), &mut taken),
            dir.join("A (2).txt")
        );
        assert_eq!(
            unique_path(dir.join("a.txt"), &mut taken),
            dir.join("a (3).txt")
        );
        assert_eq!(unique_path(dir.join("b"), &mut taken), dir.join("b"));
    }

    #[test]
    fn expand_upload_walks_folders() {
        let tmp = std::env::temp_dir().join(format!("cu-gui-test-{}", std::process::id()));
        let root = tmp.join("photos");
        std::fs::create_dir_all(root.join("2024")).unwrap();
        std::fs::create_dir_all(root.join("empty")).unwrap();
        std::fs::write(root.join("2024").join("a.jpg"), b"x").unwrap();
        std::fs::write(tmp.join("note.txt"), b"y").unwrap();

        let (mut files, empty) =
            expand_upload(&[root.clone(), tmp.join("note.txt")], "/Backup").unwrap();
        files.sort_by(|a, b| a.1.cmp(&b.1));
        let remotes: Vec<&str> = files.iter().map(|(_, r)| r.as_str()).collect();
        assert_eq!(remotes, ["/Backup/note.txt", "/Backup/photos/2024/a.jpg"]);
        assert_eq!(empty, ["/Backup/photos/empty"]);

        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
