use super::{cloud_has, Index, LocalFile, LocalTree, RemoteListing, Summary};
use crate::progress::Progress;
use crate::storage::{AlreadyExists, StorageProvider};
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
/// Names tried for a file whose own name is taken: `a.jpg`, `a (1).jpg`, ...
const MAX_NAMES: usize = 100;

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
    /// A different cloud file under the name of a file the index has never
    /// backed up belongs to someone else, such as another phone backing up
    /// into the same folder: the local file goes next to it as `a (1).jpg`
    /// instead of replacing it. Needs an index; ignored with `overwrite`.
    pub keep_both: bool,
}

impl Default for UploadOptions {
    fn default() -> Self {
        Self {
            parallel: super::DEFAULT_PARALLEL,
            overwrite: false,
            dry_run: false,
            verify: false,
            create_dirs: false,
            keep_both: false,
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

/// What the files of one upload share.
struct Run<'a> {
    provider: &'a dyn StorageProvider,
    root: String,
    listing: RemoteListing,
    /// Folder creation is serialized: Google Drive would make duplicates.
    ensured: tokio::sync::Mutex<HashSet<String>>,
    /// With `keep_both`: cloud paths, relative to the root, that files of
    /// this run use or may use, so two of them never pick the same new name.
    claimed: Mutex<HashSet<String>>,
    opts: &'a UploadOptions,
    observer: &'a dyn Observer,
}

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
    let keep_both = opts.keep_both && !opts.overwrite && index.is_some();
    // Owned: streams over references trip the compiler's `Send` checks. With
    // each file, where it is in the cloud if the index has backed it up.
    let mut claimed = HashSet::new();
    let candidates: Vec<(LocalFile, Option<String>)> = match index {
        Some(index) if !opts.overwrite => {
            let index = index.lock().unwrap();
            if keep_both {
                claimed.extend(
                    tree.files
                        .iter()
                        .map(|f| index.cloud_path(&f.relative).to_string()),
                );
            }
            tree.files
                .iter()
                .filter(|f| !index.is_current(f))
                .map(|f| {
                    let known = index.contains(&f.relative);
                    (
                        f.clone(),
                        known.then(|| index.cloud_path(&f.relative).to_string()),
                    )
                })
                .collect()
        }
        _ => tree.files.iter().map(|f| (f.clone(), None)).collect(),
    };
    summary.unchanged = (tree.files.len() - candidates.len()) as u64;
    summary.checked = candidates.len() as u64;
    if candidates.is_empty() && !(opts.create_dirs && !tree.dirs.is_empty()) {
        return Ok(summary);
    }

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
            candidates
                .iter()
                .map(|(f, known)| known.as_deref().unwrap_or(&f.relative)),
            opts.parallel,
        )
        .await
    };
    let run = Run {
        provider,
        root,
        listing,
        ensured,
        claimed: Mutex::new(claimed),
        opts,
        observer,
    };

    let mut results = stream::iter(candidates)
        .map(|(file, known)| {
            let run = &run;
            async move {
                let (outcome, cloud_path) = match upload_one(run, &file, known, keep_both).await {
                    Ok(done) => done,
                    Err(err) => {
                        error!("  [!] Failed to upload '{}': {err:#}", file.relative);
                        (Outcome::Failed(format!("{err:#}")), String::new())
                    }
                };
                run.observer.finished(&file, &outcome);
                (file, outcome, cloud_path)
            }
        })
        .buffer_unordered(opts.parallel.max(1));

    let mut unsaved = 0;
    while let Some((file, outcome, cloud_path)) = results.next().await {
        match &outcome {
            Outcome::Transferred(bytes) => {
                summary.transferred += 1;
                summary.bytes += bytes;
            }
            Outcome::Unchanged => summary.unchanged += 1,
            Outcome::Failed(reason) => summary.fail(&file.relative, reason.clone()),
        }
        if let (Some(index), false, false) =
            (index, opts.dry_run, matches!(outcome, Outcome::Failed(_)))
        {
            let mut index = index.lock().unwrap();
            index.record_at(&file, &cloud_path);
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

/// Uploads a file unless the cloud has it. Returns the outcome and where the
/// file is in the cloud, relative to the root: `known` for a file the index
/// has backed up there, else its own path or, with `keep_both`, the first
/// name that is free or holds the same content.
async fn upload_one(
    run: &Run<'_>,
    file: &LocalFile,
    known: Option<String>,
    keep_both: bool,
) -> Result<(Outcome, String)> {
    if !keep_both || known.is_some() {
        let path = known.unwrap_or_else(|| file.relative.clone());
        let outcome = place(run, file, &path, false)
            .await?
            .expect("not exclusive");
        return Ok((outcome, path));
    }
    for n in 0..MAX_NAMES {
        let name = numbered(&file.relative, n);
        // The own name is claimed by the file itself.
        if n > 0 && !run.claimed.lock().unwrap().insert(name.clone()) {
            continue;
        }
        if let Some(outcome) = place(run, file, &name, true).await? {
            if n > 0 {
                info!(
                    "  [~] '{}' is backed up as '{name}': the name is taken by another file",
                    file.relative
                );
            }
            return Ok((outcome, name));
        }
    }
    bail!(
        "No free name for '{}' among {MAX_NAMES} tried",
        file.relative
    )
}

/// Uploads a file to `relative` under the root unless the cloud has it
/// there. A different file there is replaced, or with `exclusive` left
/// alone: `None`.
async fn place(
    run: &Run<'_>,
    file: &LocalFile,
    relative: &str,
    exclusive: bool,
) -> Result<Option<Outcome>> {
    let (provider, opts) = (run.provider, run.opts);
    let target = join_remote(&run.root, relative);
    let mut exists = true;
    if !opts.overwrite {
        let mut found = run.listing.info(provider, &target).await;
        // The listing is from the start of the run: another device may
        // have put a file there since.
        if exclusive && !opts.dry_run && matches!(found, Ok(None)) {
            found = provider.get_file_info(&target).await;
        }
        match found {
            Ok(Some(remote)) => match cloud_has(provider, &remote, file, false).await {
                Ok(true) => {
                    info!("  [=] Unchanged: {target}");
                    return Ok(Some(Outcome::Unchanged));
                }
                Ok(false) if exclusive => return Ok(None),
                Ok(false) => {}
                Err(err) if exclusive => {
                    warn!("  [~] Cannot compare '{target}' ({err:#}); keeping it");
                    return Ok(None);
                }
                Err(err) => warn!("  [~] Cannot compare '{target}' ({err:#}); uploading anyway"),
            },
            Ok(None) => exists = false,
            Err(err) if exclusive => {
                return Err(err).with_context(|| format!("Cannot check '{target}'"))
            }
            Err(err) => warn!("  [~] Cannot check '{target}' ({err:#}); uploading anyway"),
        }
    }

    if opts.dry_run {
        info!(
            "  [dry-run] Would upload {target} ({})",
            crate::utils::format_bytes(file.size)
        );
        return Ok(Some(Outcome::Transferred(file.size)));
    }

    ensure(provider, &run.ensured, &split_remote(&target).0).await?;
    info!(
        "  [^] Uploading {target} ({})...",
        crate::utils::format_bytes(file.size)
    );
    let progress = Progress::new();
    progress.set_total(file.size);
    run.observer.started(file, &progress);
    let started = Instant::now();
    if exists {
        provider.upload_file(&file.path, &target, &progress).await?;
    } else {
        match provider
            .upload_new_file(&file.path, &target, &progress)
            .await
        {
            Ok(()) => {}
            // Written by someone else while this file was on its way.
            Err(err) if err.is::<AlreadyExists>() => {
                if exclusive {
                    return Ok(None);
                }
                provider.upload_file(&file.path, &target, &progress).await?;
            }
            Err(err) => return Err(err),
        }
    }
    if opts.verify {
        verify(provider, file, &target).await?;
    }
    info!(
        "  [+] Uploaded {target} in {:.1}s ({})",
        started.elapsed().as_secs_f64(),
        crate::utils::format_rate(file.size, started.elapsed())
    );
    Ok(Some(Outcome::Transferred(file.size)))
}

/// `relative` with ` (n)` before the extension; itself for 0.
fn numbered(relative: &str, n: usize) -> String {
    if n == 0 {
        return relative.to_string();
    }
    let (dir, name) = relative
        .rsplit_once('/')
        .map_or(("", relative), |(d, n)| (d, n));
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    };
    let name = format!("{stem} ({n}){ext}");
    if dir.is_empty() {
        name
    } else {
        format!("{dir}/{name}")
    }
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

#[cfg(test)]
mod tests {
    use super::numbered;

    #[test]
    fn numbered_names_keep_the_extension() {
        assert_eq!(numbered("Camera/IMG_1.jpg", 0), "Camera/IMG_1.jpg");
        assert_eq!(numbered("Camera/IMG_1.jpg", 1), "Camera/IMG_1 (1).jpg");
        assert_eq!(numbered("a.b/README", 2), "a.b/README (2)");
        assert_eq!(numbered(".nomedia", 1), ".nomedia (1)");
        assert_eq!(numbered("x.tar.gz", 1), "x.tar (1).gz");
    }
}
