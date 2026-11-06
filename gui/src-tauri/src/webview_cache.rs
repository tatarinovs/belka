//! Windows WebView2 needs a disk profile even in InPrivate mode. Keep it
//! in a temporary directory and remove it after the event loop has stopped.
use std::path::Path;
use std::time::Duration;
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CLOAK};

/// Keep DWM composing the window without presenting it to the user.
pub fn set_cloaked(window: &tauri::WebviewWindow, cloaked: bool) -> anyhow::Result<()> {
    let hwnd = window.hwnd()?;
    let value = i32::from(cloaked);
    // SAFETY: hwnd belongs to this live window; value is a BOOL-sized value
    // whose pointer remains valid for the duration of this synchronous call.
    let result = unsafe {
        DwmSetWindowAttribute(
            hwnd.0,
            DWMWA_CLOAK as u32,
            (&value as *const i32).cast(),
            std::mem::size_of_val(&value) as u32,
        )
    };
    anyhow::ensure!(result >= 0, "Cannot cloak window: HRESULT {result:#x}");
    Ok(())
}

pub fn create_window(app: &tauri::App, directory: &Path) -> tauri::Result<()> {
    let config = &app.config().app.windows[0];
    let window = tauri::WebviewWindowBuilder::from_config(app.handle(), config)?
        .incognito(true)
        .data_directory(directory.to_owned())
        .build()?;
    // Windows defers maximizing a hidden window until show(). Let it reach
    // its restored size and let WebView2 paint that entire area while cloaked.
    // app_ready (or an explicit tray Open) will reveal the finished window.
    match set_cloaked(&window, true) {
        Ok(()) => window.show()?,
        Err(err) => tracing::warn!("Cannot prepare the window offscreen: {err:#}"),
    }
    Ok(())
}

pub fn cleanup(directory: &Path) {
    // Browser subprocesses may release their file handles shortly after the
    // host's event loop ends. Retry without terminating shared runtime processes.
    for _ in 0..50 {
        match std::fs::remove_dir_all(directory) {
            Ok(()) => return,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
    tracing::warn!(
        "Cannot remove temporary browser cache: {}",
        directory.display()
    );
}
