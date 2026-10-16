mod clipboard;
mod commands;
mod config;
mod login;
mod netpath;
mod notify;
mod remote;

use anyhow::{Context, Result};
use belka_core::sync::{DownloadOptions, Filter};
use clap::{Args, Parser, Subcommand};
use commands::mirror::{MirrorOptions, DEFAULT_ASSET_FILTER};
use commands::task::Selection;
use commands::Stats;
use config::ProviderKind;
use notify::{Mode, Notifier, NotifyOn, Report, Status};
use regex_lite::Regex;
use remote::{Accounts, Remote, Target};
use std::future::Future;
use std::io::IsTerminal;
use std::path::PathBuf;
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
    about = "Backs up to cloud storage, restores from it and mirrors GitHub releases",
    long_about = "Backs up to cloud storage, restores from it and mirrors GitHub releases.\n\n\
A cloud location is SOURCE:PATH. SOURCE is a provider set up by environment variables \
(yandex, mailru, google, webdav, s3, synology; see `belka providers`), an account of the \
desktop app on this computer, or an account in the file given by --accounts.\n\n\
Examples:\n  \
belka upload D:\\Backups yandex:/Backups --keep-last 7\n  \
belka upload ./site \"My NAS:/www\" --delete-extra --exclude '*.tmp'\n  \
belka restore google:/Backups/db ./restored\n  \
belka mirror webdav:/Mirror --urls urls.txt\n  \
belka share Z:\\disk\\photos.7z --expires 7",
    after_help = "Exit codes: 0 success, 1 fatal error, 2 invalid arguments, 3 finished with failed files, 130 interrupted."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Accounts exported from the app (a .belka file); its password is taken
    /// from BELKA_ACCOUNTS_PASSWORD
    #[arg(long, global = true, value_name = "FILE", env = "BELKA_ACCOUNTS")]
    accounts: Option<PathBuf>,

    /// Path to a .env file (default: ./.env, then .env next to the executable)
    #[arg(long, global = true, value_name = "FILE")]
    env_file: Option<PathBuf>,

    /// When to send notifications to Telegram and the webhook (see `belka providers`)
    #[arg(
        long,
        global = true,
        value_name = "WHEN",
        env = "NOTIFY_ON",
        value_enum,
        ignore_case = true,
        default_value = "failure"
    )]
    notify_on: NotifyOn,

    /// Verbose (debug) output
    #[arg(short, long, global = true, conflicts_with = "quiet")]
    verbose: bool,

    /// Only print warnings and errors
    #[arg(short, long, global = true)]
    quiet: bool,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Copy a local file or folder into a cloud folder, skipping what is already there
    Upload(UploadArgs),
    /// Download a cloud folder or file into a local folder
    Restore(RestoreArgs),
    /// Mirror GitHub releases and files by URL into a cloud folder
    Mirror(MirrorArgs),
    /// List a cloud folder (also a quick check that a source works)
    Ls {
        /// SOURCE:PATH
        remote: Remote,
        /// Also list subfolders
        #[arg(short = 'R', long)]
        recursive: bool,
    },
    /// Share files or folders by public link and copy the links to the clipboard
    Share(ShareArgs),
    /// List the accounts that SOURCE can name
    Accounts,
    /// The backup tasks of the desktop app
    Task {
        #[command(subcommand)]
        action: TaskAction,
    },
    /// Sign in to google or yandex through the browser and print the .env lines
    Login {
        #[arg(value_enum, ignore_case = true)]
        provider: ProviderKind,
    },
    /// List the environment variables of every provider and notification channel
    Providers,
    /// Send a test message to the configured notification channels
    NotifyTest,
}

#[derive(Args, Debug)]
struct TransferArgs {
    /// Show what would be transferred or deleted without changing anything
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// Files transferred in parallel
    #[arg(short, long, env = "CLOUD_JOBS", default_value_t = 3,
          value_parser = clap::value_parser!(u16).range(1..=32))]
    jobs: u16,
}

#[derive(Args, Debug)]
struct UploadArgs {
    /// Local file or folder
    local: PathBuf,
    /// SOURCE:PATH; {date}, {time} and {datetime} in PATH become the current date and time
    remote: Remote,
    #[command(flatten)]
    transfer: TransferArgs,

