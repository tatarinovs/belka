//! Shared application state and operations for Desktop and Android GUIs.

use crate::error::{translate, AppResult};
use crate::local::{self, LocalListing, Place};
use crate::preview::{self, Preview, PreviewProgress, Previews};
use crate::stream::{self, Streams};
use crate::transfers::{
    Activity, CopyFrom, Direction, Spec, TransferFilter, TransferList, Transfers,
};
use anyhow::{bail, Context, Result};
use belka_core::oauth::{self, OAuthApp, Service};
use belka_core::storage::crypt::CryptProvider;
use belka_core::storage::{check_name, rename_target, Quota, ShareOptions};
use belka_core::sync::{self, Filter, RemoteListing};
use belka_core::utils::path::{
    join_remote, normalize_remote_dir, sanitize_file_name, split_remote, unique_path,
};
use belka_core::{ProviderConfig, RemoteFileInfo, StorageProvider};
use belka_store::accounts::{Account, AccountSource, AccountStore, Backup, CryptFolder, Imported, CRYPT_KIND};
use belka_store::secrets::Secrets;
use belka_store::tasks::{self, Task, TaskStore, TaskView};
use chrono::{DateTime, NaiveDate, Utc};
use futures_util::stream::{self as fstream, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

pub struct AppCore {
    /// Serializes changes of the account list.
    pub accounts: Mutex<AccountStore>,
    /// Providers are built lazily and reused, so tokens and connections are kept.
    providers: Mutex<HashMap<String, Arc<dyn StorageProvider>>>,
    pub transfers: Arc<Transfers>,
    /// Aborts the browser sign-in in progress.
    sign_in_cancel: Mutex<Option<oneshot::Sender<()>>>,
    pub previews: Previews,
    pub streams: Streams,
    pub tasks: TaskStore,
}

#[derive(Serialize)]
pub struct OAuthApps {
    /// This build has an OAuth app of its own for the service.
    pub builtin: bool,
    /// The user's own OAuth app, saved after the last sign-in with it.
    pub custom: Option<OAuthApp>,
}

#[derive(Deserialize)]
pub struct CloudItem {
    pub path: String,
    pub is_dir: bool,
}

/// Link settings as the UI sends them; empty values mean "not set".
#[derive(Deserialize)]
pub struct ShareParams {
    pub password: Option<String>,
    /// `YYYY-MM-DD`
    pub expires: Option<String>,
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

/// A cloud item as the UI has it from the listing.
#[derive(Deserialize)]
pub struct RemoteItem {
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    #[serde(default)]
    pub md5: Option<String>,
    #[serde(default)]
    pub modified: Option<DateTime<Utc>>,
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

#[derive(Serialize)]
pub struct RetryFailures {
    pub retried: usize,
    pub errors: Vec<String>,
}

#[derive(Serialize)]
pub struct Update {
    pub version: String,
    /// The release page.
    pub page: String,
    /// The Android package of the release, if it has one.
    pub apk: Option<String>,
}

impl AppCore {
    pub fn new(config_dir: &Path, cache_dir: &Path, secrets: Secrets) -> Self {
        Self {
            tasks: TaskStore::new(config_dir),
            accounts: Mutex::new(AccountStore::new(config_dir, secrets)),
            providers: Mutex::new(HashMap::new()),
            transfers: Transfers::new(cache_dir.join("copy")),
            sign_in_cancel: Mutex::new(None),
            previews: Previews::new(cache_dir.join("preview")),
            streams: Streams::default(),
        }
    }

    pub fn provider(&self, account_id: &str) -> Result<Arc<dyn StorageProvider>> {
        if let Some(p) = self.providers.lock().unwrap().get(account_id) {
            return Ok(p.clone());
        }
        let provider: Arc<dyn StorageProvider> = if self.account(account_id)?.kind == CRYPT_KIND {
            let folder = self.accounts.lock().unwrap().crypt_folder(account_id)?;
            if self.account(&folder.base)?.kind == CRYPT_KIND {
                anyhow::bail!("An encrypted folder cannot be placed inside another one");
            }
            // The base comes from the cache, with its tokens and connections.
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

    pub fn account(&self, account_id: &str) -> Result<Account> {
        self.accounts.lock().unwrap().account(account_id)
    }

    /// Uses `provider` for the account from now on. Encrypted folders in it
    /// hold the old one, so they are rebuilt on next use.
    pub fn replace_provider(&self, account_id: &str, provider: Arc<dyn StorageProvider>) {
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

    // ---- accounts ----

    pub fn list_accounts(&self) -> AppResult<Vec<Account>> {
        Ok(self.accounts.lock().unwrap().list()?)
    }

    /// Checks that the credentials work before saving the account.
    pub async fn add_account(&self, name: &str, config: ProviderConfig) -> AppResult<Account> {
        let provider = connect(&config).await?;
        let account = self.accounts.lock().unwrap().add(name, &config)?;
        self.providers
            .lock()
            .unwrap()
            .insert(account.id.clone(), provider);
        Ok(account)
    }

    pub fn oauth_apps(&self, service: Service) -> OAuthApps {
        OAuthApps {
            builtin: service.builtin_app().is_some(),
            custom: self.accounts.lock().unwrap().load_oauth_app(service),
        }
    }

    pub async fn oauth_sign_in(
        &self,
        service: Service,
        custom_app: Option<OAuthApp>,
        name: &str,
        account_id: Option<&str>,
        open_url: impl FnOnce(&str) -> Result<()>,
    ) -> AppResult<Account> {
        let (cancel, cancelled) = oneshot::channel();
        // A sign-in still waiting in another browser tab is abandoned.
        self.sign_in_cancel.lock().unwrap().replace(cancel);
        let oauth_app = match &custom_app {
            Some(custom) => custom.clone(),
            None => service
                .builtin_app()
                .context("This build has no built-in OAuth app; enter your own")?,
        };
        let signed_in = tokio::select! {
            result = oauth::sign_in(service, &oauth_app, open_url) => result?,
            _ = cancelled => return Err(anyhow::anyhow!("Sign-in cancelled").into()),
        };

        let provider = connect(&signed_in.config).await?;
        let current = account_id.map(|id| self.account(id)).transpose()?;
        let name = Some(name.trim().to_string())
            .filter(|n| !n.is_empty())
            .or_else(|| current.as_ref().map(|a| a.name.clone()))
            .or(signed_in.label)
            .unwrap_or_else(|| service.title().to_string());
        let account = {
            let store = self.accounts.lock().unwrap();
            match &current {
                Some(current) => store.update(&current.id, &name, Some(&signed_in.config))?,
                None => store.add(&name, &signed_in.config)?,
            }
        };
        self.replace_provider(&account.id, provider);
        // Only the user's own app is remembered: the built-in one always
        // comes from the binary, so an update can replace it.
        if let Some(custom) = &custom_app {
            if let Err(err) = self
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

    pub fn oauth_cancel(&self) {
        if let Some(cancel) = self.sign_in_cancel.lock().unwrap().take() {
            let _ = cancel.send(());
        }
    }

    /// Adds an encrypted folder in account `folder.base`. An empty folder or
    /// one encrypted with the same password is accepted.
    pub async fn add_crypt_folder(&self, name: &str, folder: CryptFolder) -> AppResult<Account> {
        if self.account(&folder.base)?.kind == CRYPT_KIND {
            return Err(
                anyhow::anyhow!("An encrypted folder cannot be placed inside another one").into(),
            );
        }
        let base = self.provider(&folder.base)?;
        let (root, config) = (folder.root.clone(), folder.config.clone());
        // Key derivation is deliberately slow.
        let provider =
            tokio::task::spawn_blocking(move || CryptProvider::new(base, &root, &config))
                .await
                .map_err(anyhow::Error::from)??;
        provider.verify_password().await?;
        let account = self.accounts.lock().unwrap().add_crypt(name, &folder)?;
        self.providers
            .lock()
            .unwrap()
            .insert(account.id.clone(), Arc::new(provider));
        Ok(account)
    }

    /// The stored credentials of an account for its settings form; `None` for
    /// an encrypted folder, whose password is not shown.
    pub fn account_config(&self, id: &str) -> AppResult<Option<ProviderConfig>> {
        if self.account(id)?.kind == CRYPT_KIND {
            return Ok(None);
        }
        Ok(Some(self.accounts.lock().unwrap().credentials(id)?))
    }

    /// Renames an account and, with `config`, replaces its credentials once
    /// they are checked.
    pub async fn update_account(
        &self,
        id: &str,
        name: &str,
        config: Option<ProviderConfig>,
    ) -> AppResult<Account> {
        let provider = match &config {
            Some(config) => Some(connect(config).await?),
            None => None,
        };
        let account = self
            .accounts
            .lock()
            .unwrap()
            .update(id, name, config.as_ref())?;
        if let Some(provider) = provider {
            self.replace_provider(id, provider);
        }
        Ok(account)
    }

    /// Saves the order in which accounts are listed.
    pub fn reorder_accounts(&self, ids: &[String]) -> AppResult<()> {
        Ok(self.accounts.lock().unwrap().reorder(ids)?)
    }

    /// Refuses to remove an account that encrypted folders are stored in or
    /// that backups go to.
    pub fn remove_account(&self, id: &str) -> AppResult<()> {
        let quoted = |names: Vec<String>| {
            names
                .iter()
                .map(|n| format!("«{n}»"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let store = self.accounts.lock().unwrap();
        let folders = store.crypt_folders_in(id);
        if !folders.is_empty() {
            let names = quoted(folders);
            return Err(anyhow::anyhow!(
                "Encrypted folders {names} are stored in this account; remove them first"
            )
            .into());
        }
        let tasks = self.tasks.using_account(id);
        if !tasks.is_empty() {
            let names = quoted(tasks);
            return Err(
                anyhow::anyhow!("Backups {names} go to this account; remove them first").into(),
            );
        }
        self.providers.lock().unwrap().remove(id);
        Ok(store.remove(id)?)
    }

    /// Seals all accounts with their credentials into encrypted bytes.
    pub async fn export_accounts_bytes(&self, password: String) -> AppResult<(usize, Vec<u8>)> {
        let backup = self.accounts.lock().unwrap().export()?;
        let count = backup.account_count();
        // Key derivation is deliberately slow.
        let file = tokio::task::spawn_blocking(move || backup.seal(&password))
            .await
            .map_err(anyhow::Error::from)??;
        Ok((count, file))
    }

    /// Adds the accounts from encrypted backup bytes.
    pub async fn import_accounts_bytes(
        &self,
        data: Vec<u8>,
        password: String,
    ) -> AppResult<Imported> {
        let backup = tokio::task::spawn_blocking(move || Backup::open(&data, &password))
            .await
            .map_err(anyhow::Error::from)??;
        Ok(self.accounts.lock().unwrap().import(backup)?)
    }

    // ---- cloud ----

    pub async fn list_dir(&self, account_id: &str, path: &str) -> AppResult<Vec<RemoteFileInfo>> {
        let provider = self.provider(account_id)?;
        Ok(provider.list_dir(&normalize_remote_dir(path)).await?)
    }

    pub async fn cloud_quota(&self, account_id: &str, path: &str) -> AppResult<Option<Quota>> {
        let provider = self.provider(account_id)?;
        Ok(provider.quota(&normalize_remote_dir(path)).await?)
    }

    pub async fn cloud_mkdir(&self, account_id: &str, parent: &str, name: &str) -> AppResult<String> {
        let provider = self.provider(account_id)?;
        let path = join_remote(&normalize_remote_dir(parent), check_name(name)?);
        // Creating a folder that exists succeeds silently in most clouds.
        if provider.get_file_info(&path).await?.is_some() {
            return Err(anyhow::anyhow!("'{}' already exists", name.trim()).into());
        }
        provider.ensure_dir(&path).await?;
        Ok(path)
    }

    pub async fn cloud_rename(
        &self,
        account_id: &str,
        path: &str,
        new_name: &str,
        is_dir: bool,
    ) -> AppResult<String> {
        let provider = self.provider(account_id)?;
        let target = rename_target(path, new_name)?;
        provider.rename(path, new_name, is_dir).await?;
        if is_dir {
            self.transfers.forget_dir(account_id, path).await;
        }
        Ok(target)
    }

    pub async fn cloud_delete(&self, account_id: &str, items: &[CloudItem]) -> AppResult<()> {
        let provider = self.provider(account_id)?;
        for item in items {
            provider.delete(&item.path, item.is_dir).await?;
            if item.is_dir {
                self.transfers.forget_dir(account_id, &item.path).await;
            }
        }
        Ok(())
    }

    pub async fn cloud_share(
        &self,
        account_id: &str,
        items: &[CloudItem],
        options: Option<ShareParams>,
    ) -> AppResult<Vec<String>> {
        let provider = self.provider(account_id)?;
        let options = options.map(ShareParams::parse).transpose()?;
        let mut links = Vec::with_capacity(items.len());
        for item in items {
            links.push(match &options {
                Some(options) => {
                    provider
                        .share_with(&item.path, item.is_dir, options)
                        .await?
                }
                None => provider.share(&item.path, item.is_dir).await?,
            });
        }
        Ok(links)
    }

    pub async fn cloud_unshare(&self, account_id: &str, items: &[CloudItem]) -> AppResult<()> {
        let provider = self.provider(account_id)?;
        for item in items {
            provider.unshare(&item.path, item.is_dir).await?;
        }
        Ok(())
    }

    pub async fn preview_file(
        &self,
        account_id: &str,
        path: &str,
        size: u64,
        modified: Option<&str>,
        text: bool,
    ) -> AppResult<Preview> {
        let provider = self.provider(account_id)?;
        let path = normalize_remote_dir(path);
        let item = preview::Item {
            account_id,
            path: &path,
            size,
            modified,
        };
        Ok(self.previews.load(&*provider, item, text).await?)
    }

    pub fn preview_progress(&self) -> PreviewProgress {
        self.previews.progress()
    }

    pub fn preview_cancel(&self) {
        self.previews.cancel();
        self.streams.close();
    }

    pub fn preview_cached(&self, path: &str) -> AppResult<PathBuf> {
        Ok(self.previews.cached(path)?)
    }

    pub async fn stream_open(
        &self,
        account_id: &str,
        path: &str,
        size: u64,
    ) -> AppResult<Option<String>> {
        let provider = self.provider(account_id)?;
        let path = normalize_remote_dir(path);
        match self.streams.open(provider, &path, size).await {
            Ok(url) => Ok(Some(url)),
            Err(err) => {
                tracing::info!("Cannot stream '{path}', downloading instead: {err:#}");
                Ok(None)
            }
        }
    }

    pub async fn stream_open_player(
        &self,
        account_id: &str,
        path: &str,
        size: u64,
    ) -> AppResult<(String, &'static str, String)> {
        let provider = self.provider(account_id)?;
        let path = normalize_remote_dir(path);
        let url = match self.streams.open_player(provider, &path, size).await {
            Ok(url) => url,
            Err(err) => {
                tracing::info!("Cannot stream '{path}' to a player: {err:#}");
                return Err(anyhow::anyhow!("The cloud cannot stream this file").into());
            }
        };
        let name = path.rsplit('/').next().unwrap_or_default().to_string();
        let mime = stream::mime_type(&path);
        Ok((url, mime, name))
    }

    // ---- local & transfers ----

    pub fn local_mkdir(&self, parent: &str, name: &str) -> AppResult<String> {
        Ok(local::mkdir(parent, name)?)
    }

    pub fn local_rename(&self, path: &str, new_name: &str) -> AppResult<String> {
        Ok(local::rename(path, new_name)?)
    }

    pub async fn local_delete(&self, paths: Vec<String>) -> AppResult<()> {
        let deleted = tokio::task::spawn_blocking(move || local::delete(&paths))
            .await
            .map_err(anyhow::Error::from)?;
        Ok(deleted?)
    }

    pub async fn download(
        &self,
        account_id: &str,
        items: Vec<RemoteItem>,
        local_dir: &str,
    ) -> AppResult<usize> {
        let provider = self.provider(account_id)?;
        let account_name = self.account(account_id)?.name;
        let files = expand_download(&*provider, items, Path::new(local_dir)).await?;
        let count = files.len();
        for (remote, local) in files {
            let spec = Spec {
                direction: Direction::Download,
                account_id: account_id.to_string(),
                account_name: account_name.clone(),
                local,
                remote: remote.path.clone(),
                size: remote.size,
                known: Some(remote),
                from: None,
            };
            self.transfers.enqueue(spec, provider.clone());
        }
        Ok(count)
    }

    pub async fn upload(
        &self,
        account_id: &str,
        local_paths: Vec<String>,
        remote_dir: &str,
    ) -> AppResult<usize> {
        let provider = self.provider(account_id)?;
        let account_name = self.account(account_id)?.name;
        let remote_dir = normalize_remote_dir(remote_dir);
        let paths: Vec<PathBuf> = local_paths.into_iter().map(PathBuf::from).collect();
        let target = remote_dir.clone();
        let (files, empty_dirs) =
            tokio::task::spawn_blocking(move || expand_upload(&paths, &target))
                .await
                .context("Failed to scan local files")??;

        for dir in &empty_dirs {
            self.transfers
                .ensure_dir(account_id, dir, &*provider)
                .await?;
        }
        let targets: Vec<&str> = files.iter().map(|(_, remote)| remote.as_str()).collect();
        let mut existing = existing_targets(&*provider, &remote_dir, &targets).await;
        let count = files.len();
        for (local, remote) in files {
            let spec = Spec {
                direction: Direction::Upload,
                account_id: account_id.to_string(),
                account_name: account_name.clone(),
                local,
                known: existing.remove(&remote),
                remote,
                size: 0,
                from: None,
            };
            self.transfers.enqueue(spec, provider.clone());
        }
        Ok(count)
    }

    pub async fn cloud_copy(
        &self,
        account_id: &str,
        items: Vec<RemoteItem>,
        target_account_id: &str,
        target_dir: &str,
    ) -> AppResult<usize> {
        let source = self.provider(account_id)?;
        let target = self.provider(target_account_id)?;
        let source_name = self.account(account_id)?.name;
        let target_name = self.account(target_account_id)?.name;
        let target_dir = normalize_remote_dir(target_dir);
        let items: Vec<RemoteFileInfo> = items.into_iter().map(RemoteItem::into_info).collect();
        if account_id == target_account_id {
            check_copy_target(&items, &target_dir)?;
        }

        let (files, empty_dirs) = expand_copy(&*source, items, &target_dir).await?;
        for dir in &empty_dirs {
            self.transfers
                .ensure_dir(target_account_id, dir, &*target)
                .await?;
        }
        let targets: Vec<&str> = files.iter().map(|(_, to)| to.as_str()).collect();
        let mut existing = existing_targets(&*target, &target_dir, &targets).await;
        let count = files.len();
        for (file, to) in files {
            let spec = Spec {
                direction: Direction::Copy,
                account_id: target_account_id.to_string(),
                account_name: target_name.clone(),
                local: PathBuf::new(),
                known: existing.remove(&to),
                remote: to,
                size: file.size,
                from: Some(CopyFrom {
                    account_id: account_id.to_string(),
                    account_name: source_name.clone(),
                    file,
                }),
            };
            self.transfers
                .enqueue_copy(spec, target.clone(), source.clone());
        }
        Ok(count)
    }

    pub fn local_conflicts(&self, dir: &str, names: Vec<String>) -> Vec<String> {
        names
            .into_iter()
            .filter(|name| Path::new(dir).join(sanitize_file_name(name)).exists())
            .collect()
    }

    pub async fn list_local(&self, path: String, places: Vec<Place>) -> AppResult<LocalListing> {
        let listing = tokio::task::spawn_blocking(move || local::list(&path, &places))
            .await
            .map_err(anyhow::Error::from)?;
        Ok(listing?)
    }

    pub fn list_transfers(&self, filter: Option<TransferFilter>) -> TransferList {
        match filter.unwrap_or_default() {
            TransferFilter::All => self.transfers.list(),
            filter => self.transfers.list_filtered(filter),
        }
    }

    pub fn cancel_transfer(&self, id: u64) {
        self.transfers.cancel(id);
    }

    pub fn cancel_all_transfers(&self) {
        self.transfers.cancel_all();
    }

    pub fn retry_transfer(&self, id: u64) -> AppResult<()> {
        let Some(spec) = self.transfers.retry_spec(id) else {
            return Ok(());
        };
        let provider = self.provider(&spec.account_id)?;
        let source = spec
            .from
            .as_ref()
            .map(|from| self.provider(&from.account_id))
            .transpose()?;
        self.transfers.retry(id, provider, source);
        Ok(())
    }

    pub fn retry_failed_transfers(&self) -> RetryFailures {
        let mut result = RetryFailures {
            retried: 0,
            errors: Vec::new(),
        };
        for id in self.transfers.failed_ids() {
            let Some(spec) = self.transfers.retry_spec(id) else {
                continue;
            };
            let providers = (|| -> Result<_> {
                let target = self.provider(&spec.account_id)?;
                let source = spec
                    .from
                    .as_ref()
                    .map(|from| self.provider(&from.account_id))
                    .transpose()?;
                Ok((target, source))
            })();
            match providers {
                Ok((target, source)) => {
                    if self.transfers.retry(id, target, source) {
                        result.retried += 1;
                    }
                }
                Err(err) => result.errors.push(translate(&format!("{err:#}"))),
            }
        }
        result
    }

    pub fn clear_transfers(&self) {
        self.transfers.clear_finished();
    }

    pub fn transfer_activity(&self) -> Activity {
        let mut activity = self.transfers.activity();
        activity.player = self.streams.player_activity().unwrap_or_default();
        activity
    }

    // ---- backups ----

    pub fn backup_tasks(&self) -> AppResult<Vec<TaskView>> {
        let mut views = self.tasks.views()?;
        for run in views.iter_mut().flat_map(|v| v.task.history.iter_mut()) {
            run.error = run.error.as_deref().map(translate);
            for (_, reason) in &mut run.failures {
                *reason = translate(reason);
            }
        }
        Ok(views)
    }

    pub fn backup_list(&self) -> AppResult<Vec<Task>> {
        Ok(self.tasks.list()?)
    }

    pub fn backup_save(&self, task: Task) -> AppResult<Task> {
        self.account(&task.account_id)?;
        if tasks::is_running(&task.id) {
            // A run must not record its index for a target the task no longer has.
            if let Ok(old) = self.tasks.task(&task.id) {
                if (&old.local, &old.account_id, &old.remote)
                    != (&task.local, &task.account_id, &task.remote)
                {
                    tasks::cancel(&task.id);
                }
            }
        }
        Ok(self.tasks.save(task)?)
    }

    pub fn backup_remove(&self, id: &str) -> AppResult<()> {
        tasks::cancel(id);
        self.tasks.remove(id)?;
        Ok(())
    }

    pub fn backup_task(&self, id: &str) -> AppResult<Task> {
        Ok(self.tasks.task(id)?)
    }

    pub fn backup_cancel(&self, id: &str) {
        tasks::cancel(id);
    }

    pub async fn run_backup_task(&self, id: &str) -> Result<()> {
        let provider = |account: &str| self.provider(account);
        tasks::run(&self.tasks, id, &provider).await?;
        Ok(())
    }
}

/// Builds the provider and checks that the credentials work.
async fn connect(config: &ProviderConfig) -> Result<Arc<dyn StorageProvider>> {
    let provider: Arc<dyn StorageProvider> = config.build()?.into();
    provider
        .list_dir("/")
        .await
        .with_context(|| format!("Cannot connect to {}", provider.name()))?;
    Ok(provider)
}

/// The latest release on GitHub if it is newer than `current_version`.
pub async fn check_update(current_version: &str) -> AppResult<Option<Update>> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
        html_url: String,
        #[serde(default)]
        draft: bool,
        #[serde(default)]
        prerelease: bool,
        #[serde(default)]
        assets: Vec<Asset>,
    }
    #[derive(Deserialize)]
    struct Asset {
        name: String,
        browser_download_url: String,
    }
    async {
        let release: Release = reqwest::Client::builder()
            .user_agent(format!("belka-gui/{current_version}"))
            .timeout(Duration::from_secs(20))
            .build()?
            .get("https://api.github.com/repos/tatarinovs/belka/releases/latest")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let version = release.tag_name.trim_start_matches('v').to_string();
        if release.draft || release.prerelease || !is_newer(&version, current_version) {
            return Ok(None);
        }
        // A universal package or the one for 64-bit ARM phones.
        let apk = release
            .assets
            .iter()
            .filter(|a| a.name.ends_with(".apk"))
            .min_by_key(|a| !(a.name.contains("arm64") || a.name.contains("universal")))
            .map(|a| a.browser_download_url.clone());
        Ok(Some(Update {
            version,
            page: release.html_url,
            apk,
        }))
    }
    .await
    .map_err(|err: anyhow::Error| err.context("Failed to check for updates").into())
}

/// Compares dotted versions number by number; a suffix like "-beta" is ignored.
fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.split(['-', '+'])
            .next()
            .unwrap_or_default()
            .split('.')
            .map(|n| n.parse().unwrap_or(0))
            .collect()
    };
    parse(candidate) > parse(current)
}

/// Lists every file under the selected items, mapping cloud names to safe
/// local names. Folders are created locally right away, so empty ones survive.
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
        let remotes: Vec<String> = level.iter().map(|(remote, _)| remote.clone()).collect();
        let listings: Vec<Result<Vec<RemoteFileInfo>>> = fstream::iter(remotes)
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

/// The cloud files already at the targets of `files`, so that identical
/// ones are not uploaded again.
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
/// under `target_dir`, plus the target folders that stay empty.
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
        let remotes: Vec<String> = level.iter().map(|(remote, _)| remote.clone()).collect();
        let listings: Vec<Result<Vec<RemoteFileInfo>>> = fstream::iter(remotes)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_versions() {
        assert!(is_newer("1.2.0", "1.1.0"));
        assert!(is_newer("1.10.0", "1.9.3"));
        assert!(is_newer("2.0", "1.9.9"));
        assert!(!is_newer("1.1.0", "1.1.0"));
        assert!(!is_newer("1.0.9", "1.1.0"));
        assert!(!is_newer("1.1.0-beta", "1.1.0"));
    }

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
