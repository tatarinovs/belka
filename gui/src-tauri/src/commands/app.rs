//! The app itself: storage access on Android and updates.

use crate::error::CmdResult;
use crate::native;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Whether the app may browse the shared storage; always on desktops.
#[tauri::command]
pub fn storage_access(app: tauri::AppHandle) -> bool {
    native::storage_access(&app)
}

/// Opens the system page where the user grants access to all files.
#[tauri::command]
pub fn request_storage_access(app: tauri::AppHandle) -> CmdResult<()> {
    Ok(native::request_storage_access(&app)?)
}

#[derive(Serialize)]
pub struct Update {
    version: String,
    /// The release page.
    page: String,
    /// The Android package of the release, if it has one.
    apk: Option<String>,
}

/// The latest release on GitHub if it is newer than this build.
#[tauri::command]
pub async fn check_update() -> CmdResult<Option<Update>> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
        html_url: String,
        #[serde(default)]
        draft: bool,
        #[serde(default)]
        prerelease: bool,
        #[serde(default)]
        assets: Vec<Asset>,
    }
    #[derive(Deserialize)]
    struct Asset {
        name: String,
        browser_download_url: String,
    }
    async {
        let release: Release = reqwest::Client::builder()
            .user_agent(concat!("belka-gui/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(20))
            .build()?
            .get("https://api.github.com/repos/tatarinovs/belka/releases/latest")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let version = release.tag_name.trim_start_matches('v').to_string();
        if release.draft || release.prerelease || !is_newer(&version, env!("CARGO_PKG_VERSION")) {
            return Ok(None);
        }
        // A universal package or the one for 64-bit ARM phones.
        let apk = release
            .assets
            .iter()
            .filter(|a| a.name.ends_with(".apk"))
            .min_by_key(|a| !(a.name.contains("arm64") || a.name.contains("universal")))
            .map(|a| a.browser_download_url.clone());
        Ok(Some(Update {
            version,
            page: release.html_url,
            apk,
        }))
    }
    .await
    .map_err(|err: anyhow::Error| err.context("Failed to check for updates").into())
}

/// Compares dotted versions number by number; a suffix like "-beta" is ignored.
fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.split(['-', '+'])
            .next()
            .unwrap_or_default()
            .split('.')
            .map(|n| n.parse().unwrap_or(0))
            .collect()
    };
    parse(candidate) > parse(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_versions() {
        assert!(is_newer("1.2.0", "1.1.0"));
        assert!(is_newer("1.10.0", "1.9.3"));
        assert!(is_newer("2.0", "1.9.9"));
        assert!(!is_newer("1.1.0", "1.1.0"));
        assert!(!is_newer("1.0.9", "1.1.0"));
        assert!(!is_newer("1.1.0-beta", "1.1.0"));
    }
}