    /// Upload even what looks unchanged
    #[arg(short = 'f', long)]
    overwrite: bool,

    /// Read every uploaded file's size (and MD5 where the cloud has it) back
    #[arg(long)]
    verify: bool,

    /// After a successful upload, delete from the cloud folder what is not local
    #[arg(long, conflicts_with_all = ["keep_last", "keep_days"])]
    delete_extra: bool,

    /// After a successful upload, keep only the N newest dated backups of every series written
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..))]
    keep_last: Option<u32>,

    /// After a successful upload, keep dated backups of the last N days (today included)
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..))]
    keep_days: Option<u32>,

    /// Upload only files matching the pattern (*, **, ?; repeatable)
    #[arg(long, value_name = "PATTERN")]
    include: Vec<String>,

    /// Leave out files and folders matching the pattern (repeatable)
    #[arg(long, value_name = "PATTERN")]
    exclude: Vec<String>,

    /// Leave out names starting with a dot
    #[arg(long)]
    skip_hidden: bool,

    /// Compare every file with the cloud instead of trusting what the last
    /// runs remember
    #[arg(long)]
    no_index: bool,

    /// After a successful upload, share the file (or the cloud folder) and copy the link
    #[arg(long)]
    share: bool,

    /// With --share: the link works for N days, or until a date (YYYY-MM-DD)
    #[arg(long, value_name = "DAYS|DATE", value_parser = commands::share::parse_expiry, requires = "share")]
    expires: Option<chrono::NaiveDate>,

    /// With --share: ask this password when the link is opened
    // No `requires`: a password from the environment must not break uploads.
    #[arg(long, env = "BELKA_SHARE_PASSWORD", hide_env_values = true)]
    password: Option<String>,

    /// With --share: only print the link
    #[arg(long, requires = "share")]
    no_clipboard: bool,
}

#[derive(Args, Debug)]
struct RestoreArgs {
    /// SOURCE:PATH of a cloud folder or file
    remote: Remote,
    /// Local folder to download into
    local: PathBuf,
    #[command(flatten)]
    transfer: TransferArgs,

    /// Download even what looks unchanged locally
    #[arg(short = 'f', long)]
    overwrite: bool,
}

#[derive(Args, Debug)]
struct MirrorArgs {
    /// SOURCE:PATH of the cloud folder to mirror into
    remote: Remote,
    #[command(flatten)]
    transfer: TransferArgs,

    /// URL list: a local file or a cloud path [default: ./urls.txt, <exe dir>/urls.txt, PATH/urls.txt, /urls.txt]
    #[arg(long, value_name = "FILE", env = "CLOUD_URLS_FILE")]
    urls: Option<PathBuf>,

    /// Regex selecting which GitHub release assets to mirror
    #[arg(long, value_name = "REGEX", env = "CLOUD_ASSET_FILTER", default_value = DEFAULT_ASSET_FILTER)]
    assets: String,
}

#[derive(Args, Debug)]
struct ShareArgs {
    /// SOURCE:PATH, or a file on a Synology network share (\\NAS\share\... or a mapped drive)
    #[arg(required = true, value_name = "TARGET")]
    targets: Vec<String>,

    /// The link works for N days, or until a date (YYYY-MM-DD); replaces an existing link
    #[arg(long, value_name = "DAYS|DATE", value_parser = commands::share::parse_expiry)]
    expires: Option<chrono::NaiveDate>,

    /// Ask this password when the link is opened; replaces an existing link
    #[arg(long, env = "BELKA_SHARE_PASSWORD", hide_env_values = true)]
    password: Option<String>,

    /// Close access by link instead
    #[arg(long, conflicts_with = "expires")]
    unshare: bool,

    /// The Synology account of a network share, when it cannot be found by host
    #[arg(long, value_name = "NAME")]
    account: Option<String>,

    /// Only print the links
    #[arg(long)]
    no_clipboard: bool,
}

#[derive(Subcommand, Debug)]
enum TaskAction {
    /// List the tasks with their last results
    List,
    /// Run tasks now: by name, all of them, or the ones whose time has come
    Run {
        /// Task names
        #[arg(required_unless_present_any = ["all", "due"])]
        names: Vec<String>,
        /// Every enabled task
        #[arg(long, conflicts_with_all = ["names", "due"])]
        all: bool,
        /// Enabled tasks whose interval has passed; for a scheduler running this every hour
        #[arg(long, conflicts_with = "names")]
        due: bool,
    },
}

