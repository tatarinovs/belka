use super::{Filter, LocalTree, Summary};
use crate::storage::{RemoteFileInfo, StorageProvider};
use crate::utils::path::normalize_remote_dir;
use anyhow::{bail, Context, Result};
use std::collections::HashSet;
use tracing::{error, info};

/// Deletes from `remote_root` what the local tree no longer has, like
/// `rclone sync`. Entries the filter leaves out are kept: they were never
/// part of the copy. A missing folder is removed as a whole only after its
/// contents have been checked for excluded or otherwise protected entries.
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
    if !tree.errors.is_empty() {
        bail!("Refusing to delete extra files after an incomplete local scan");
    }
    let files: HashSet<&str> = tree.files.iter().map(|f| f.relative.as_str()).collect();
    let dirs: HashSet<&str> = tree.dirs.iter().map(String::as_str).collect();
    // A folder that became a link (say, moved to another disk) is still
    // there locally; its cloud copy is the only one left.
    let links: HashSet<&str> = tree.links.iter().map(String::as_str).collect();

    // Finish the scan before deleting anything. A failed listing must not
    // turn an unknown subtree into permission to recursively delete it.
    let mut plan = Vec::new();
    let mut protected = HashSet::new();
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
            if !filter.accepts(&child, item.is_dir) || links.contains(child.as_str()) {
                protect_parents(&child, &mut protected);
                continue;
            }
            if item.is_dir {
                pending.push((item.path.clone(), child.clone()));
                if dirs.contains(child.as_str()) {
                    protect_parents(&child, &mut protected);
                    continue;
                }
            }
            if !item.is_dir && files.contains(child.as_str()) {
                protect_parents(&child, &mut protected);
                continue;
            }
            plan.push((item, child));
        }
    }
    let mut summary = Summary::default();
    let mut deleted = HashSet::new();
    for (item, child) in plan {
        if (item.is_dir && protected.contains(&child)) || has_parent(&child, &deleted) {
            continue;
        }
        if remove(provider, &item, dry_run, &mut summary).await && item.is_dir {
            deleted.insert(child);
        }
    }
    info!(
        "Extra items {}: {}",
        if dry_run { "to delete" } else { "deleted" },
        summary.transferred
    );
    Ok(summary)
}

fn protect_parents(path: &str, protected: &mut HashSet<String>) {
    let mut path = path;
    loop {
        protected.insert(path.to_string());
        match path.rsplit_once('/') {
            Some((parent, _)) => path = parent,
            None => break,
        }
    }
}

fn has_parent(path: &str, parents: &HashSet<String>) -> bool {
    let mut path = path;
    while let Some((parent, _)) = path.rsplit_once('/') {
        if parents.contains(parent) {
            return true;
        }
        path = parent;
    }
    false
}

async fn remove(
    provider: &dyn StorageProvider,
    item: &RemoteFileInfo,
    dry_run: bool,
    summary: &mut Summary,
) -> bool {
    if dry_run {
        info!("  [dry-run] Would delete {}", item.path);
        summary.transferred += 1;
        return true;
    }
    match provider.delete(&item.path, item.is_dir).await {
        Ok(()) => {
            info!("  [-] Deleted {}", item.path);
            summary.transferred += 1;
            true
        }
        Err(err) => {
            error!("  [!] Failed to delete '{}': {err:#}", item.path);
            summary.fail(&item.path, format!("{err:#}"));
            false
        }
    }
}
