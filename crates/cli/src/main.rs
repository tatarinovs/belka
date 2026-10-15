mod config;
mod format;
mod hash;
mod login;
mod modes;
mod notify;

use belka_core::{client, storage, utils};

use anyhow::{bail, Context, Result};
use chrono::NaiveDateTime;
use clap::{Parser, ValueEnum};
use config::ProviderKind;
use modes::local_upload::UploadOptions;
use modes::remote_urls::{MirrorOptions, UrlList, DEFAULT_ASSET_FILTER};
use modes::restore::RestoreOptions;
use modes::rotate::RotateOptions;
use modes::Stats;
use notify::{Notifier, NotifyOn, Report, Status};
use regex_lite::Regex;
use std::collections::HashSet;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;
use tracing::{debug, error, info, warn};
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::prelude::*;

const EXIT_FATAL: u8 = 1;
const EXIT_PARTIAL: u8 = 3;
const EXIT_INTERRUPTED: u8 = 130;

#[derive(Parser, Debug)]
#[command(
    name = "belka",
    version,
    about = "Uploads backups to cloud storage and mirrors GitHub releases (Yandex Disk, Mail.ru, Google Drive, WebDAV, S3)",
    long_about = "Uploads a local file or directory to cloud storage, skipping files that are already \
up to date. Without LOCAL_PATH, mirrors the URLs listed in urls.txt (GitHub repositories are resolved \
to their latest release assets).\n\nCredentials are never passed on the command line: put them into a \
.env file or environment variables (see --list-providers).",
    after_help = "Exit codes: 0 success, 1 fatal error, 2 invalid arguments, 3 finished with failed files, 130 interrupted."
)]
struct Cli {
    /// Local file or directory to upload (with --restore: folder to download into).
    /// If omitted, mirrors URLs from urls.txt
    #[arg(value_name = "LOCAL_PATH")]
    local_path: Option<PathBuf>,

    /// Cloud provider
    #[arg(
        short,
        long,
        env = "CLOUD_PROVIDER",
        value_enum,
        ignore_case = true,
        default_value = "yandex"
    )]
    provider: ProviderKind,

    /// Target folder in the cloud; {date}, {time} and {datetime} are replaced
    /// with the current local date and time
    #[arg(short, long, env = "CLOUD_REMOTE_DIR", default_value = "/Upload")]
    remote: String,

    /// Download the cloud folder (or file) given by --remote into LOCAL_PATH
    #[arg(long, requires = "local_path", conflicts_with_all = ["clean", "keep_last", "keep_days"])]
    restore: bool,

    /// After a successful upload, keep only the N newest dated backups of every series
    #[arg(long, value_name = "N", requires = "local_path",
          value_parser = clap::value_parser!(u32).range(1..))]
    keep_last: Option<u32>,

    /// After a successful upload, keep dated backups of the last N days (today included)
    #[arg(long, value_name = "N", requires = "local_path",
          value_parser = clap::value_parser!(u32).range(1..))]
    keep_days: Option<u32>,

    /// When to send notifications to Telegram and the webhook (see --list-providers)
    #[arg(
        long,
        value_name = "WHEN",
        env = "NOTIFY_ON",
        value_enum,
        ignore_case = true,
        default_value = "failure"
    )]
    notify_on: NotifyOn,

    /// Send a test message to the configured notification channels and exit
    #[arg(long, conflicts_with_all = ["local_path", "login"])]
    notify_test: bool,

    /// Path to a .env file (default: ./.env, then .env next to the executable)
    #[arg(long, value_name = "FILE")]
    env_file: Option<PathBuf>,

    /// URL list: a local file or a cloud path [default: ./urls.txt, <exe dir>/urls.txt, <remote>/urls.txt, /urls.txt]
    #[arg(long, value_name = "PATH", env = "CLOUD_URLS_FILE")]
    urls_file: Option<PathBuf>,

    /// Delete everything in the target folder before uploading (keeps a cloud urls.txt)
    #[arg(short = 'c', long = "clean", alias = "clean-remote")]
    clean: bool,

    /// Upload even if the remote file looks identical
    #[arg(short = 'f', long)]
    overwrite: bool,

    /// Show what would be uploaded or deleted without changing anything
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// Number of files uploaded in parallel (local upload mode)
    #[arg(short, long, env = "CLOUD_JOBS", default_value_t = 2,
          value_parser = clap::value_parser!(u16).range(1..=32))]
    jobs: u16,

    /// Regex selecting which GitHub release assets to mirror
    #[arg(long, value_name = "REGEX", env = "CLOUD_ASSET_FILTER", default_value = DEFAULT_ASSET_FILTER)]
    assets: String,

    /// List supported providers and their configuration variables
    #[arg(long)]
    list_providers: bool,

    /// Sign in to google or yandex through the browser and print the .env lines
    #[arg(
        long,
        value_name = "PROVIDER",
        value_enum,
        ignore_case = true,
        conflicts_with = "local_path"
    )]
    login: Option<ProviderKind>,

    /// Verbose (debug) output
    #[arg(short, long, conflicts_with = "quiet")]
    verbose: bool,

    /// Only print warnings and errors
    #[arg(short, long)]
    quiet: bool,
}

