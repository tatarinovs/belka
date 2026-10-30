//! Preview of cloud files. A file is downloaded into the app cache once and
//! shown by the page through the asset protocol. The cache is cleared on
//! start and kept under [`CACHE_LIMIT`], and starting a preview cancels the
//! one still downloading.

use anyhow::{bail, Context, Result};
use belka_core::client::MAX_ATTEMPTS;
use belka_core::{Progress, StorageProvider};
use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;
use tokio::sync::oneshot;

/// Text files are shown up to this size.
const TEXT_LIMIT: u64 = 1024 * 1024;
/// The cache drops the previews used longest ago above this size. A few
/// videos from a cloud that cannot stream would otherwise fill the disk.
const CACHE_LIMIT: u64 = 2 * 1024 * 1024 * 1024;

pub struct Previews {
    dir: PathBuf,
    current: Mutex<Option<Current>>,
}

struct Current {
    progress: Progress,
    _cancel: oneshot::Sender<()>,
}

#[derive(Serialize)]
pub struct Preview {
    /// The cached file, for the asset protocol.
    path: String,
    /// Content of a text file, decoded from UTF-8 or Windows-1251.
    text: Option<String>,
    /// The text was cut at [`TEXT_LIMIT`].
    truncated: bool,
}

/// How far the preview in progress is.
#[derive(Serialize)]
pub struct PreviewProgress {
    done: u64,
    total: u64,
    /// Above 1 while network failures are retried.
    attempt: u32,
    max_attempts: u32,
}

/// A cloud file to preview; `size` and `modified` tell versions apart.
pub struct Item<'a> {
    pub account_id: &'a str,
    pub path: &'a str,
    pub size: u64,
    pub modified: Option<&'a str>,
}

impl Previews {
    pub fn new(dir: PathBuf) -> Self {
        let _ = std::fs::remove_dir_all(&dir);
        Self {
            dir,
            current: Mutex::new(None),
        }
    }

    fn cache_path(&self, item: &Item) -> PathBuf {
        let mut hasher = DefaultHasher::new();
        (item.account_id, item.path, item.size, item.modified).hash(&mut hasher);
        // The extension lets the asset protocol send the right content type.
        let ext: String = Path::new(item.path)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default()
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(10)
            .collect();
        self.dir.join(format!("{:016x}.{ext}", hasher.finish()))
    }

    pub async fn load(
        &self,
        provider: &dyn StorageProvider,
        item: Item<'_>,
        text: bool,
    ) -> Result<Preview> {
        let local = self.cache_path(&item);
        if local.exists() {
            touch(&local);
        } else {
            let (cancel, cancelled) = oneshot::channel();
            let progress = Progress::new();
            // Replacing the sender cancels the preview still downloading.
            *self.current.lock().unwrap() = Some(Current {
                progress: progress.clone(),
                _cancel: cancel,
            });
            std::fs::create_dir_all(&self.dir)
                .with_context(|| format!("Failed to create '{}'", self.dir.display()))?;
            tokio::select! {
                result = provider.download_file(item.path, &local, &progress) => { result?; }
                _ = cancelled => bail!("Preview cancelled"),
            }
            mark_from_internet(&local);
            let (dir, keep) = (self.dir.clone(), local.clone());
            tauri::async_runtime::spawn_blocking(move || trim(&dir, &keep, CACHE_LIMIT));
        }
        let (text, truncated) = if text {
            let (text, truncated) = read_text(&local)?;
            (Some(text), truncated)
        } else {
            (None, false)
        };
        Ok(Preview {
            path: local.to_string_lossy().into_owned(),
            text,
            truncated,
        })
    }

    pub fn progress(&self) -> PreviewProgress {
        let current = self.current.lock().unwrap();
        let progress = current.as_ref().map(|c| &c.progress);
        PreviewProgress {
            done: progress.map_or(0, Progress::done),
            total: progress.map_or(0, Progress::total),
            attempt: progress.map_or(1, Progress::attempt),
            max_attempts: MAX_ATTEMPTS,
        }
    }

    pub fn cancel(&self) {
        self.current.lock().unwrap().take();
    }

    /// Checks that `path` is a cached preview, so the page cannot open other files.
    pub fn cached(&self, path: &str) -> Result<PathBuf> {
        let path = PathBuf::from(path);
        if path.parent() != Some(self.dir.as_path()) || !path.is_file() {
            bail!("Not a preview file");
        }
        Ok(path)
    }
}

/// Extensions that Windows runs rather than shows when such a file is
/// opened: scripts and programs. A file from someone's shared folder must
/// not run because its preview was opened "in the program".
const RUNNABLE: &[&str] = &[
    "bat", "cmd", "com", "cpl", "exe", "hta", "jar", "js", "jse", "lnk", "msi", "msp", "pif",
    "ps1", "psm1", "reg", "scr", "url", "vb", "vbe", "vbs", "wsf", "wsh",
];

