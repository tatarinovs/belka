mod commands;
mod error;
mod local;
mod native;
mod preview;
mod stream;
mod transfers;

use anyhow::Result;
use belka_core::storage::crypt::CryptProvider;
use belka_core::StorageProvider;
use belka_store::accounts::{Account, AccountSource, AccountStore, CRYPT_KIND};
use belka_store::secrets::Secrets;
use belka_store::tasks::TaskStore;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::Manager;
use tokio::sync::oneshot;
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::prelude::*;
use transfers::Transfers;

pub(crate) struct AppState {
    /// Serializes changes of the account list.
    accounts: Mutex<AccountStore>,
    /// Providers are built lazily and reused, so tokens and connections are kept.
    providers: Mutex<HashMap<String, Arc<dyn StorageProvider>>>,
    transfers: Arc<Transfers>,
    /// Aborts the browser sign-in in progress.
    sign_in_cancel: Mutex<Option<oneshot::Sender<()>>>,
    previews: preview::Previews,
    streams: stream::Streams,
    tasks: TaskStore,
}

impl AppState {
    fn provider(&self, account_id: &str) -> Result<Arc<dyn StorageProvider>> {
        if let Some(p) = self.providers.lock().unwrap().get(account_id) {
            return Ok(p.clone());
        }
        let provider: Arc<dyn StorageProvider> = if self.account(account_id)?.kind == CRYPT_KIND {
            let folder = self.accounts.lock().unwrap().crypt_folder(account_id)?;
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

    fn account(&self, account_id: &str) -> Result<Account> {
        self.accounts.lock().unwrap().account(account_id)
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
                .with_target("belka_store", LevelFilter::INFO)
                .with_target("belka_gui_lib", LevelFilter::INFO)
        });
    tracing_subscriber::fmt()
        .with_max_level(LevelFilter::TRACE)
        .with_target(false)
        .finish()
        .with(filter)
        .init();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(native::plugin());
    // Position, size and maximized state survive restarts; visibility is
    // left to the page so the window still appears only once drawn.
    #[cfg(desktop)]
    let builder = builder.plugin(
        tauri_plugin_window_state::Builder::new()
            .with_state_flags(
                tauri_plugin_window_state::StateFlags::all()
                    - tauri_plugin_window_state::StateFlags::VISIBLE,
            )
            .build(),
    );
    builder
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let preview_dir = app.path().app_cache_dir()?.join("preview");
            // On Android the file is in the app's private folder; desktops
            // use the keyring, which the CLI shares.
            let secrets = Secrets::system_or(app.path().app_data_dir()?.join("secrets.json"));
            let transfers = Transfers::new();
            native::watch_transfers(app.handle().clone(), transfers.clone());
            app.manage(AppState {
                tasks: TaskStore::new(&config_dir),
                accounts: Mutex::new(AccountStore::new(&config_dir, secrets)),
                providers: Mutex::new(HashMap::new()),
                transfers,
                sign_in_cancel: Mutex::new(None),
                previews: preview::Previews::new(preview_dir),
                streams: stream::Streams::default(),
            });
            // WorkManager keeps the schedule across restarts; this only
            // repairs it, say after the app data was restored. On desktops
            // the app runs due tasks itself while it is open.
            commands::backup::reschedule(app.handle(), &app.state::<AppState>());
            commands::backup::start_scheduler(app.handle().clone());
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
            commands::accounts::list_accounts,
            commands::accounts::add_account,
            commands::accounts::add_crypt_folder,
            commands::accounts::oauth_apps,
            commands::accounts::oauth_sign_in,
            commands::accounts::oauth_cancel,
            commands::accounts::account_config,
            commands::accounts::update_account,
            commands::accounts::reorder_accounts,
            commands::accounts::remove_account,
            commands::accounts::export_accounts,
            commands::accounts::import_accounts,
            commands::cloud::list_dir,
            commands::cloud::cloud_mkdir,
            commands::cloud::cloud_rename,
            commands::cloud::cloud_delete,
            commands::cloud::cloud_share,
            commands::cloud::cloud_unshare,
            commands::cloud::preview_file,
            commands::cloud::preview_progress,
            commands::cloud::preview_cancel,
            commands::cloud::preview_open,
            commands::cloud::stream_open,
            commands::cloud::stream_play,
            commands::cloud::pdf_info,
            commands::cloud::pdf_page,
            commands::files::download,
            commands::files::upload,
            commands::files::local_conflicts,
            commands::files::list_local,
            commands::files::local_open,
            commands::files::local_mkdir,
            commands::files::local_rename,
            commands::files::local_delete,
            commands::files::list_transfers,
            commands::files::cancel_transfer,
            commands::files::cancel_all_transfers,
            commands::files::retry_transfer,
            commands::files::clear_transfers,
            commands::app::storage_access,
            commands::app::request_storage_access,
            commands::app::set_immersive,
            commands::app::check_update,
            commands::backup::backup_tasks,
            commands::backup::backup_save,
            commands::backup::backup_remove,
            commands::backup::backup_run,
            commands::backup::backup_cancel
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
