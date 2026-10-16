//! `belka restore SOURCE:PATH LOCAL`: downloads a cloud folder (or file)
//! into a local folder. Unchanged files are skipped, so an interrupted
//! restore continues where it stopped.

use super::{log_totals, Stats};
use anyhow::Result;
use belka_core::sync::{self, DownloadOptions};
use belka_core::StorageProvider;
use std::path::Path;
use std::time::Instant;

pub async fn run(
    provider: &dyn StorageProvider,
    remote: &str,
    local: &Path,
    opts: &DownloadOptions,
) -> Result<Stats> {
    let started = Instant::now();
    let summary = sync::download(provider, remote, local, opts).await?;
    let stats = Stats::from(&summary);
    log_totals("downloaded", &stats, started.elapsed(), opts.dry_run);
    Ok(stats)
}