pub fn runs_when_opened(path: &Path) -> bool {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .is_some_and(|e| RUNNABLE.contains(&e.as_str()))
}

/// Marks a file as downloaded from the internet (Mark of the Web), so
/// Windows and Office treat it with the caution they give to downloads.
#[cfg(windows)]
fn mark_from_internet(path: &Path) {
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    if let Err(err) = std::fs::write(&stream, "[ZoneTransfer]\r\nZoneId=3\r\n") {
        tracing::debug!("Cannot mark '{}' as downloaded: {err}", path.display());
    }
}

#[cfg(not(windows))]
fn mark_from_internet(_path: &Path) {}

/// Marks a cached preview as just used, so [`trim`] keeps it longest.
fn touch(path: &Path) {
    let touched = std::fs::File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(SystemTime::now()));
    if let Err(err) = touched {
        tracing::debug!("Cannot mark '{}' as used: {err}", path.display());
    }
}

/// Deletes the files of `dir` used longest ago until the rest fit in
/// `limit`. `keep`, the preview being shown, stays even if it alone is
/// larger. Files that another program holds open stay too.
fn trim(dir: &Path, keep: &Path, limit: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let meta = entry.metadata().ok().filter(|m| m.is_file())?;
            Some((meta.modified().ok()?, meta.len(), entry.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
    files.sort();
    for (_, len, path) in files {
        if total <= limit {
            break;
        }
        if path == keep {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => total -= len,
            Err(err) => tracing::debug!("Cannot drop preview '{}': {err}", path.display()),
        }
    }
}

fn read_text(path: &Path) -> Result<(String, bool)> {
    let mut data = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(TEXT_LIMIT + 1).read_to_end(&mut data))
        .with_context(|| format!("Failed to read '{}'", path.display()))?;
    let truncated = data.len() as u64 > TEXT_LIMIT;
    data.truncate(TEXT_LIMIT as usize);
    let text = match std::str::from_utf8(&data) {
        Ok(text) => text.to_string(),
        // A cut can split a UTF-8 character at the very end.
        Err(err) if truncated && err.valid_up_to() + 4 > data.len() => {
            String::from_utf8_lossy(&data[..err.valid_up_to()]).into_owned()
        }
        // Old Russian text files are often in Windows-1251.
        Err(_) => encoding_rs::WINDOWS_1251.decode(&data).0.into_owned(),
    };
    Ok((text.trim_start_matches('\u{feff}').to_string(), truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf8_and_windows_1251() {
        let dir = tempfile::tempdir().unwrap();
        let utf8 = dir.path().join("a.txt");
        std::fs::write(&utf8, "\u{feff}Привет").unwrap();
        assert_eq!(read_text(&utf8).unwrap(), ("Привет".to_string(), false));

        let cp1251 = dir.path().join("b.txt");
        std::fs::write(&cp1251, [0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2]).unwrap();
        assert_eq!(read_text(&cp1251).unwrap().0, "Привет");
    }

    #[test]
    fn trim_drops_the_oldest_previews() {
        let tmp = tempfile::tempdir().unwrap();
        let file = |name: &str, age_secs: u64| {
            let path = tmp.path().join(name);
            std::fs::write(&path, [0u8; 100]).unwrap();
            let time = SystemTime::now() - std::time::Duration::from_secs(age_secs);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(time)
                .unwrap();
            path
        };
        let oldest = file("a.mp4", 300);
        let kept = file("b.mp4", 200);
        let newest = file("c.mp4", 100);
        trim(tmp.path(), &kept, 150);
        assert!(!oldest.exists());
        assert!(kept.exists(), "the preview being shown stays");
        assert!(!newest.exists());
    }

    #[test]
    fn scripts_are_runnable() {
        assert!(runs_when_opened(Path::new("a/x.BAT")));
        assert!(runs_when_opened(Path::new("x.js")));
        assert!(!runs_when_opened(Path::new("x.txt")));
        assert!(!runs_when_opened(Path::new("README")));
    }

    #[test]
    fn only_cached_files_are_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let previews = Previews::new(dir.path().join("preview"));
        std::fs::create_dir_all(&previews.dir).unwrap();
        let file = previews.dir.join("x.txt");
        std::fs::write(&file, "x").unwrap();
        assert!(previews.cached(&file.to_string_lossy()).is_ok());
        let outside = dir.path().join("y.txt");
        std::fs::write(&outside, "y").unwrap();
        assert!(previews.cached(&outside.to_string_lossy()).is_err());
        let sneaky = previews.dir.join("..").join("y.txt");
        assert!(previews.cached(&sneaky.to_string_lossy()).is_err());
    }
}
