//! Android specifics, served by the Kotlin plugin `BelkaPlugin` in the
//! generated Android project: access to the shared storage and a foreground
//! service that keeps transfers going while the app is in the background.
//! On desktops the commands answer as if everything were allowed.

use crate::transfers::Transfers;
use std::sync::Arc;
use tauri::{AppHandle, Runtime};

#[cfg(mobile)]
mod imp {
    use super::*;
    use serde::Deserialize;
    use std::time::Duration;
    use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
    use tauri::Manager;

    pub struct Native<R: Runtime>(PluginHandle<R>);

    pub fn plugin<R: Runtime>() -> TauriPlugin<R> {
        Builder::new("belka")
            .setup(|app, api| {
                let handle =
                    api.register_android_plugin("io.github.tatarinovs.belka", "BelkaPlugin")?;
                app.manage(Native(handle));
                Ok(())
            })
            .build()
    }

    fn handle<R: Runtime>(app: &AppHandle<R>) -> Option<PluginHandle<R>> {
        app.try_state::<Native<R>>().map(|n| n.0.clone())
    }

    #[derive(Deserialize)]
    struct Granted {
        granted: bool,
    }

    pub fn storage_access<R: Runtime>(app: &AppHandle<R>) -> bool {
        handle(app)
            .and_then(|h| h.run_mobile_plugin::<Granted>("storageAccess", ()).ok())
            .is_some_and(|g| g.granted)
    }

    fn run<R: Runtime>(
        app: &AppHandle<R>,
        command: &str,
        payload: impl serde::Serialize,
    ) -> anyhow::Result<()> {
        let handle =
            handle(app).ok_or_else(|| anyhow::anyhow!("The Android plugin is not loaded"))?;
        handle
            .run_mobile_plugin::<()>(command, payload)
            .map_err(|err| anyhow::anyhow!("{err}"))
    }

    pub fn request_storage_access<R: Runtime>(app: &AppHandle<R>) -> anyhow::Result<()> {
        run(app, "requestStorageAccess", ())
    }

    /// Android has no "open with the default program": the user picks an app.
    pub fn open_file<R: Runtime>(app: &AppHandle<R>, path: &str) -> anyhow::Result<()> {
        #[derive(serde::Serialize)]
        struct Args<'a> {
            path: &'a str,
        }
        run(app, "openFile", Args { path })
    }

    /// Reports the transfer activity to the service once a second, and only
    /// when it changed. The page cannot do it: Android pauses its timers
    /// in the background.
    pub fn watch_transfers<R: Runtime>(app: AppHandle<R>, transfers: Arc<Transfers>) {
        std::thread::spawn(move || {
            let mut last = None;
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let activity = transfers.activity();
                if last.as_ref() == Some(&activity) {
                    continue;
                }
                if let Some(handle) = handle(&app) {
                    if let Err(err) = handle.run_mobile_plugin::<()>("transfers", &activity) {
                        tracing::warn!("Failed to update the transfer notification: {err}");
                    }
                }
                last = Some(activity);
            }
        });
    }
}

#[cfg(desktop)]
mod imp {
    use super::*;

    pub fn plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
        tauri::plugin::Builder::new("belka").build()
    }

    pub fn storage_access<R: Runtime>(_app: &AppHandle<R>) -> bool {
        true
    }

    pub fn request_storage_access<R: Runtime>(_app: &AppHandle<R>) -> anyhow::Result<()> {
        Ok(())
    }

    pub fn open_file<R: Runtime>(app: &AppHandle<R>, path: &str) -> anyhow::Result<()> {
        use tauri_plugin_opener::OpenerExt;
        app.opener()
            .open_path(path, None::<&str>)
            .map_err(|err| anyhow::anyhow!("{err}"))
    }

    pub fn watch_transfers<R: Runtime>(_app: AppHandle<R>, _transfers: Arc<Transfers>) {}
}

pub use imp::{open_file, plugin, request_storage_access, storage_access, watch_transfers};