/// What a finished command reports to the exit code and the notifications.
struct Outcome {
    status: Status,
    stats: Stats,
    error: Option<String>,
}

#[tokio::main]
async fn main() -> ExitCode {
    // Load .env before parsing so it can provide defaults (CLOUD_JOBS, ...).
    let env_loaded = config::load_environment(config::env_file_from_args().as_deref());
    let cli = Cli::parse();

    if let Command::Providers = cli.command {
        config::print_providers();
        return ExitCode::SUCCESS;
    }
    let to_stderr = matches!(
        cli.command,
        Command::Login { .. } | Command::Ls { .. } | Command::Accounts | Command::Share(_)
    );
    init_logging(cli.verbose, cli.quiet, to_stderr);
    match env_loaded {
        Ok(Some(path)) => debug!("Loaded environment from {}", path.display()),
        Ok(None) => debug!("No .env file found, using environment variables only"),
        Err(err) => {
            error!("{err:#}");
            return ExitCode::from(EXIT_FATAL);
        }
    }

    match interruptible(dispatch(cli)).await {
        Some(Ok(code)) => ExitCode::from(code),
        Some(Err(err)) => {
            error!("{err:#}");
            ExitCode::from(EXIT_FATAL)
        }
        None => {
            warn!("Interrupted");
            ExitCode::from(EXIT_INTERRUPTED)
        }
    }
}

async fn interruptible<T>(work: impl Future<Output = T>) -> Option<T> {
    tokio::select! {
        result = work => Some(result),
        _ = tokio::signal::ctrl_c() => None,
    }
}

