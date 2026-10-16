use super::{Filter, LocalTree, Summary};
use crate::storage::{RemoteFileInfo, StorageProvider};
use crate::utils::path::normalize_remote_dir;
use anyhow::{bail, Context, Result};
use std::collections::HashSet;
use tracing::{error, info};

/// Deletes from `remote_root` what the local tree no longer has, like
/// `rclone sync`. Entries the filter leaves out are kept: they were never
/// part of the copy. A cloud folder missing locally goes as a whole.
/// Returns the number of deleted entries in `transferred` and failures.
pub async fn delete_extra(
    provider: &dyn StorageProvider,
    tree: &LocalTree,
    remote_root: &str,
    filter: &Filter,
    dry_run: bool,
) -> Result<Summary> {
    let root = normalize_remote_dir(remote_root);
    if tree.single_file {
        bail!("Extra files are deleted only when a folder is copied");
    }
    if root == "/" {
        bail!("Refusing to delete extra files in the root folder '/'; use a subfolder");
    }
    let files: HashSet<&str> = tree.files.iter().map(|f| f.relative.as_str()).collect();
    let dirs: HashSet<&str> = tree.dirs.iter().map(String::as_str).collect();

    let mut summary = Summary::default();
    let mut pending = vec![(root.clone(), String::new())];
    while let Some((dir, relative)) = pending.pop() {
        let items = provider
            .list_dir(&dir)
            .await
            .with_context(|| format!("Failed to list '{dir}'"))?;
        for item in items {
            let child = if relative.is_empty() {
                item.name.clone()
            } else {
                format!("{relative}/{}", item.name)
            };
            if !filter.accepts(&child, item.is_dir) {
                continue;
            }
            if item.is_dir && dirs.contains(child.as_str()) {
                pending.push((item.path.clone(), child));
                continue;
            }
            if !item.is_dir && files.contains(child.as_str()) {
                continue;
            }
            remove(provider, &item, dry_run, &mut summary).await;
        }
    }
    info!(
        "Extra items {}: {}",
        if dry_run { "to delete" } else { "deleted" },
        summary.transferred
    );
    Ok(summary)
}

async fn remove(
    provider: &dyn StorageProvider,
    item: &RemoteFileInfo,
    dry_run: bool,
    summary: &mut Summary,
) {
    if dry_run {
        info!("  [dry-run] Would delete {}", item.path);
        summary.transferred += 1;
        return;
    }
    match provider.delete(&item.path, item.is_dir).await {
        Ok(()) => {
            info!("  [-] Deleted {}", item.path);
            summary.transferred += 1;
        }
        Err(err) => {
            error!("  [!] Failed to delete '{}': {err:#}", item.path);
            summary.fail(&item.path, format!("{err:#}"));
        }
    }
}
