//! The app itself: storage access and the full screen on Android, updates.

use crate::native;
use belka_app::error::AppResult;
use belka_app::Update;

/// The frontend is drawn. On Windows a close made during startup stays hidden.
#[tauri::command]
pub fn app_ready(app: tauri::AppHandle) -> AppResult<()> {
    #[cfg(target_os = "windows")]
    crate::tray::show_when_ready(&app);
    #[cfg(not(target_os = "windows"))]
    {
        use tauri::Manager;
        if let Some(window) = app.get_webview_window("main") {
            window.show().map_err(anyhow::Error::from)?;
        }
    }
    Ok(())
}

/// Whether the app may browse the shared storage; always on desktops.
#[tauri::command]
pub fn storage_access(app: tauri::AppHandle) -> bool {
    native::storage_access(&app)
}

/// Opens the system page where the user grants access to all files.
#[tauri::command]
pub fn request_storage_access(app: tauri::AppHandle) -> AppResult<()> {
    Ok(native::request_storage_access(&app)?)
}

/// Android: hides the system bars while a video fills the screen, or shows
/// them again.
#[tauri::command]
pub fn set_immersive(app: tauri::AppHandle, on: bool) -> AppResult<()> {
    Ok(native::immersive(&app, on)?)
}

/// The latest release on GitHub if it is newer than this build.
#[tauri::command]
pub async fn check_update() -> AppResult<Option<Update>> {
    belka_app::state::check_update(env!("CARGO_PKG_VERSION")).await
}