/// Runs the command; returns the exit code. Errors before any work starts
/// (bad settings) are not reported to the notification channels.
async fn dispatch(cli: Cli) -> Result<u8> {
    let mut accounts = Accounts::new(cli.accounts.clone());
    match cli.command {
        Command::Login { provider } => login::run(provider).await.map(|()| 0),
        Command::Providers => unreachable!("handled before logging starts"),
        Command::Accounts => commands::accounts::run(&mut accounts).map(|()| 0),
        Command::Share(args) => {
            let opts = commands::share::Options {
                share: belka_core::storage::ShareOptions {
                    password: args.password.filter(|p| !p.is_empty()),
                    expires: args.expires,
                },
                unshare: args.unshare,
                account: args.account,
                clipboard: !args.no_clipboard,
            };
            let failed = commands::share::run(&mut accounts, &args.targets, &opts).await?;
            Ok(match failed {
                0 => 0,
                n if n == args.targets.len() as u64 => EXIT_FATAL,
                _ => EXIT_PARTIAL,
            })
        }
        Command::Ls { remote, recursive } => {
            let (provider, _) = accounts.provider(&remote.source)?;
            commands::ls::run(&*provider, &remote.path, recursive)
                .await
                .map(|()| 0)
        }
        Command::NotifyTest => {
            let notifier = Notifier::from_env(cli.notify_on)?
                .context("No notification channel is configured (see `belka providers`)")?;
            info!("Sending a test notification...");
            notifier.test().await.map(|()| 0)
        }
        Command::Task {
            action: TaskAction::List,
        } => commands::task::list(&mut accounts).map(|()| 0),
        Command::Task {
            action: TaskAction::Run { names, all, due },
        } => {
            let selection = if all {
                Selection::All
            } else if due {
                Selection::Due
            } else {
                Selection::Named(names)
            };
            let notifier = Notifier::from_env(cli.notify_on)?;
            let done = commands::task::run(&mut accounts, selection).await?;
            let mut code = 0;
            for (task, stats, error) in done {
                let outcome = Outcome {
                    status: status_of(&stats, error.is_some()),
                    stats,
                    error,
                };
                code = code.max(exit_code(&outcome));
                if let Some(notifier) = &notifier {
                    let report = Report {
                        status: outcome.status,
                        mode: Mode::Task,
                        target: format!("«{}» {} -> {}", task.name, task.local, task.remote),
                        stats: outcome.stats,
                        elapsed: task
                            .last()
                            .and_then(|r| (r.finished? - r.started?).to_std().ok())
                            .unwrap_or_default(),
                        error: outcome.error.clone(),
                    };
                    notifier.finished(&report).await;
                }
            }
            Ok(code)
        }
        Command::Upload(args) => {
            let now = chrono::Local::now().naive_local();
            let target = Target::expand(&args.remote.path, now)?;
            let shown = format!("{}:{}", args.remote.source, target.remote_dir);
            std::fs::metadata(&args.local).with_context(|| {
                format!("Local path '{}' is not accessible", args.local.display())
            })?;
            let (provider, name) = accounts.provider(&args.remote.source)?;
            let opts = commands::upload::Options {
                jobs: usize::from(args.transfer.jobs),
                dry_run: args.transfer.dry_run,
                overwrite: args.overwrite,
                verify: args.verify,
                delete_extra: args.delete_extra,
                keep_last: args.keep_last,
                keep_days: args.keep_days,
                filter: Filter::new(&args.include, &args.exclude, args.skip_hidden),
                index: !args.no_index,
                share: args.share.then(|| belka_core::storage::ShareOptions {
                    password: args.password.filter(|p| !p.is_empty()),
                    expires: args.expires,
                }),
                clipboard: !args.no_clipboard,
            };
            // Uploading for nothing is no way to learn the link cannot be made.
            if let Some(share) = &opts.share {
                provider.check_share(share)?;
            }
            banner(&*provider, &shown, opts.dry_run);
            let work = commands::upload::run(&*provider, &name, &args.local, &target, now, &opts);
            reported(cli.notify_on, opts.dry_run, Mode::Upload, shown, work).await
        }
        Command::Restore(args) => {
            let (provider, _) = accounts.provider(&args.remote.source)?;
            let shown = args.remote.to_string();
            let opts = DownloadOptions {
                parallel: usize::from(args.transfer.jobs),
                overwrite: args.overwrite,
                dry_run: args.transfer.dry_run,
            };
            banner(&*provider, &shown, opts.dry_run);
            let work = commands::restore::run(&*provider, &args.remote.path, &args.local, &opts);
            reported(cli.notify_on, opts.dry_run, Mode::Restore, shown, work).await
        }
        Command::Mirror(args) => {
            let asset_filter =
                Regex::new(&args.assets).context("Invalid --assets regular expression")?;
            let (provider, _) = accounts.provider(&args.remote.source)?;
            let remote_dir = belka_core::utils::path::normalize_remote_dir(&args.remote.path);
            let shown = format!("{}:{remote_dir}", args.remote.source);
            let opts = MirrorOptions {
                dry_run: args.transfer.dry_run,
                jobs: usize::from(args.transfer.jobs),
                asset_filter,
            };
            banner(&*provider, &shown, opts.dry_run);
            let provider = &*provider;
            let work = async {
                // The list is checked before anything is downloaded.
                let list =
                    commands::mirror::load_url_list(provider, &remote_dir, args.urls.as_deref())
                        .await?;
                if !opts.dry_run {
                    provider.check_access(&remote_dir).await.with_context(|| {
                        format!("Cannot access '{remote_dir}' on {}", provider.name())
                    })?;
                }
                commands::mirror::run(provider, &remote_dir, &list, &opts).await
            };
            reported(cli.notify_on, opts.dry_run, Mode::Mirror, shown, work).await
        }
    }
}

fn banner(provider: &dyn belka_core::StorageProvider, shown: &str, dry_run: bool) {
    info!(
        "belka v{} | {} | {shown}{}",
        env!("CARGO_PKG_VERSION"),
        provider.name(),
        if dry_run { " | DRY RUN" } else { "" }
    );
}

