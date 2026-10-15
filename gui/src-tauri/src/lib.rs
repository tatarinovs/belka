mod accounts;
mod local;
mod transfers;

use accounts::{Account, AccountStore};
use anyhow::{bail, Context, Result};
use cloud_uploader_core::storage::{check_name, rename_target};
use cloud_uploader_core::utils::path::{join_remote, normalize_remote_dir, sanitize_file_name};
use cloud_uploader_core::{ProviderConfig, RemoteFileInfo, StorageProvider};
use serde::Deserialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{Manager, State};
use tracing_subscriber::EnvFilter;
use transfers::{Direction, Spec, TransferView, Transfers};

struct AppState {
    accounts: Mutex<AccountStore>,
    /// Providers are built lazily and reused, so tokens and connections are kept.
    providers: Mutex<HashMap<String, Arc<dyn StorageProvider>>>,
    transfers: Arc<Transfers>,
}

impl AppState {
    fn provider(&self, account_id: &str) -> Result<Arc<dyn StorageProvider>> {
        if let Some(p) = self.providers.lock().unwrap().get(account_id) {
            return Ok(p.clone());
        }
        let config = self.accounts.lock().unwrap().credentials(account_id)?;
        let provider: Arc<dyn StorageProvider> = config.build()?.into();
        self.providers
            .lock()
            .unwrap()
            .insert(account_id.to_string(), provider.clone());
        Ok(provider)
    }

    fn account_name(&self, account_id: &str) -> Result<String> {
        self.accounts
            .lock()
            .unwrap()
            .list()?
            .into_iter()
            .find(|a| a.id == account_id)
            .map(|a| a.name)
            .context("Account not found")
    }
}

/// Commands return errors as strings with the full context chain.
type CmdResult<T> = std::result::Result<T, String>;

fn to_msg(err: anyhow::Error) -> String {
    format!("{err:#}")
}

#[tauri::command]
fn list_accounts(state: State<'_, AppState>) -> CmdResult<Vec<Account>> {
    state.accounts.lock().unwrap().list().map_err(to_msg)
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

#[tauri::command]
fn remove_account(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.providers.lock().unwrap().remove(&id);
    state.accounts.lock().unwrap().remove(&id).map_err(to_msg)
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
struct DeleteItem {
    path: String,
    is_dir: bool,
}

/// Deletes cloud items permanently, one by one; stops at the first failure.
#[tauri::command]
async fn cloud_delete(
    state: State<'_, AppState>,
    account_id: String,
    items: Vec<DeleteItem>,
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
fn list_transfers(state: State<'_, AppState>) -> Vec<TransferView> {
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
    let filter = EnvFilter::try_from_env("CLOUD_UPLOADER_LOG").unwrap_or_else(|_| {
        EnvFilter::new("warn,cloud_uploader_core=info,cloud_uploader_gui_lib=info")
    });
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            app.manage(AppState {
                accounts: Mutex::new(AccountStore::new(config_dir)),
                providers: Mutex::new(HashMap::new()),
                transfers: Transfers::new(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_accounts,
            add_account,
            remove_account,
            list_dir,
            download,
            upload,
            local_conflicts,
            list_local,
            cloud_mkdir,
            cloud_rename,
            cloud_delete,
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
