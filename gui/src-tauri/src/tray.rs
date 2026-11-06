//! Windows window lifetime and graceful shutdown.
use crate::AppState;
use anyhow::Result;
use belka_store::tasks;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

#[derive(Default)]
struct TrayState {
    hidden_by_user: AtomicBool,
    exit_prompt: AtomicBool,
    exiting: AtomicBool,
}

pub fn exiting(app: &AppHandle) -> bool {
    app.try_state::<TrayState>()
        .is_some_and(|s| s.exiting.load(Ordering::Acquire))
}

pub fn show(app: &AppHandle) {
    if exiting(app) {
        return;
    }
    if let Some(state) = app.try_state::<TrayState>() {
        state.hidden_by_user.store(false, Ordering::Release);
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Late frontend readiness and the startup fallback must not undo a close.
pub fn show_when_ready(app: &AppHandle) {
    if app
        .try_state::<TrayState>()
        .is_some_and(|s| s.hidden_by_user.load(Ordering::Acquire))
    {
        return;
    }
    show(app);
}

pub fn close(window: &tauri::Window, api: &tauri::CloseRequestApi) {
    if window.label() != "main" || window.app_handle().tray_by_id("belka").is_none() {
        return;
    }
    api.prevent_close();
    if !window
        .app_handle()
        .state::<crate::ui_state::UiState>()
        .close_to_tray()
    {
        request_exit(window.app_handle());
        return;
    }
    match window.hide() {
        Ok(()) => {
            if let Some(state) = window.app_handle().try_state::<TrayState>() {
                state.hidden_by_user.store(true, Ordering::Release);
            }
        }
        Err(err) => tracing::warn!("Cannot hide the window: {err}"),
    }
}

fn finish_exit(app: AppHandle) {
    let state = app.state::<TrayState>();
    if state.exiting.swap(true, Ordering::AcqRel) {
        return;
    }
    app.state::<AppState>().transfers.shutdown();
    for id in tasks::running_ids() {
        tasks::cancel(&id);
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    tauri::async_runtime::spawn(async move {
        // Give cancelled backup runs time to write their indexes and history.
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let running = tasks::running_ids();
            if running.is_empty() {
                app.exit(0);
                return;
            }
            for id in running {
                tasks::cancel(&id);
            }
        }
        tracing::warn!("Backup shutdown timed out");
        app.exit(0);
    });
}

fn request_exit(app: &AppHandle) {
    let state = app.state::<TrayState>();
    if state.exit_prompt.swap(true, Ordering::AcqRel) || exiting(app) {
        return;
    }
    let transfers = app.state::<AppState>().transfers.active_count();
    let backups = tasks::running_ids().len();
    if transfers == 0 && backups == 0 {
        finish_exit(app.clone());
        return;
    }
    show(app);
    let app_copy = app.clone();
    let mut dialog = app.dialog().message(format!(
        "Незавершённых передач: {transfers}. Задач автобэкапа в работе: {backups}.\n\nОстановить их и выйти из Belka?"
    )).title("Выход из Belka").kind(MessageDialogKind::Warning)
      .buttons(MessageDialogButtons::OkCancelCustom("Остановить и выйти".into(), "Отмена".into()));
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.parent(&window);
    }
    dialog.show(move |confirmed| {
        if confirmed {
            finish_exit(app_copy);
        } else {
            app_copy
                .state::<TrayState>()
                .exit_prompt
                .store(false, Ordering::Release);
        }
    });
}

pub fn install(app: &AppHandle) -> Result<()> {
    let open = MenuItem::with_id(app, "open", "Открыть Belka", true, None::<&str>)?;
    let status = MenuItem::with_id(app, "status", "Belka работает в фоне", false, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let exit = MenuItem::with_id(app, "exit", "Выход", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &status, &separator, &exit])?;
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Tray icon is missing"))?;
    app.manage(TrayState::default());
    TrayIconBuilder::with_id("belka")
        .icon(icon)
        .tooltip("Belka — работает в фоне")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show(app),
            "exit" => request_exit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show(tray.app_handle());
            }
        })
        .build(app)?;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut last = String::new();
        loop {
            let transfers = app.state::<AppState>().transfers.active_count();
            let backups = tasks::running_ids().len();
            let text = if exiting(&app) {
                "Завершение работы…".into()
            } else if transfers + backups > 0 {
                format!("Передач: {transfers} · бэкапов: {backups}")
            } else {
                "Belka работает в фоне".into()
            };
            if text != last {
                let _ = status.set_text(&text);
                if let Some(tray) = app.tray_by_id("belka") {
                    let _ = tray.set_tooltip(Some(&text));
                }
                last = text;
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });
    Ok(())
}
