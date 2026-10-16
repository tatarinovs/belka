pub mod accounts;
pub mod ls;
pub mod mirror;
pub mod restore;
pub mod rotate;
pub mod share;
pub mod task;
pub mod upload;

use belka_core::sync::Summary;
use belka_core::utils::format_bytes;
use std::time::Duration;
use tracing::info;

/// Counters reported at the end of a run; `failed > 0` yields a non-zero exit code.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    /// Files looked at closely; ones an index vouched for are not counted.
    pub processed: u64,
    /// Uploaded or downloaded.
    pub transferred: u64,
    pub unchanged: u64,
    pub deleted: u64,
    pub failed: u64,
    pub bytes: u64,
}

impl Stats {
    pub fn merge(&mut self, other: Stats) {
        self.processed += other.processed;
        self.transferred += other.transferred;
        self.unchanged += other.unchanged;
        self.deleted += other.deleted;
        self.failed += other.failed;
        self.bytes += other.bytes;
    }

    pub fn failed(count: u64) -> Self {
        Self {
            failed: count,
            ..Self::default()
        }
    }
}

impl From<&Summary> for Stats {
    fn from(s: &Summary) -> Self {
        Self {
            processed: s.checked,
            transferred: s.transferred,
            unchanged: s.unchanged,
            deleted: 0,
            failed: s.failed,
            bytes: s.bytes,
        }
    }
}

/// The closing lines of a transfer.
pub fn log_totals(verb: &str, stats: &Stats, elapsed: Duration, dry_run: bool) {
    info!("--------------------------------------------------");
    let deleted = if stats.deleted > 0 {
        format!(", {} deleted", stats.deleted)
    } else {
        String::new()
    };
    info!(
        "Files: {} {verb} ({}), {} unchanged{deleted}, {} failed",
        stats.transferred,
        format_bytes(stats.bytes),
        stats.unchanged,
        stats.failed
    );
    if !dry_run && stats.bytes > 0 {
        info!(
            "Elapsed: {:.0}s, average {}",
            elapsed.as_secs_f64(),
            belka_core::utils::format_rate(stats.bytes, elapsed)
        );
    }
}