enum Mode {
    Upload(PathBuf),
    Restore(PathBuf),
    Mirror(UrlList),
}

/// The target folder with placeholders expanded, and the folder rotation
/// works in: the parent of the first templated segment, else the target.
struct Target {
    remote_dir: String,
    rotate_dir: String,
}

impl Target {
    fn expand(template: &str, now: NaiveDateTime) -> Result<Self> {
        let template = utils::path::normalize_remote_dir(template);
        let segments: Vec<&str> = template.split('/').filter(|s| !s.is_empty()).collect();
        let templated = segments.iter().position(|s| s.contains('{'));
        let expanded = template
            .replace("{datetime}", &now.format("%Y-%m-%d_%H-%M-%S").to_string())
            .replace("{date}", &now.format("%Y-%m-%d").to_string())
            .replace("{time}", &now.format("%H-%M-%S").to_string());
        if let Some(start) = expanded.find('{') {
            let end = expanded[start..]
                .find('}')
                .map_or(expanded.len(), |i| start + i + 1);
            bail!(
                "Unknown placeholder '{}' in --remote; use {{date}}, {{time}} or {{datetime}}",
                &expanded[start..end]
            );
        }
        let remote_dir = utils::path::normalize_remote_dir(&expanded);
        let rotate_dir = match templated {
            Some(i) => format!("/{}", segments[..i].join("/")),
            None => remote_dir.clone(),
        };
        Ok(Self {
            remote_dir,
            rotate_dir,
        })
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    // Load .env before parsing so it can provide defaults (CLOUD_PROVIDER, ...).
    let env_loaded = config::load_environment(config::env_file_from_args().as_deref());
    let cli = Cli::parse();

    if cli.list_providers {
        config::print_providers();
        return ExitCode::SUCCESS;
    }
    init_logging(cli.verbose, cli.quiet, cli.login.is_some());

    match env_loaded {
        Ok(Some(path)) => debug!("Loaded environment from {}", path.display()),
        Ok(None) => debug!("No .env file found, using environment variables only"),
        Err(err) => {
            error!("{err:#}");
            return ExitCode::from(EXIT_FATAL);
        }
    }

    if let Some(kind) = cli.login {
        let result = tokio::select! {
            result = login::run(kind) => result,
            _ = tokio::signal::ctrl_c() => {
                warn!("Interrupted");
                return ExitCode::from(EXIT_INTERRUPTED);
            }
        };
        return match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                error!("{err:#}");
                ExitCode::from(EXIT_FATAL)
            }
        };
    }

    let notifier = match Notifier::from_env(cli.notify_on) {
        Ok(notifier) => notifier,
        Err(err) => {
            error!("{err:#}");
            return ExitCode::from(EXIT_FATAL);
        }
    };
    if cli.notify_test {
        let Some(notifier) = notifier else {
            error!("No notification channel is configured (see --list-providers)");
            return ExitCode::from(EXIT_FATAL);
        };
        info!("Sending a test notification...");
        return match notifier.test().await {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                error!("{err:#}");
                ExitCode::from(EXIT_FATAL)
            }
        };
    }
    // A dry run must not tell the monitoring that a backup was made.
    if cli.dry_run && notifier.is_some() {
        debug!("Notifications are not sent in a dry run");
    }
    let notifier = notifier.filter(|_| !cli.dry_run);

    let started = Instant::now();
    let now = chrono::Local::now().naive_local();
    if let Some(notifier) = &notifier {
        notifier.started().await;
    }
    let target = Target::expand(&cli.remote, now);
    let remote_shown = target
        .as_ref()
        .map_or_else(|_| cli.remote.clone(), |t| t.remote_dir.clone());

    let result = tokio::select! {
        result = async { run(&cli, &target?, now).await } => Some(result),
        _ = tokio::signal::ctrl_c() => None,
    };

    let (code, status, stats, error_text) = match result {
        None => {
            warn!("Interrupted");
            (
                EXIT_INTERRUPTED,
                Status::Interrupted,
                Stats::default(),
                None,
            )
        }
        Some(Ok(stats)) if stats.failed == 0 => {
            info!(
                "Done{}",
                if cli.dry_run {
                    " (dry run, nothing was changed)"
                } else {
                    ""
                }
            );
            (0, Status::Success, stats, None)
        }
        Some(Ok(stats)) => {
            error!("Finished with {} failure(s)", stats.failed);
            (EXIT_PARTIAL, Status::Partial, stats, None)
        }
        Some(Err(err)) => {
            error!("{err:#}");
            let text = format!("{err:#}");
            (EXIT_FATAL, Status::Failure, Stats::default(), Some(text))
        }
    };

    if let Some(notifier) = &notifier {
        let mode = match (&cli.local_path, cli.restore) {
            (Some(_), true) => "restore",
            (Some(_), false) => "upload",
            (None, _) => "mirror",
        };
        notifier
            .finished(&Report {
                status,
                mode,
                provider: provider_label(cli.provider),
                remote: remote_shown,
                stats,
                elapsed: started.elapsed(),
                error: error_text,
            })
            .await;
    }
    ExitCode::from(code)
}

