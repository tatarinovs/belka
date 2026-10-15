//! Local file system browsing for the right-hand pane. The empty path is the
//! virtual "This PC" root: well-known folders followed by the drives.

use anyhow::{bail, Context, Result};
use belka_core::storage::check_name;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::path::{Component, Path, PathBuf};

#[derive(Serialize)]
pub struct LocalEntry {
    name: String,
    path: String,
    is_dir: bool,
    size: u64,
    modified: Option<DateTime<Utc>>,
    /// Icon hint for entries of the virtual root: "drive" or "place".
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<&'static str>,
}

#[derive(Serialize)]
pub struct Crumb {
    name: String,
    path: String,
}

#[derive(Serialize)]
pub struct LocalListing {
    path: String,
    /// `None` at the virtual root; `""` means "go up to the root".
    parent: Option<String>,
    crumbs: Vec<Crumb>,
    entries: Vec<LocalEntry>,
}

/// A well-known folder shown at the virtual root.
pub struct Place {
    pub name: &'static str,
    pub path: PathBuf,
}

pub fn list(path: &str, places: &[Place]) -> Result<LocalListing> {
    if path.is_empty() {
        return Ok(LocalListing {
            path: String::new(),
            parent: None,
            crumbs: Vec::new(),
            entries: root_entries(places),
        });
    }

    let dir = PathBuf::from(path);
    let mut entries = Vec::new();
    let read = std::fs::read_dir(&dir)
        .with_context(|| format!("Cannot open folder '{}'", dir.display()))?;
    for entry in read.flatten() {
        // Entries that cannot be inspected (no access, broken links) are skipped.
        let Ok(meta) = std::fs::metadata(entry.path()) else {
            continue;
        };
        if is_hidden(&entry.file_name().to_string_lossy(), &meta) {
            continue;
        }
        entries.push(LocalEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: entry.path().display().to_string(),
            is_dir: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
            modified: meta.modified().ok().map(DateTime::<Utc>::from),
            icon: None,
        });
    }

    Ok(LocalListing {
        path: dir.display().to_string(),
        parent: Some(
            dir.parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        ),
        crumbs: crumbs(&dir),
        entries,
    })
}

/// Rejects names the local file system cannot store.
fn local_name(name: &str) -> Result<&str> {
    let name = check_name(name)?;
    if cfg!(windows) && name.contains(['<', '>', ':', '"', '|', '?', '*']) {
        bail!("A name cannot contain any of < > : \" | ? *");
    }
    Ok(name)
}

/// Creates `name` in `parent` and returns its path.
pub fn mkdir(parent: &str, name: &str) -> Result<String> {
    if parent.is_empty() {
        bail!("Open a folder first");
    }
    let path = Path::new(parent).join(local_name(name)?);
    if path.exists() {
        bail!("'{}' already exists", name.trim());
    }
    std::fs::create_dir(&path).with_context(|| format!("Failed to create '{}'", path.display()))?;
    Ok(path.display().to_string())
}

pub fn rename(path: &str, new_name: &str) -> Result<String> {
    let from = PathBuf::from(path);
    let parent = from.parent().context("A drive cannot be renamed")?;
    let to = parent.join(local_name(new_name)?);
    // A case-only rename ("a.txt" → "A.txt") targets the same file.
    let same_file = from.to_string_lossy().to_lowercase() == to.to_string_lossy().to_lowercase();
    if to.exists() && !same_file {
        bail!("'{}' already exists", new_name.trim());
    }
    std::fs::rename(&from, &to)
        .with_context(|| format!("Failed to rename '{}'", from.display()))?;
    Ok(to.display().to_string())
}

/// Moves files and folders to the Recycle Bin.
pub fn delete(paths: &[String]) -> Result<()> {
    if paths.iter().any(|p| Path::new(p).parent().is_none()) {
        bail!("A drive cannot be deleted");
    }
    trash::delete_all(paths).context("Failed to move to the Recycle Bin")
}

