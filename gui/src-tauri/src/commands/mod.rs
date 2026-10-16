//! The commands the page invokes, grouped by what they work on.

pub mod accounts;
pub mod app;
pub mod backup;
pub mod cloud;
pub mod files;

use tauri_plugin_fs::FilePath;

/// `path` is what the file dialog returned: a path, or on Android a
/// `content://` URI.
fn file_path(path: &str) -> FilePath {
    path.parse().unwrap_or_else(|never| match never {})
}
