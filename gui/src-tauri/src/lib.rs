mod commands;
mod error;
mod local;
mod native;
mod preview;
mod settings;
mod stream;
mod transfers;
#[cfg(target_os = "windows")]
mod tray;
mod ui_state;
#[cfg(target_os = "windows")]
mod webview_cache;

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

    #[allow(unused_mut)]
    let mut context = tauri::generate_context!();
    #[cfg(target_os = "windows")]
    let webview_cache = Arc::new(Mutex::new(None::<tempfile::TempDir>));
    #[cfg(target_os = "windows")]
    let webview_cache_setup = webview_cache.clone();
    #[cfg(target_os = "windows")]
    {
        // Build the window in setup with a disposable InPrivate profile.
        context.config_mut().app.windows[0].create = false;
    }
    let builder = tauri::Builder::default();
    #[cfg(target_os = "windows")]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _, _| {
        tray::show(app);
    }));
    let builder = builder
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
    let app = builder
        .on_window_event(|_window, _event| {
            #[cfg(target_os = "windows")]
            if let tauri::WindowEvent::CloseRequested { api, .. } = _event {
                tray::close(_window, api);
            }
        })
        .setup(move |app| {
            let config_dir = app.path().app_config_dir()?;
            let ui_state = ui_state::UiState::new(&config_dir)?;
            app.manage(ui_state);
            let preview_dir = app.path().app_cache_dir()?.join("preview");
            // On Android the file is in the app's private folder; desktops
            // use the keyring, which the CLI shares.
            let secrets = Secrets::system_or(app.path().app_data_dir()?.join("secrets.json"));
            let transfers = Transfers::new(app.path().app_cache_dir()?.join("copy"));
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
            #[cfg(target_os = "windows")]
            {
                // Allocate only after the single-instance check, so repeated
                // launches do not leave unused temporary directories behind.
                let cache = tempfile::Builder::new()
                    .prefix("belka-webview-")
                    .tempdir()?;
                webview_cache::create_window(app, cache.path())?;
                *webview_cache_setup.lock().unwrap() = Some(cache);
            }
            #[cfg(target_os = "windows")]
            if let Err(err) = tray::install(app.handle()) {
                // A failed tray setup must leave a normal, closable window.
                tracing::warn!("Cannot create the tray icon: {err:#}");
            }
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
                        #[cfg(target_os = "windows")]
                        tray::show_when_ready(window.app_handle());
                        #[cfg(not(target_os = "windows"))]
                        let _ = window.show();
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            settings::get_desktop_settings,
            settings::set_close_to_tray,
            settings::set_autostart,
            ui_state::load_ui_state,
            ui_state::remember_ui_state,
            commands::app::app_ready,
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
            commands::cloud::cloud_quota,
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
            commands::files::cloud_copy,
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
            commands::files::retry_failed_transfers,
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
        .build(context)
        .expect("error while building tauri application");
    #[cfg(target_os = "windows")]
    {
        // App::run exits the process directly and skips destructors. Returning
        // from the event loop lets WebView2 close before removing its profile.
        let mut closing_webviews = false;
        let code = app.run_return(move |app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                let windows = app.webview_windows();
                if !closing_webviews && !windows.is_empty() {
                    // Dropping the event loop alone leaves COM handles alive.
                    // Destroy WebViews while it is still processing messages;
                    // destroying the last window requests exit again.
                    closing_webviews = true;
                    api.prevent_exit();
                    for window in windows.into_values() {
                        if let Err(err) = window.destroy() {
                            tracing::warn!("Cannot close browser window: {err}");
                            app.exit(1);
                        }
                    }
                }
            }
        });
        if let Some(cache) = webview_cache.lock().unwrap().take() {
            webview_cache::cleanup(cache.path());
            drop(cache);
        }
        std::process::exit(code);
    }
    #[cfg(not(target_os = "windows"))]
    app.run(|_, _| {});
}