fn root_entries(places: &[Place]) -> Vec<LocalEntry> {
    let places = places
        .iter()
        .filter(|p| p.path.is_dir())
        .map(|p| LocalEntry {
            name: p.name.to_string(),
            path: p.path.display().to_string(),
            is_dir: true,
            size: 0,
            modified: None,
            icon: Some("place"),
        });
    let drives = drives().into_iter().map(|path| LocalEntry {
        name: path
            .display()
            .to_string()
            .trim_end_matches('\\')
            .to_string(),
        path: path.display().to_string(),
        is_dir: true,
        size: 0,
        modified: None,
        icon: Some("drive"),
    });
    places.chain(drives).collect()
}

#[cfg(windows)]
fn drives() -> Vec<PathBuf> {
    extern "system" {
        fn GetLogicalDrives() -> u32;
    }
    // SAFETY: a parameterless Win32 call returning a bit mask of drive letters.
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| PathBuf::from(format!("{}:\\", (b'A' + i) as char)))
        .collect()
}

#[cfg(not(windows))]
fn drives() -> Vec<PathBuf> {
    vec![PathBuf::from("/")]
}

#[cfg(windows)]
fn is_hidden(_name: &str, meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    meta.file_attributes() & (HIDDEN | SYSTEM) != 0
}

#[cfg(not(windows))]
fn is_hidden(name: &str, _meta: &std::fs::Metadata) -> bool {
    name.starts_with('.')
}

/// `C:\Users\me` → `C:`, `Users`, `me`, each with the path it leads to.
fn crumbs(dir: &Path) -> Vec<Crumb> {
    let mut crumbs: Vec<Crumb> = Vec::new();
    let mut acc = PathBuf::new();
    for component in dir.components() {
        acc.push(component.as_os_str());
        let name = match component {
            Component::Prefix(prefix) => prefix.as_os_str().to_string_lossy().into_owned(),
            Component::RootDir => {
                // The root after a drive prefix belongs to the drive crumb.
                if let Some(drive) = crumbs.last_mut() {
                    drive.path = acc.display().to_string();
                    continue;
                }
                "/".to_string()
            }
            Component::Normal(name) => name.to_string_lossy().into_owned(),
            Component::CurDir | Component::ParentDir => continue,
        };
        crumbs.push(Crumb {
            name,
            path: acc.display().to_string(),
        });
    }
    crumbs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn crumbs_include_the_drive() {
        let c = crumbs(Path::new(r"C:\Users\me"));
        let pairs: Vec<(&str, &str)> = c
            .iter()
            .map(|c| (c.name.as_str(), c.path.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("C:", r"C:\"),
                ("Users", r"C:\Users"),
                ("me", r"C:\Users\me")
            ]
        );
    }

    #[test]
    fn lists_a_folder_and_its_parent() {
        let dir = std::env::temp_dir().join(format!("cu-local-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();

        let listing = list(&dir.display().to_string(), &[]).unwrap();
        let mut names: Vec<(&str, bool, u64)> = listing
            .entries
            .iter()
            .map(|e| (e.name.as_str(), e.is_dir, e.size))
            .collect();
        names.sort();
        assert_eq!(names, [("a.txt", false, 5), ("sub", true, 0)]);
        assert_eq!(
            listing.parent.as_deref(),
            Some(
                std::env::temp_dir()
                    .display()
                    .to_string()
                    .trim_end_matches(['\\', '/'])
            )
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn creates_and_renames_folders() {
        let dir = std::env::temp_dir().join(format!("cu-local-ops-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let parent = dir.display().to_string();

        let made = mkdir(&parent, " New ").unwrap();
        assert!(Path::new(&made).is_dir());
        assert!(mkdir(&parent, "New").is_err());
        assert!(mkdir(&parent, "..").is_err());
        #[cfg(windows)]
        assert!(mkdir(&parent, "a:b").is_err());

        std::fs::create_dir(dir.join("Other")).unwrap();
        assert!(rename(&made, "Other").is_err());
        let renamed = rename(&made, "Renamed").unwrap();
        assert!(Path::new(&renamed).is_dir());
        assert!(!Path::new(&made).exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn root_lists_drives() {
        let listing = list("", &[]).unwrap();
        assert!(listing.parent.is_none());
        assert!(listing.entries.iter().any(|e| e.icon == Some("drive")));
    }
}
