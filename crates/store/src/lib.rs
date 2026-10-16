//! What the GUI keeps between runs and the CLI can use too: cloud accounts
//! with their credentials ([`accounts`]), where the credentials live
//! ([`secrets`]) and backup tasks ([`tasks`]).

pub mod accounts;
pub mod secrets;
pub mod tasks;

use anyhow::{Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// The app id: the GUI keeps its settings in a folder of this name.
pub const APP_ID: &str = "io.github.tatarinovs.belka";

/// The folder where the desktop GUI keeps accounts and tasks, as Tauri
/// resolves its config folder; `BELKA_CONFIG_DIR` overrides it.
pub fn default_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("BELKA_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.map(|b| b.join(APP_ID))
}

/// Writes JSON through a temporary file so a crash never leaves a
/// truncated file.
pub(crate) fn write_json<T: Serialize + ?Sized>(file: &Path, value: &T) -> Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("Failed to create '{}'", dir.display()))?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)
        .with_context(|| format!("Failed to write '{}'", tmp.display()))?;
    std::fs::rename(&tmp, file).with_context(|| format!("Failed to write '{}'", file.display()))
}
