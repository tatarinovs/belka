//! `belka task`: the backup tasks of the desktop app. With the system
//! scheduler running `belka task run --due` every hour, tasks run on time
//! while the app is closed.

use super::Stats;
use crate::remote::Accounts;
use anyhow::{bail, Context, Result};
use belka_store::tasks::{self, Task, TaskStore};
use belka_core::utils::format_bytes;
use tracing::{error, info, warn};

pub fn store() -> Result<TaskStore> {
    Ok(TaskStore::new(
        belka_store::default_dir().context("Cannot find the app's settings folder")?,
    ))
}

pub fn list(accounts: &mut Accounts) -> Result<()> {
    let tasks = store()?.list()?;
    if tasks.is_empty() {
        eprintln!("There are no backup tasks; they are made in the app");
        return Ok(());
    }
    let names = accounts.source().ok().and_then(|s| s.accounts().ok()).unwrap_or_default();
    for task in &tasks {
        let account = names
            .iter()
            .find(|a| a.id == task.account_id)
            .map_or("?", |a| a.name.as_str());
        println!(
            "{}{}\n    {} -> {account}:{}\n    every {} h{}",
            task.name,
            if task.enabled { "" } else { " (off)" },
            task.local,
            task.remote,
            task.interval_hours,
            match task.last() {
                Some(last) => format!(
                    "; last run {}: {} uploaded ({}), {} failed{}",
                    last.started
                        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_default(),
                    last.uploaded,
                    format_bytes(last.bytes),
                    last.failed,
                    last.error.as_deref().map(|e| format!(", {e}")).unwrap_or_default()
                ),
                None => "; never run".into(),
            }
        );
    }
    Ok(())
}

/// Which tasks `task run` takes.
pub enum Selection {
    Named(Vec<String>),
    All,
    Due,
}

/// Runs the selected tasks one by one; returns what each did.
pub async fn run(accounts: &mut Accounts, selection: Selection) -> Result<Vec<(Task, Stats, Option<String>)>> {
    let store = store()?;
    let all = store.list()?;
    let chosen: Vec<Task> = match selection {
        Selection::Named(names) => names
            .iter()
            .map(|name| store.find(name))
            .collect::<Result<_>>()?,
        Selection::All => all.into_iter().filter(|t| t.enabled).collect(),
        Selection::Due => {
            let now = chrono::Utc::now();
            all.into_iter().filter(|t| t.is_due(now)).collect()
        }
    };
    if chosen.is_empty() {
        info!("No backup task to run");
        return Ok(Vec::new());
    }
    let source = accounts.source()?;
    let mut done = Vec::new();
    for task in chosen {
        if tasks::is_running(&task.id) {
            bail!("The backup «{}» is running already", task.name);
        }
        info!("Backup «{}»: {} -> {}", task.name, task.local, task.remote);
        let provider = |id: &str| source.provider(id);
        match tasks::run(&store, &task.id, &provider).await {
            Ok(run) => {
                let r = &run.result;
                let stats = Stats {
                    processed: r.uploaded + r.unchanged + r.failed,
                    transferred: r.uploaded,
                    unchanged: r.unchanged,
                    deleted: r.freed,
                    failed: r.failed + u64::from(r.error.is_some()),
                    bytes: r.bytes,
                };
                for (file, reason) in &r.failures {
                    warn!("  [!] {file}: {reason}");
                }
                if let Some(err) = &r.error {
                    error!("Backup «{}» failed: {err}", task.name);
                }
                info!(
                    "Backup «{}»: {} uploaded ({}), {} unchanged, {} failed",
                    task.name,
                    r.uploaded,
                    format_bytes(r.bytes),
                    r.unchanged,
                    r.failed
                );
                let error = r.error.clone();
                done.push((task, stats, error));
            }
            Err(err) => {
                error!("Backup «{}»: {err:#}", task.name);
                done.push((task, Stats::failed(1), Some(format!("{err:#}"))));
            }
        }
    }
    Ok(done)
}