fn provider_label(kind: ProviderKind) -> String {
    kind.to_possible_value()
        .map(|v| v.get_name().to_string())
        .unwrap_or_default()
}

async fn run(cli: &Cli, target: &Target, now: NaiveDateTime) -> Result<Stats> {
    let remote_dir = target.remote_dir.clone();
    let asset_filter = Regex::new(&cli.assets).context("Invalid --assets regular expression")?;
    let provider = config::init_provider(cli.provider)?;

    info!(
        "belka v{} | {} | target folder '{remote_dir}'{}",
        env!("CARGO_PKG_VERSION"),
        provider.name(),
        if cli.dry_run { " | DRY RUN" } else { "" }
    );

    // Everything that can fail is validated before anything is deleted.
    let mode = match &cli.local_path {
        Some(path) if cli.restore => Mode::Restore(path.clone()),
        Some(path) => {
            std::fs::metadata(path)
                .with_context(|| format!("Local path '{}' is not accessible", path.display()))?;
            Mode::Upload(path.clone())
        }
        None => Mode::Mirror(
            modes::remote_urls::load_url_list(&*provider, &remote_dir, cli.urls_file.as_deref())
                .await?,
        ),
    };

    if let Mode::Restore(local) = &mode {
        let opts = RestoreOptions {
            overwrite: cli.overwrite,
            dry_run: cli.dry_run,
            jobs: usize::from(cli.jobs),
        };
        return modes::restore::run_restore(&*provider, &remote_dir, local, &opts).await;
    }

    let access = if cli.dry_run {
        provider.list_dir(&remote_dir).await.map(drop)
    } else {
        provider.check_access(&remote_dir).await
    };
    access.with_context(|| {
        format!(
            "Cannot access '{remote_dir}' on {}; check credentials and network",
            provider.name()
        )
    })?;

    let mut stats = Stats::default();
    if cli.clean {
        let keep: Vec<String> = match &mode {
            Mode::Mirror(list) => list.remote_path().map(str::to_string).into_iter().collect(),
            Mode::Upload(_) | Mode::Restore(_) => Vec::new(),
        };
        stats.merge(
            modes::clean::clean_remote_dir(&*provider, &remote_dir, &keep, cli.dry_run).await?,
        );
    }

    match mode {
        Mode::Upload(path) => {
            let opts = UploadOptions {
                overwrite: cli.overwrite || cli.clean,
                dry_run: cli.dry_run,
                jobs: usize::from(cli.jobs),
            };
            stats.merge(
                modes::local_upload::run_local_upload(&*provider, &path, &remote_dir, &opts)
                    .await?,
            );
            if cli.keep_last.is_some() || cli.keep_days.is_some() {
                stats.merge(rotate(cli, &*provider, &path, target, now, stats.failed).await);
            }
        }
        Mode::Restore(_) => unreachable!("handled above"),
        Mode::Mirror(list) => {
            let opts = MirrorOptions {
                dry_run: cli.dry_run,
                asset_filter,
            };
            stats.merge(
                modes::remote_urls::run_remote_urls(&*provider, &remote_dir, &list, &opts).await?,
            );
        }
    }
    Ok(stats)
}

/// Deletes old backups unless something failed: a broken new backup must
/// not cost an old good one.
async fn rotate(
    cli: &Cli,
    provider: &dyn storage::StorageProvider,
    local: &Path,
    target: &Target,
    now: NaiveDateTime,
    failed: u64,
) -> Stats {
    if failed > 0 {
        warn!("Rotation skipped: {failed} failure(s), old backups are kept");
        return Stats::default();
    }
    let opts = RotateOptions {
        keep_last: cli.keep_last.map(|n| n as usize),
        keep_days: cli.keep_days,
        dry_run: cli.dry_run,
        protect: protected_names(local, target),
    };
    info!("Rotating backups in '{}'...", target.rotate_dir);
    match modes::rotate::rotate(provider, &target.rotate_dir, &opts, now).await {
        Ok(stats) => stats,
        Err(err) => {
            error!("{err:#}");
            Stats {
                failed: 1,
                ..Stats::default()
            }
        }
    }
}