/// Runs a transfer with notifications around it and turns the result into
/// an exit code. A dry run must not tell the monitoring that a backup was made.
async fn reported(
    notify_on: NotifyOn,
    dry_run: bool,
    mode: Mode,
    target: String,
    work: impl Future<Output = Result<Stats>>,
) -> Result<u8> {
    let notifier = Notifier::from_env(notify_on)?.filter(|_| !dry_run);
    if let Some(notifier) = &notifier {
        notifier.started().await;
    }
    let started = Instant::now();
    let outcome = match interruptible(work).await {
        None => {
            warn!("Interrupted");
            Outcome {
                status: Status::Interrupted,
                stats: Stats::default(),
                error: None,
            }
        }
        Some(Ok(stats)) => Outcome {
            status: status_of(&stats, false),
            stats,
            error: None,
        },
        Some(Err(err)) => {
            error!("{err:#}");
            Outcome {
                status: Status::Failure,
                stats: Stats::default(),
                error: Some(format!("{err:#}")),
            }
        }
    };
    match outcome.status {
        Status::Success => info!(
            "Done{}",
            if dry_run {
                " (dry run, nothing was changed)"
            } else {
                ""
            }
        ),
        Status::Partial => error!("Finished with {} failure(s)", outcome.stats.failed),
        _ => {}
    }
    if let Some(notifier) = &notifier {
        notifier
            .finished(&Report {
                status: outcome.status,
                mode,
                target,
                stats: outcome.stats,
                elapsed: started.elapsed(),
                error: outcome.error.clone(),
            })
            .await;
    }
    Ok(exit_code(&outcome))
}

fn status_of(stats: &Stats, error: bool) -> Status {
    if error {
        Status::Failure
    } else if stats.failed > 0 {
        Status::Partial
    } else {
        Status::Success
    }
}

fn exit_code(outcome: &Outcome) -> u8 {
    match outcome.status {
        Status::Success => 0,
        Status::Partial => EXIT_PARTIAL,
        Status::Failure => EXIT_FATAL,
        Status::Interrupted => EXIT_INTERRUPTED,
    }
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
            .with_target("belka_store", level)
    });

    let builder = tracing_subscriber::fmt()
        .with_max_level(LevelFilter::TRACE)
        .with_target(false)
        .with_timer(LocalTimer);
    // Commands that print results to stdout keep their log out of it.
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

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("belka").chain(args.iter().copied()))
    }

    #[test]
    fn upload_takes_a_path_and_a_location() {
        let cli = parse(&["upload", "d", "GDrive:/B", "-j", "4", "--exclude", "*.tmp"]).unwrap();
        let Command::Upload(args) = cli.command else {
            panic!("not upload");
        };
        assert_eq!(args.remote.source, "GDrive");
        assert_eq!(args.transfer.jobs, 4);
        assert_eq!(args.exclude, ["*.tmp"]);
        assert!(parse(&["upload", "d", "/no-source"]).is_err());
        assert!(parse(&["upload", "d", "y:/B", "-j", "0"]).is_err());
        assert!(parse(&["upload", "d", "y:/B", "--keep-last", "0"]).is_err());
        assert!(parse(&[
            "upload",
            "d",
            "y:/B",
            "--keep-days",
            "7",
            "--keep-last",
            "2"
        ])
        .is_ok());
        assert!(parse(&["upload", "d", "y:/B", "--delete-extra", "--keep-last", "2"]).is_err());
        assert!(parse(&["upload", "d", "y:/B", "--share", "--expires", "7"]).is_ok());
        assert!(parse(&["upload", "d", "y:/B", "--expires", "7"]).is_err());
    }

    #[test]
    fn task_run_needs_a_selection() {
        assert!(parse(&["task", "run"]).is_err());
        assert!(parse(&["task", "run", "--due"]).is_ok());
        assert!(parse(&["task", "run", "a", "--all"]).is_err());
        assert!(parse(&["-v", "task", "run", "Камера"]).is_ok());
    }

    #[test]
    fn share_takes_targets_and_link_settings() {
        let cli = parse(&["share", r"Z:\disk\a.7z", "NAS:/docs/b", "--expires", "7"]).unwrap();
        let Command::Share(args) = cli.command else {
            panic!("not share");
        };
        assert_eq!(args.targets.len(), 2);
        assert!(args.expires.is_some());
        assert!(parse(&["share"]).is_err());
        assert!(parse(&["share", "x:/a", "--expires", "0"]).is_err());
        assert!(parse(&["share", "x:/a", "--unshare", "--expires", "7"]).is_err());
        assert!(parse(&["share", "x:/a", "--unshare", "--password", "p"]).is_ok());
    }

    #[test]
    fn global_options_go_anywhere() {
        let cli = parse(&["ls", "yandex:/", "--accounts", "a.belka", "-q"]).unwrap();
        assert_eq!(cli.accounts, Some(PathBuf::from("a.belka")));
        assert!(cli.quiet);
    }
}
