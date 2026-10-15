//! Preview of cloud files. A file is downloaded into the app cache once and
//! shown by the page through the asset protocol. The cache is cleared on
//! start, and starting a preview cancels the one still downloading.

use anyhow::{bail, Context, Result};
use belka_core::{Progress, StorageProvider};
use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tokio::sync::oneshot;

/// Text files are shown up to this size.
const TEXT_LIMIT: u64 = 1024 * 1024;

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
        if !local.exists() {
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

    /// Bytes downloaded and total of the preview in progress.
    pub fn progress(&self) -> (u64, u64) {
        self.current
            .lock()
            .unwrap()
            .as_ref()
            .map_or((0, 0), |c| (c.progress.done(), c.progress.total()))
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
