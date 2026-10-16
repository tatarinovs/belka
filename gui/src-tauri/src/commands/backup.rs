//! Backup tasks. On Android WorkManager runs them on schedule (see
//! `native.rs`); on desktops the app runs due tasks itself while it is open,
//! and `belka task run` can do it from the system scheduler otherwise.

use crate::error::{translate, CmdResult};
use crate::{native, AppState};
use belka_store::tasks::{self, Task, TaskView};
use tauri::{Manager, State};

/// The tasks with their history; errors are stored as the core wrote them
/// and translated for the page here.
#[tauri::command]
pub fn backup_tasks(state: State<'_, AppState>) -> CmdResult<Vec<TaskView>> {
    let mut views = state.tasks.views()?;
    for run in views.iter_mut().flat_map(|v| v.task.history.iter_mut()) {
        run.error = run.error.as_deref().map(translate);
        for (_, reason) in &mut run.failures {
            *reason = translate(reason);
        }
    }
    Ok(views)
}

/// Hands the schedule to the system on Android; nothing to do elsewhere.
pub fn reschedule(app: &tauri::AppHandle, state: &AppState) {
    match state.tasks.list() {
        Ok(tasks) => {
            if let Err(err) = native::schedule_backups(app, &tasks) {
                tracing::warn!("Failed to schedule backups: {err:#}");
            }
        }
        Err(err) => tracing::warn!("{err:#}"),
    }
}

/// Adds a task (an empty id) or changes one.
#[tauri::command]
pub fn backup_save(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    task: Task,
) -> CmdResult<Task> {
    state.account(&task.account_id)?;
    if tasks::is_running(&task.id) {
        // A run must not record its index for a target the task no longer has.
        if let Ok(old) = state.tasks.task(&task.id) {
            if (&old.local, &old.account_id, &old.remote)
                != (&task.local, &task.account_id, &task.remote)
            {
                tasks::cancel(&task.id);
            }
        }
    }
    let task = state.tasks.save(task)?;
    reschedule(&app, &state);
    Ok(task)
}

#[tauri::command]
pub fn backup_remove(app: tauri::AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    tasks::cancel(&id);
    state.tasks.remove(&id)?;
    reschedule(&app, &state);
    Ok(())
}

/// Starts a task now. On Android the run goes through WorkManager, which
/// keeps it going in the background under a notification.
#[tauri::command]
pub fn backup_run(app: tauri::AppHandle, id: String) -> CmdResult<()> {
    if tasks::is_running(&id) {
        return Ok(());
    }
    let task = app.state::<AppState>().tasks.task(&id)?;
    if !native::run_backup(&app, &task)? {
        spawn_run(app, id);
    }
    Ok(())
}

#[tauri::command]
pub fn backup_cancel(id: String) {
    tasks::cancel(&id);
}

/// Runs a task in the app's own runtime.
fn spawn_run(app: tauri::AppHandle, id: String) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let provider = |account: &str| state.provider(account);
        if let Err(err) = tasks::run(&state.tasks, &id, &provider).await {
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
            let now = chrono::Utc::now();
            let due: Vec<String> = app
                .state::<AppState>()
                .tasks
                .list()
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
