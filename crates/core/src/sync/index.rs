//! What is known to be backed up: relative path -> size and modification
//! time. A file with the same size and time as recorded is not read and not
//! compared with the cloud, so a run over thousands of unchanged files is
//! a folder walk and nothing more.

use super::{LocalFile, LocalTree};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub struct Index {
    file: PathBuf,
    data: Data,
}

#[derive(Default, Serialize, Deserialize)]
struct Data {
    /// Where the files were backed up to. An index made for another target
    /// says nothing about this one and is dropped on load.
    #[serde(default)]
    target: String,
    files: HashMap<String, (u64, i64)>,
}

impl Index {
    /// Loads the index kept in `file` for `target`, a string that names
    /// the source and the destination; empty if there is none yet, it is
    /// damaged or it was made for another target.
    pub fn load(file: impl Into<PathBuf>, target: &str) -> Self {
        let file = file.into();
        let data = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Data>(&bytes).ok())
            .filter(|data| data.target == target)
            .unwrap_or_else(|| Data {
                target: target.to_string(),
                files: HashMap::new(),
            });
        Self { file, data }
    }

    /// A file in `dir` named after `target`, for callers that keep one index
    /// per source and destination pair.
    pub fn file_for(dir: &Path, target: &str) -> PathBuf {
        use md5::{Digest, Md5};
        dir.join(format!("{}.json", hex::encode(Md5::digest(target.as_bytes()))))
    }

    pub fn path(&self) -> &Path {
        &self.file
    }

    pub fn len(&self) -> usize {
        self.data.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.files.is_empty()
    }

    /// The file is backed up as it is now.
    pub fn is_current(&self, file: &LocalFile) -> bool {
        self.data.files.get(&file.relative) == Some(&file.stamp())
    }

    /// The file was backed up at some point, maybe in another version.
    pub fn contains(&self, relative: &str) -> bool {
        self.data.files.contains_key(relative)
    }

    pub fn record(&mut self, file: &LocalFile) {
        self.data
            .files
            .insert(file.relative.clone(), file.stamp());
    }

    pub fn remove(&mut self, relative: &str) {
        self.data.files.remove(relative);
    }

    /// Forgets files that are no longer in the tree, so the index does not
    /// grow forever. Only for a tree scanned without errors: an unreadable
    /// folder must not look like deleted files.
    pub fn retain_scanned(&mut self, tree: &LocalTree) {
        if !tree.errors.is_empty() {
            return;
        }
        let present: HashSet<&str> = tree.files.iter().map(|f| f.relative.as_str()).collect();
        self.data.files.retain(|path, _| present.contains(path.as_str()));
    }

    /// Writes through a temporary file, so a crash never leaves half an index.
    pub fn save(&self) -> Result<()> {
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("Failed to create '{}'", dir.display()))?;
        }
        let tmp = self.file.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec(&self.data)?)
            .with_context(|| format!("Failed to write '{}'", tmp.display()))?;
        std::fs::rename(&tmp, &self.file)
            .with_context(|| format!("Failed to write '{}'", self.file.display()))
    }

    /// Deletes the index file: the next run compares every file again.
    pub fn forget(file: &Path) {
        let _ = std::fs::remove_file(file);
    }
}
