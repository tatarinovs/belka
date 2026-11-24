//! Backup tasks. On Android WorkManager runs them on schedule (see
//! `native.rs`); on desktops the app runs due tasks itself while it is open,
//! and `belka task run` can do it from the system scheduler otherwise.

use crate::{native, AppState};
use belka_app::error::AppResult;
use belka_store::tasks::{self, Task, TaskView};
use tauri::{Manager, State};

#[tauri::command]
pub fn backup_tasks(state: State<'_, AppState>) -> AppResult<Vec<TaskView>> {
    state.backup_tasks()
}

/// Hands the schedule to the system on Android; nothing to do elsewhere.
pub fn reschedule(app: &tauri::AppHandle, state: &AppState) {
    match state.backup_list() {
        Ok(tasks) => {
            if let Err(err) = native::schedule_backups(app, &tasks) {
                tracing::warn!("Failed to schedule backups: {err:#}");
            }
        }
        Err(err) => tracing::warn!("{err:#}"),
    }
}

#[tauri::command]
pub fn backup_save(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    task: Task,
) -> AppResult<Task> {
    let task = state.backup_save(task)?;
    reschedule(&app, &state);
    Ok(task)
}

#[tauri::command]
pub fn backup_remove(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    state.backup_remove(&id)?;
    reschedule(&app, &state);
    Ok(())
}

#[tauri::command]
pub fn backup_run(app: tauri::AppHandle, id: String) -> AppResult<()> {
    if tasks::is_running(&id) {
        return Ok(());
    }
    let task = app.state::<AppState>().backup_task(&id)?;
    if !native::run_backup(&app, &task)? {
        spawn_run(app, id);
    }
    Ok(())
}

#[tauri::command]
pub fn backup_cancel(state: State<'_, AppState>, id: String) {
    state.backup_cancel(&id);
}

/// Runs a task in the app's own runtime.
fn spawn_run(app: tauri::AppHandle, id: String) {
    tauri::async_runtime::spawn(async move {
        #[cfg(target_os = "windows")]
        if crate::tray::exiting(&app) {
            return;
        }
        let state = app.state::<AppState>();
        if let Err(err) = state.run_backup_task(&id).await {
            tracing::warn!("{err:#}");
        }
    });
}

/// Desktops: checks every minute for tasks whose time has come.
#[cfg(desktop)]
pub fn start_scheduler(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Give the window time to appear before the first run.
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        loop {
            #[cfg(target_os = "windows")]
            if crate::tray::exiting(&app) {
                return;
            }
            let now = chrono::Utc::now();
            let due: Vec<String> = app
                .state::<AppState>()
                .backup_list()
                .unwrap_or_default()
                .into_iter()
                .filter(|t| t.is_due(now) && !tasks::is_running(&t.id))
                .map(|t| t.id)
                .collect();
            for id in due {
                spawn_run(app.clone(), id);
            }
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}

/// Android: WorkManager runs the tasks.
#[cfg(mobile)]
pub fn start_scheduler(_app: tauri::AppHandle) {}