/// What this run wrote into the rotation folder, which rotation never deletes.
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

fn init_logging(verbose: bool, quiet: bool, to_stderr: bool) {
    let level = match (verbose, quiet) {
        (true, _) => LevelFilter::DEBUG,
        (_, true) => LevelFilter::WARN,
        _ => LevelFilter::INFO,
    };
    // BELKA_LOG takes levels per target, e.g. "debug,reqwest=trace".
    let filter = log_filter_from_env().unwrap_or_else(|| {
        Targets::new()
            .with_default(LevelFilter::WARN)
            .with_target("belka", level)
            .with_target("belka_core", level)
    });

    let builder = tracing_subscriber::fmt()
        .with_max_level(LevelFilter::TRACE)
        .with_target(false)
        .with_timer(LocalTimer);
    // --login prints .env lines to stdout, so its log must not mix in.
    if to_stderr {
        builder
            .with_ansi(use_ansi(std::io::stderr().is_terminal()))
            .with_writer(std::io::stderr)
            .finish()
            .with(filter)
            .init();
    } else {
        builder
            .with_ansi(use_ansi(std::io::stdout().is_terminal()))
            .finish()
            .with(filter)
            .init();
    }
}

fn log_filter_from_env() -> Option<Targets> {
    let value = std::env::var("BELKA_LOG").ok()?;
    match value.parse() {
        Ok(filter) => Some(filter),
        Err(err) => {
            eprintln!("BELKA_LOG is ignored: {err}");
            None
        }
    }
}

struct LocalTimer;

impl tracing_subscriber::fmt::time::FormatTime for LocalTimer {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"))
    }
}

/// Colors only for interactive terminals, never in log files; honors NO_COLOR.
fn use_ansi(terminal: bool) -> bool {
    std::env::var_os("NO_COLOR").is_none() && terminal && enable_virtual_terminal()
}

#[cfg(windows)]
fn enable_virtual_terminal() -> bool {
    use std::os::raw::c_void;
    type Handle = *mut c_void;

    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;

    extern "system" {
        fn GetStdHandle(std_handle: u32) -> Handle;
        fn GetConsoleMode(console: Handle, mode: *mut u32) -> i32;
        fn SetConsoleMode(console: Handle, mode: u32) -> i32;
    }

    // SAFETY: plain Win32 console calls on this process's own stdout handle.
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        if handle.is_null() || handle == (-1isize as Handle) {
            return false;
        }
        let mut mode = 0u32;
        if GetConsoleMode(handle, &mut mode) == 0 {
            return false;
        }
        mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0
            || SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
    }
}

#[cfg(not(windows))]
fn enable_virtual_terminal() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    #[test]
    fn expands_remote_placeholders() {
        let now = at("2026-10-15 03:04:05");
        let t = Target::expand("/Backups/db_{date}/x", now).unwrap();
        assert_eq!(t.remote_dir, "/Backups/db_2026-10-15/x");
        assert_eq!(t.rotate_dir, "/Backups");
        let t = Target::expand("{datetime}", now).unwrap();
        assert_eq!(t.remote_dir, "/2026-10-15_03-04-05");
        assert_eq!(t.rotate_dir, "/");
        let t = Target::expand("/Backups/", now).unwrap();
        assert_eq!(
            (t.remote_dir.as_str(), t.rotate_dir.as_str()),
            ("/Backups", "/Backups")
        );
        assert!(Target::expand("/B/{year}", now).is_err());
    }

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

    #[test]
    fn restore_and_rotation_need_a_local_path() {
        assert!(Cli::try_parse_from(["x", "--restore"]).is_err());
        assert!(Cli::try_parse_from(["x", "--keep-last", "3"]).is_err());
        assert!(Cli::try_parse_from(["x", "--restore", "--keep-last", "3", "d"]).is_err());
        assert!(Cli::try_parse_from(["x", "--keep-last", "0", "d"]).is_err());
        assert!(Cli::try_parse_from(["x", "--keep-days", "7", "--keep-last", "2", "d"]).is_ok());
    }

    #[test]
    fn provider_aliases_are_accepted() {
        let cli = Cli::try_parse_from(["x", "-p", "GDrive", "-j", "4", "dir"]).unwrap();
        assert_eq!(cli.provider, ProviderKind::Google);
        assert_eq!(cli.jobs, 4);
        assert!(Cli::try_parse_from(["x", "-j", "0"]).is_err());
    }
}
