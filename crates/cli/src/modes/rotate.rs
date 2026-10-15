//! Deletes old backups from the target folder after a successful upload.
//!
//! A backup is recognized by a date in its name: `db_2026-10-15.sql.gz`,
//! `site-20261015_0300.7z.001`, a folder `2026-10-15`. Entries without a date
//! are never touched. Backups whose names differ only in the date form one
//! series (`db_*.sql.gz`), and every series is rotated on its own, so one
//! folder can hold several kinds of backups. Volumes of a multi-volume archive
//! (`.7z.001`, `.part01.rar`, `.z01`) make up one backup and are deleted together.

use super::Stats;
use crate::storage::{RemoteFileInfo, StorageProvider};
use anyhow::{bail, Context, Result};
use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};
use regex_lite::Regex;
use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;
use tracing::{debug, error, info};

pub struct RotateOptions {
    /// Keep this many newest backups of every series.
    pub keep_last: Option<usize>,
    /// Keep backups dated within this many days, today included.
    pub keep_days: Option<u32>,
    pub dry_run: bool,
    /// Names in the folder that must survive, such as what was just uploaded.
    pub protect: HashSet<String>,
}

/// One backup: a file, a folder or the volumes of one archive.
struct Backup {
    when: NaiveDateTime,
    key: String,
    entries: Vec<RemoteFileInfo>,
}

/// Rotates the backups in `dir`. `now` is the local time of the run.
pub async fn rotate(
    provider: &dyn StorageProvider,
    dir: &str,
    opts: &RotateOptions,
    now: NaiveDateTime,
) -> Result<Stats> {
    if opts.keep_last.is_none() && opts.keep_days.is_none() {
        bail!("Rotation needs --keep-last or --keep-days");
    }
    let items = provider
        .list_dir(dir)
        .await
        .with_context(|| format!("Failed to list '{dir}' for rotation"))?;

    let (series, undated) = group(items);
    if undated > 0 {
        debug!("Rotation: {undated} item(s) in '{dir}' have no date in the name and are kept");
    }
    let doomed = select_expired(series, opts, now.date());
    if doomed.is_empty() {
        info!("Rotation: nothing to delete in '{dir}'");
        return Ok(Stats::default());
    }

    let mut stats = Stats::default();
    for backup in doomed {
        let verb = if opts.dry_run {
            "Would delete"
        } else {
            "Deleting"
        };
        info!(
            "  [-] {verb} old backup {} ({})",
            backup.key,
            backup.when.format("%Y-%m-%d %H:%M")
        );
        for entry in backup.entries {
            if opts.dry_run {
                info!("  [dry-run] Would delete {}", entry.path);
                stats.deleted += 1;
                continue;
            }
            match provider.delete(&entry.path, entry.is_dir).await {
                Ok(()) => {
                    debug!("  [-] Deleted {}", entry.path);
                    stats.deleted += 1;
                }
                Err(err) => {
                    error!("  [!] Failed to delete '{}': {err:#}", entry.path);
                    stats.failed += 1;
                }
            }
        }
    }
    info!("Rotation: {} item(s) deleted from '{dir}'", stats.deleted);
    Ok(stats)
}

/// Splits folder entries into series of backups; returns them and the
/// number of entries without a date.
fn group(items: Vec<RemoteFileInfo>) -> (BTreeMap<String, BTreeMap<String, Backup>>, usize) {
    let mut series: BTreeMap<String, BTreeMap<String, Backup>> = BTreeMap::new();
    let mut undated = 0;
    for item in items {
        let key = if item.is_dir {
            item.name.clone()
        } else {
            strip_volume(&item.name)
        };
        let Some((when, pattern)) = parse_dated(&key) else {
            undated += 1;
            continue;
        };
        series
            .entry(pattern)
            .or_default()
            .entry(key.clone())
            .or_insert_with(|| Backup {
                when,
                key,
                entries: Vec::new(),
            })
            .entries
            .push(item);
    }
    (series, undated)
}

/// The backups that no rule keeps.
fn select_expired(
    series: BTreeMap<String, BTreeMap<String, Backup>>,
    opts: &RotateOptions,
    today: NaiveDate,
) -> Vec<Backup> {
    let oldest_kept_day = opts
        .keep_days
        .map(|days| today - Duration::days(i64::from(days.max(1)) - 1));
    let mut doomed = Vec::new();
    for (pattern, backups) in series {
        let mut backups: Vec<Backup> = backups.into_values().collect();
        backups.sort_by(|a, b| b.when.cmp(&a.when).then_with(|| b.key.cmp(&a.key)));
        debug!(
            "Rotation: series '{pattern}' has {} backup(s)",
            backups.len()
        );
        for (index, backup) in backups.into_iter().enumerate() {
            let by_count = opts.keep_last.is_some_and(|n| index < n.max(1));
            let by_age = oldest_kept_day.is_some_and(|day| backup.when.date() >= day);
            let protected = backup
                .entries
                .iter()
                .any(|e| opts.protect.contains(&e.name));
            if !(by_count || by_age || protected) {
                doomed.push(backup);
            }
        }
    }
    doomed
}

/// The name of the whole archive for a volume: `a.7z.001` -> `a.7z`,
/// `a.part01.rar` -> `a.rar`, `a.z01` -> `a.zip`, `a.r00` -> `a.rar`.
fn strip_volume(name: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)^(.+?)(?:(\.[^.]+)\.\d{3}|\.part\d+(\.rar)|\.(z)\d{2}|\.(r)\d{2})$")
            .unwrap()
    });
    let Some(caps) = re.captures(name) else {
        return name.to_string();
    };
    let base = &caps[1];
    if let Some(ext) = caps.get(2).or(caps.get(3)) {
        format!("{base}{}", ext.as_str())
    } else if caps.get(4).is_some() {
        format!("{base}.zip")
    } else {
        format!("{base}.rar")
    }
}

