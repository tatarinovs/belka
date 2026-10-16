//! Android specifics, served by the Kotlin plugin `BelkaPlugin` in the
//! generated Android project: access to the shared storage and a foreground
//! service that keeps transfers going while the app is in the background,
//! and backups scheduled with WorkManager, which call back into Rust
//! through JNI ([`jni_api`]).
//! On desktops the commands answer as if everything were allowed.

use belka_store::tasks::Task;
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

    /// A task as `BelkaPlugin.scheduleBackups` reads it.
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Scheduled<'a> {
        id: &'a str,
        /// For the notification.
        name: &'a str,
        interval_hours: u32,
        wifi_only: bool,
        charging_only: bool,
    }

    /// Replaces the WorkManager schedule with the enabled tasks.
    pub fn schedule_backups<R: Runtime>(app: &AppHandle<R>, tasks: &[Task]) -> anyhow::Result<()> {
        #[derive(serde::Serialize)]
        struct Args<'a> {
            tasks: Vec<Scheduled<'a>>,
        }
        let tasks = tasks
            .iter()
            .filter(|t| t.enabled)
            .map(|t| Scheduled {
                id: &t.id,
                name: &t.name,
                interval_hours: t.interval_hours,
                wifi_only: t.wifi_only,
                charging_only: t.charging_only,
            })
            .collect();
        run(app, "scheduleBackups", Args { tasks })
    }

    /// Starts a task now as WorkManager work; true once it is queued.
    pub fn run_backup<R: Runtime>(app: &AppHandle<R>, task: &Task) -> anyhow::Result<bool> {
        #[derive(serde::Serialize)]
        struct Args<'a> {
            id: &'a str,
            name: &'a str,
        }
        let (id, name) = (&task.id, &task.name);
        run(app, "runBackup", Args { id, name })?;
        Ok(true)
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

    pub fn schedule_backups<R: Runtime>(_app: &AppHandle<R>, _tasks: &[Task]) -> anyhow::Result<()> {
        Ok(())
    }

    /// Desktops run tasks in the app itself.
    pub fn run_backup<R: Runtime>(_app: &AppHandle<R>, _task: &Task) -> anyhow::Result<bool> {
        Ok(false)
    }
}

pub use imp::{
    open_file, plugin, request_storage_access, run_backup, schedule_backups, storage_access,
    watch_transfers,
};

/// Entry points of `BackupNative` in the Android project. `BackupWorker`
/// loads the library itself, so they work while the page is not running.
#[cfg(target_os = "android")]
mod jni_api {
    use crate::error::translate;
    use belka_store::accounts::AccountStore;
    use belka_store::secrets::Secrets;
    use belka_store::tasks::{self, TaskStore};
    use jni::objects::{JClass, JString};
    use jni::sys::jstring;
    use jni::JNIEnv;
    use std::path::Path;

    fn string(env: &mut JNIEnv, value: &JString) -> String {
        env.get_string(value).map(Into::into).unwrap_or_default()
    }

    fn to_java(env: &mut JNIEnv, value: &str) -> jstring {
        env.new_string(value)
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }

    /// Runs task `id` with the settings in `dir` to the end; returns the
    /// result as JSON with `freed_paths`, the local files deleted to free
    /// space; `{"error": …}` if it could not run at all, or `{"busy": true}`
    /// if it is running already. Errors are translated for the notification.
    #[no_mangle]
    pub extern "system" fn Java_io_github_tatarinovs_belka_BackupNative_run<'l>(
        mut env: JNIEnv<'l>,
        _class: JClass<'l>,
        dir: JString<'l>,
        id: JString<'l>,
    ) -> jstring {
        let (dir, id) = (string(&mut env, &dir), string(&mut env, &id));
        // Started by the page while the schedule fired, or the other way round.
        if tasks::is_running(&id) {
            return to_java(&mut env, r#"{"busy":true}"#);
        }
        let dir = Path::new(&dir);
        let accounts = AccountStore::new(dir, Secrets::File(dir.join("secrets.json")));
        let json = match tasks::run_blocking(&TaskStore::new(dir), &accounts, &id) {
            Ok(mut run) => {
                run.result.error = run.result.error.as_deref().map(translate);
                let mut value = serde_json::to_value(&run.result).unwrap_or_default();
                value["freed_paths"] = serde_json::json!(run.freed);
                value.to_string()
            }
            Err(err) => serde_json::json!({ "error": translate(&format!("{err:#}")) }).to_string(),
        };
        to_java(&mut env, &json)
    }

    /// The progress of a running task as JSON, or an empty string.
    #[no_mangle]
    pub extern "system" fn Java_io_github_tatarinovs_belka_BackupNative_progress<'l>(
        mut env: JNIEnv<'l>,
        _class: JClass<'l>,
        id: JString<'l>,
    ) -> jstring {
        let id = string(&mut env, &id);
        let json = tasks::live(&id)
            .and_then(|live| serde_json::to_string(&live).ok())
            .unwrap_or_default();
        to_java(&mut env, &json)
    }

    #[no_mangle]
    pub extern "system" fn Java_io_github_tatarinovs_belka_BackupNative_cancel<'l>(
        mut env: JNIEnv<'l>,
        _class: JClass<'l>,
        id: JString<'l>,
    ) {
        let id = string(&mut env, &id);
        tasks::cancel(&id);
    }
}
