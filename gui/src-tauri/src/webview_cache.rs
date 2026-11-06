//! Windows WebView2 needs a disk profile even in InPrivate mode. Keep it
//! in a temporary directory and remove it after the event loop has stopped.
use std::path::Path;
use std::time::Duration;
pub fn create_window(app: &tauri::App, directory: &Path) -> tauri::Result<()> {
    let config = &app.config().app.windows[0];
    tauri::WebviewWindowBuilder::from_config(app.handle(), config)?
        .incognito(true)
        .data_directory(directory.to_owned())
        .build()?;
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