/// Finds the first plausible date (with an optional time) in `name`.
/// Returns it and the name with the date replaced by `*`, which identifies
/// the series. Accepts `2026-10-15`, `2026_10_15`, `20261015`, followed by
/// an optional time `03-00`, `0300`, `03.00.15`, `T030015`.
pub fn parse_dated(name: &str) -> Option<(NaiveDateTime, String)> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"(?:^|[^0-9])((\d{4})[-_.]?(\d{2})[-_.]?(\d{2})(?:[ _T.-]?(\d{2})[-_:.]?(\d{2})(?:[-_:.]?(\d{2}))?)?)(?:[^0-9]|$)",
        )
        .unwrap()
    });
    let mut start = 0;
    while let Some(caps) = re.captures_at(name, start) {
        let whole = caps.get(1).unwrap();
        let num = |i: usize| caps.get(i).and_then(|m| m.as_str().parse::<u32>().ok());
        let date = num(2)
            .filter(|y| (1990..=2200).contains(y))
            .and_then(|y| NaiveDate::from_ymd_opt(y as i32, num(3)?, num(4)?));
        if let Some(date) = date {
            let time = match (num(5), num(6)) {
                (Some(h), Some(m)) => NaiveTime::from_hms_opt(h, m, num(7).unwrap_or(0)),
                _ => Some(NaiveTime::MIN),
            };
            if let Some(time) = time {
                let pattern = format!("{}*{}", &name[..whole.start()], &name[whole.end()..]);
                return Some((date.and_time(time), pattern));
            }
        }
        // Retry from the next character: a number can hide a date further on.
        start = whole.start() + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    #[test]
    fn parses_dates_in_names() {
        let cases = [
            ("db_2026-10-15.sql.gz", "2026-10-15 00:00:00", "db_*.sql.gz"),
            ("2026-10-15", "2026-10-15 00:00:00", "*"),
            ("site-20261015_0300.7z", "2026-10-15 03:00:00", "site-*.7z"),
            (
                "x 2026.10.15 23-59-58.zip",
                "2026-10-15 23:59:58",
                "x *.zip",
            ),
            (
                "v2_2026_01_02T101112.tar",
                "2026-01-02 10:11:12",
                "v2_*.tar",
            ),
        ];
        for (name, when, pattern) in cases {
            assert_eq!(
                parse_dated(name),
                Some((dt(when), pattern.to_string())),
                "{name}"
            );
        }
        for name in [
            "backup.zip",
            "build-12345678.zip",
            "2026-13-01.zip",
            "urls.txt",
        ] {
            assert_eq!(parse_dated(name), None, "{name}");
        }
    }

    #[test]
    fn strips_volume_suffixes() {
        assert_eq!(strip_volume("a_2026-10-15.7z.001"), "a_2026-10-15.7z");
        assert_eq!(strip_volume("a.PART02.rar"), "a.rar");
        assert_eq!(strip_volume("a.z01"), "a.zip");
        assert_eq!(strip_volume("a.r00"), "a.rar");
        assert_eq!(strip_volume("a.zip"), "a.zip");
        assert_eq!(strip_volume("a.7z"), "a.7z");
    }

    fn file(name: &str) -> RemoteFileInfo {
        RemoteFileInfo {
            name: name.into(),
            path: format!("/B/{name}"),
            is_dir: false,
            size: 1,
            md5: None,
            modified: None,
        }
    }

    fn expired(names: &[&str], opts: &RotateOptions, today: &str) -> Vec<String> {
        let (series, _) = group(names.iter().map(|n| file(n)).collect());
        let today = NaiveDate::parse_from_str(today, "%Y-%m-%d").unwrap();
        let mut names: Vec<String> = select_expired(series, opts, today)
            .into_iter()
            .flat_map(|b| b.entries.into_iter().map(|e| e.name))
            .collect();
        names.sort();
        names
    }

    fn opts(keep_last: Option<usize>, keep_days: Option<u32>) -> RotateOptions {
        RotateOptions {
            keep_last,
            keep_days,
            dry_run: false,
            protect: HashSet::new(),
        }
    }

    #[test]
    fn keeps_newest_per_series_and_volumes_together() {
        let names = [
            "db_2026-10-13.gz",
            "db_2026-10-14.gz",
            "db_2026-10-15.gz",
            "site_2026-10-01.7z.001",
            "site_2026-10-01.7z.002",
            "site_2026-10-08.7z.001",
            "notes.txt",
        ];
        assert_eq!(
            expired(&names, &opts(Some(1), None), "2026-10-15"),
            [
                "db_2026-10-13.gz",
                "db_2026-10-14.gz",
                "site_2026-10-01.7z.001",
                "site_2026-10-01.7z.002"
            ]
        );
    }

    #[test]
    fn keep_days_counts_today() {
        let names = ["2026-10-08", "2026-10-09", "2026-10-15"];
        assert_eq!(
            expired(&names, &opts(None, Some(7)), "2026-10-15"),
            ["2026-10-08"]
        );
        // Either rule keeps a backup.
        assert!(expired(&names, &opts(Some(3), Some(1)), "2026-10-15").is_empty());
    }

    #[test]
    fn protected_names_survive() {
        let mut o = opts(Some(1), None);
        o.protect.insert("a_2026-01-01.zip".into());
        assert!(expired(&["a_2026-01-01.zip", "a_2026-10-01.zip"], &o, "2026-10-15").is_empty());
    }
}
