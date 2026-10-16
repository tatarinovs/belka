//! `belka upload LOCAL SOURCE:PATH`: copies a file or a folder into a cloud
//! folder, skipping what is already there; then optionally deletes what is
//! no longer local (`--delete-extra`) or old dated backups (`--keep-*`),
//! and shares what was uploaded (`--share`).

use super::rotate::{self, RotateOptions};
use super::{log_totals, Stats};
use crate::remote::Target;
use anyhow::{Context, Result};
use belka_core::storage::ShareOptions;
use belka_core::sync::{self, Filter, Index, UploadOptions};
use belka_core::utils::path::join_remote;
use belka_core::StorageProvider;
use chrono::NaiveDateTime;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;
use tracing::{error, info, warn};

pub struct Options {
    pub jobs: usize,
    pub dry_run: bool,
    pub overwrite: bool,
    pub verify: bool,
    pub delete_extra: bool,
    pub keep_last: Option<u32>,
    pub keep_days: Option<u32>,
    pub filter: Filter,
    /// Remember what is uploaded, so unchanged files are not even read next time.
    pub index: bool,
    /// After a successful upload, share what was uploaded with these settings.
    pub share: Option<ShareOptions>,
    pub clipboard: bool,
}

pub async fn run(
    provider: &dyn StorageProvider,
    source_name: &str,
    local: &Path,
    target: &Target,
    now: NaiveDateTime,
    opts: &Options,
) -> Result<Stats> {
    let remote_dir = &target.remote_dir;
    info!("Scanning '{}'...", local.display());
    let root = local.to_path_buf();
    let filter = opts.filter.clone();
    let tree = tokio::task::spawn_blocking(move || sync::scan(&root, &filter))
        .await
        .context("Failed to scan local files")??;
    for err in &tree.errors {
        error!("  [!] Cannot read {err}");
    }
    let total: u64 = tree.files.iter().map(|f| f.size).sum();
    info!(
        "Found {} file(s), {} total; uploading into '{remote_dir}' with {} parallel job(s)",
        tree.files.len(),
        belka_core::utils::format_bytes(total),
        opts.jobs
    );

    // A dry run must not create the folder, only see that the cloud answers.
    let access = if opts.dry_run {
        provider.get_file_info(remote_dir).await.map(drop)
    } else {
        provider.check_access(remote_dir).await
    };
    access.with_context(|| {
        format!(
            "Cannot access '{remote_dir}' on {}; check credentials and network",
            provider.name()
        )
    })?;

    let index = (opts.index && !opts.dry_run && !opts.overwrite)
        .then(|| index_for(local, source_name, remote_dir))
        .flatten()
        .map(Mutex::new);
    let started = Instant::now();
    let upload_opts = UploadOptions {
        parallel: opts.jobs,
        overwrite: opts.overwrite,
        dry_run: opts.dry_run,
        verify: opts.verify,
        create_dirs: true,
    };
    let summary = sync::upload(provider, &tree, remote_dir, index.as_ref(), &upload_opts, &()).await?;
    if let Some(index) = &index {
        let mut index = index.lock().unwrap();
        index.retain_scanned(&tree);
        if let Err(err) = index.save() {
            warn!("{err:#}");
        }
    }
    let mut stats = Stats::from(&summary);
    stats.failed += tree.errors.len() as u64;
    log_totals("uploaded", &stats, started.elapsed(), opts.dry_run);

    if let Some(share) = &opts.share {
        if stats.failed > 0 {
            warn!(
                "Not shared: {} failure(s), the cloud copy may be incomplete",
                stats.failed
            );
        } else if let Err(err) = share_uploaded(provider, local, remote_dir, share, opts).await {
            error!("{err:#}");
            stats.failed += 1;
        }
    }

    // A broken new backup must not cost the old good ones.
    let clean = opts.delete_extra || opts.keep_last.is_some() || opts.keep_days.is_some();
    if clean && stats.failed > 0 {
        warn!(
            "Nothing is deleted: {} failure(s), the cloud copy may be incomplete",
            stats.failed
        );
        return Ok(stats);
    }
    if opts.delete_extra {
        info!("Deleting extra items in '{remote_dir}'...");
        let extra = sync::delete_extra(provider, &tree, remote_dir, &opts.filter, opts.dry_run).await?;
        stats.deleted += extra.transferred;
        stats.failed += extra.failed;
    }
    if opts.keep_last.is_some() || opts.keep_days.is_some() {
        let rotate_opts = RotateOptions {
            keep_last: opts.keep_last.map(|n| n as usize),
            keep_days: opts.keep_days,
            dry_run: opts.dry_run,
            protect: protected_names(local, target),
        };
        info!("Rotating backups in '{}'...", target.rotate_dir);
        match rotate::rotate(provider, &target.rotate_dir, &rotate_opts, now).await {
            Ok(rotated) => stats.merge(rotated),
            Err(err) => {
                error!("{err:#}");
                stats.failed += 1;
            }
        }
    }
    Ok(stats)
}

/// Shares the uploaded file, or the cloud folder an uploaded folder went into.
async fn share_uploaded(
    provider: &dyn StorageProvider,
    local: &Path,
    remote_dir: &str,
    share: &ShareOptions,
    opts: &Options,
) -> Result<()> {
    let (path, is_dir) = match local.file_name().and_then(|n| n.to_str()) {
        Some(name) if local.is_file() => (join_remote(remote_dir, name), false),
        _ => (remote_dir.to_string(), true),
    };
    if opts.dry_run {
        info!("Would share '{path}'");
        return Ok(());
    }
    let link = super::share::link(provider, &path, is_dir, share)
        .await
        .with_context(|| format!("Failed to share '{path}'"))?;
    info!("Link: {link}");
    if opts.clipboard {
        let name = path.rsplit('/').next().unwrap_or_default().to_string();
        super::share::copy_links(&[(name, link)]);
    }
    Ok(())
}

/// The index of this source and destination, kept in the user's cache.
fn index_for(local: &Path, source: &str, remote_dir: &str) -> Option<Index> {
    let dir = cache_dir()?.join("index");
    let local = std::fs::canonicalize(local).unwrap_or_else(|_| local.to_path_buf());
    let target = format!("{}\n{source}:{remote_dir}", local.display());
    Some(Index::load(Index::file_for(&dir, &target), &target))
}

fn cache_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Caches"))
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
    };
    base.map(|b| b.join("belka"))
}

/// What this run wrote into the rotation folder: rotation touches only
/// these series and never deletes these names.
fn protected_names(local: &Path, target: &Target) -> HashSet<String> {
    if target.remote_dir != target.rotate_dir {
        let rest = target.remote_dir[target.rotate_dir.len()..].trim_start_matches('/');
        return rest
            .split('/')
            .next()
            .map(str::to_string)
            .into_iter()
            .collect();
    }
    if local.is_file() {
        return local
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
            .into_iter()
            .collect();
    }
    std::fs::read_dir(local)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protects_what_this_run_wrote() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("db_2026-01-01.gz"), b"x").unwrap();
        let templated = Target {
            remote_dir: "/B/2026-10-15/sub".into(),
            rotate_dir: "/B".into(),
        };
        assert_eq!(
            protected_names(dir.path(), &templated),
            HashSet::from(["2026-10-15".to_string()])
        );
        let plain = Target {
            remote_dir: "/B".into(),
            rotate_dir: "/B".into(),
        };
        assert_eq!(
            protected_names(dir.path(), &plain),
            HashSet::from(["db_2026-01-01.gz".to_string()])
        );
    }
}
