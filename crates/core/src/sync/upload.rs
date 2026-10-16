use super::{cloud_has, Index, LocalFile, LocalTree, RemoteListing, Summary};
use crate::progress::Progress;
use crate::storage::StorageProvider;
use crate::utils::hash::compute_md5_file;
use crate::utils::path::{join_remote, normalize_remote_dir, split_remote};
use anyhow::{bail, Context, Result};
use futures_util::stream::{self, StreamExt};
use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Instant;
use tracing::{error, info, warn};

/// The index is saved after this many recorded files, so a run that is
/// stopped halfway does not check them again.
const SAVE_EVERY: usize = 25;

#[derive(Clone, Debug)]
pub struct UploadOptions {
    pub parallel: usize,
    /// Upload even what looks unchanged.
    pub overwrite: bool,
    pub dry_run: bool,
    /// After every upload, read the file's size (and MD5, where the cloud
    /// has one) back from the cloud and compare.
    pub verify: bool,
    /// Create the local folders in the cloud, empty ones included; otherwise
    /// only the folders files go to appear.
    pub create_dirs: bool,
}

impl Default for UploadOptions {
    fn default() -> Self {
        Self {
            parallel: super::DEFAULT_PARALLEL,
            overwrite: false,
            dry_run: false,
            verify: false,
            create_dirs: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Uploaded, or in a dry run would be; with the size.
    Transferred(u64),
    Unchanged,
    Failed(String),
}

/// Follows a transfer, e.g. to show progress. Called from the transfer tasks.
pub trait Observer: Send + Sync {
    /// A file starts going over the network; `progress` follows its bytes.
    fn started(&self, _file: &LocalFile, _progress: &Progress) {}
    fn finished(&self, _file: &LocalFile, _outcome: &Outcome) {}
}

impl Observer for () {}

/// Uploads the files of `tree` into `remote_root`, keeping the folder
/// structure, except those the cloud already has. With `index`, files
/// recorded there unchanged are skipped without a look, and what is backed
/// up gets recorded; the caller saves the index at the end. Failures of
/// single files are counted in the summary; an error means nothing could
/// be done at all.
pub async fn upload(
    provider: &dyn StorageProvider,
    tree: &LocalTree,
    remote_root: &str,
    index: Option<&Mutex<Index>>,
    opts: &UploadOptions,
    observer: &dyn Observer,
) -> Result<Summary> {
    let root = normalize_remote_dir(remote_root);
    let mut summary = Summary::default();
    // Owned: streams over references trip the compiler's `Send` checks.
    let candidates: Vec<LocalFile> = match index {
        Some(index) if !opts.overwrite => {
            let index = index.lock().unwrap();
            tree.files
                .iter()
                .filter(|f| !index.is_current(f))
                .cloned()
                .collect()
        }
        _ => tree.files.clone(),
    };
    summary.unchanged = (tree.files.len() - candidates.len()) as u64;
    summary.checked = candidates.len() as u64;
    if candidates.is_empty() && !(opts.create_dirs && !tree.dirs.is_empty()) {
        return Ok(summary);
    }

    // Folder creation is serialized: Google Drive would make duplicates.
    let ensured = tokio::sync::Mutex::new(HashSet::new());
    if !opts.dry_run {
        provider
            .ensure_dir(&root)
            .await
            .with_context(|| format!("Failed to create folder '{root}'"))?;
        ensured.lock().await.insert(root.clone());
        if opts.create_dirs {
            for dir in &tree.dirs {
                let remote = join_remote(&root, dir);
                if let Err(err) = ensure(provider, &ensured, &remote).await {
                    error!("  [!] {err:#}");
                    summary.fail(dir, format!("{err:#}"));
                }
            }
        }
    }

    let listing = if opts.overwrite {
        RemoteListing::empty()
    } else {
        RemoteListing::new(
            provider,
            &root,
            candidates.iter().map(|f| f.relative.as_str()),
            opts.parallel,
        )
        .await
    };

    let mut results = stream::iter(candidates)
        .map(|file| {
            let (listing, ensured, root) = (&listing, &ensured, &root);
            async move {
                let target = join_remote(root, &file.relative);
                let outcome =
                    match upload_one(provider, listing, ensured, &file, &target, opts, observer)
                        .await
                    {
                        Ok(outcome) => outcome,
                        Err(err) => {
                            error!("  [!] Failed to upload '{target}': {err:#}");
                            Outcome::Failed(format!("{err:#}"))
                        }
                    };
                observer.finished(&file, &outcome);
                (file, outcome)
            }
        })
        .buffer_unordered(opts.parallel.max(1));

    let mut unsaved = 0;
    while let Some((file, outcome)) = results.next().await {
        match &outcome {
            Outcome::Transferred(bytes) => {
                summary.transferred += 1;
                summary.bytes += bytes;
            }
            Outcome::Unchanged => summary.unchanged += 1,
            Outcome::Failed(reason) => summary.fail(&file.relative, reason.clone()),
        }
        if let (Some(index), false, false) = (
            index,
            opts.dry_run,
            matches!(outcome, Outcome::Failed(_)),
        ) {
            let mut index = index.lock().unwrap();
            index.record(&file);
            unsaved += 1;
            if unsaved >= SAVE_EVERY {
                unsaved = 0;
                if let Err(err) = index.save() {
                    warn!("{err:#}");
                }
            }
        }
    }
    Ok(summary)
}

async fn ensure(
    provider: &dyn StorageProvider,
    ensured: &tokio::sync::Mutex<HashSet<String>>,
    dir: &str,
) -> Result<()> {
    let mut ensured = ensured.lock().await;
    if ensured.contains(dir) {
        return Ok(());
    }
    provider
        .ensure_dir(dir)
        .await
        .with_context(|| format!("Failed to create folder '{dir}'"))?;
    // Its parents exist now as well.
    let mut path = dir.to_string();
    while ensured.insert(path.clone()) && path != "/" {
        path = split_remote(&path).0;
    }
    Ok(())
}

async fn upload_one(
    provider: &dyn StorageProvider,
    listing: &RemoteListing,
    ensured: &tokio::sync::Mutex<HashSet<String>>,
    file: &LocalFile,
    target: &str,
    opts: &UploadOptions,
    observer: &dyn Observer,
) -> Result<Outcome> {
    let mut exists = true;
    if !opts.overwrite {
        match listing.info(provider, target).await {
            Ok(Some(remote)) => match cloud_has(&remote, file).await {
                Ok(true) => {
                    info!("  [=] Unchanged: {target}");
                    return Ok(Outcome::Unchanged);
                }
                Ok(false) => {}
                Err(err) => warn!("  [~] Cannot compare '{target}' ({err:#}); uploading anyway"),
            },
            Ok(None) => exists = false,
            Err(err) => warn!("  [~] Cannot check '{target}' ({err:#}); uploading anyway"),
        }
    }

    if opts.dry_run {
        info!(
            "  [dry-run] Would upload {target} ({})",
            crate::utils::format_bytes(file.size)
        );
        return Ok(Outcome::Transferred(file.size));
    }

    ensure(provider, ensured, &split_remote(target).0).await?;
    info!(
        "  [^] Uploading {target} ({})...",
        crate::utils::format_bytes(file.size)
    );
    let progress = Progress::new();
    progress.set_total(file.size);
    observer.started(file, &progress);
    let started = Instant::now();
    if exists {
        provider.upload_file(&file.path, target, &progress).await?;
    } else {
        provider.upload_new_file(&file.path, target, &progress).await?;
    }
    if opts.verify {
        verify(provider, file, target).await?;
    }
    info!(
        "  [+] Uploaded {target} in {:.1}s ({})",
        started.elapsed().as_secs_f64(),
        crate::utils::format_rate(file.size, started.elapsed())
    );
    Ok(Outcome::Transferred(file.size))
}

/// Reads the uploaded file's size, and its MD5 where the cloud keeps one,
/// back from the cloud.
async fn verify(provider: &dyn StorageProvider, file: &LocalFile, target: &str) -> Result<()> {
    let remote = provider
        .get_file_info(target)
        .await
        .context("Verification failed")?
        .with_context(|| format!("Verification failed: '{target}' is missing after upload"))?;
    if remote.size != file.size {
        bail!(
            "Verification failed: the cloud has {} bytes instead of {}",
            remote.size,
            file.size
        );
    }
    if let Some(remote_md5) = remote.md5.as_deref() {
        let local_md5 = compute_md5_file(&file.path).await?;
        if !local_md5.eq_ignore_ascii_case(remote_md5) {
            bail!("Verification failed: the MD5 in the cloud differs");
        }
    }
    Ok(())
}
