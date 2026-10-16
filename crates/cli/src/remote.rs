//! Cloud locations on the command line: `SOURCE:PATH`, like rclone remotes.
//!
//! SOURCE is, in this order:
//! 1. an account in the file given by `--accounts` (an export from the app);
//! 2. a provider kind such as `yandex` or `s3`, configured by environment
//!    variables (see `belka providers`);
//! 3. an account of the desktop app on this computer, by name.

use crate::config::{self, ProviderKind};
use anyhow::{bail, Context, Result};
use belka_core::utils::path::normalize_remote_dir;
use belka_core::StorageProvider;
use belka_store::accounts::{AccountSource, AccountStore, Backup};
use chrono::NaiveDateTime;
use clap::ValueEnum;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

pub const ACCOUNTS_PASSWORD_VAR: &str = "BELKA_ACCOUNTS_PASSWORD";

/// A parsed `SOURCE:PATH`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    pub source: String,
    /// As given; may hold `{date}` and other placeholders.
    pub path: String,
}

impl FromStr for Remote {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let Some((source, path)) = s.split_once(':') else {
            return Err(format!(
                "'{s}' is not a cloud location; write SOURCE:PATH, e.g. yandex:/Backups or \"My NAS:/Backups\""
            ));
        };
        let source = source.trim();
        if source.is_empty() {
            return Err(format!("'{s}' has no source before ':'"));
        }
        Ok(Self {
            source: source.to_string(),
            path: if path.trim().is_empty() {
                "/".into()
            } else {
                path.to_string()
            },
        })
    }
}

impl fmt::Display for Remote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.source, self.path)
    }
}

/// Where accounts come from, opened on first use.
pub struct Accounts {
    file: Option<PathBuf>,
    opened: Option<Box<dyn AccountSource + Sync>>,
}

impl Accounts {
    pub fn new(file: Option<PathBuf>) -> Self {
        Self { file, opened: None }
    }

    pub fn is_file(&self) -> bool {
        self.file.is_some()
    }

    /// The export file of `--accounts`, or the desktop app's accounts.
    pub fn source(&mut self) -> Result<&(dyn AccountSource + Sync)> {
        if self.opened.is_none() {
            self.opened = Some(match &self.file {
                Some(file) => Box::new(open_export(file)?),
                None => Box::new(app_accounts()?),
            });
        }
        Ok(self.opened.as_deref().expect("opened above"))
    }

    /// The provider for `source`, and how to name it in messages.
    pub fn provider(&mut self, source: &str) -> Result<(Arc<dyn StorageProvider>, String)> {
        if self.is_file() {
            let accounts = self.source()?;
            if let Ok(account) = accounts.find(source) {
                return Ok((accounts.provider(&account.id)?, account.name));
            }
        }
        if let Ok(kind) = ProviderKind::from_str(source, true) {
            let provider: Arc<dyn StorageProvider> = config::init_provider(kind)?.into();
            let name = kind
                .to_possible_value()
                .map(|v| v.get_name().to_string())
                .unwrap_or_default();
            return Ok((provider, name));
        }
        let accounts = self.source()?;
        let account = accounts.find(source).with_context(|| {
            format!(
                "Unknown source '{source}': not a provider (yandex, mailru, google, webdav, s3, \
                 synology) and not an account"
            )
        })?;
        Ok((accounts.provider(&account.id)?, account.name))
    }
}

fn open_export(file: &Path) -> Result<Backup> {
    let data =
        std::fs::read(file).with_context(|| format!("Failed to read '{}'", file.display()))?;
    let password = std::env::var(ACCOUNTS_PASSWORD_VAR)
        .ok()
        .filter(|p| !p.is_empty())
        .with_context(|| {
            format!("Set {ACCOUNTS_PASSWORD_VAR} to the password of '{}'", file.display())
        })?;
    Backup::open(&data, &password)
        .with_context(|| format!("Cannot open accounts file '{}'", file.display()))
}

/// The accounts of the desktop app on this computer. Their credentials are
/// in the system keyring, which the CLI reads on Windows and macOS.
pub fn app_accounts() -> Result<AccountStore> {
    if cfg!(not(any(windows, target_os = "macos"))) && std::env::var_os("BELKA_CONFIG_DIR").is_none() {
        bail!(
            "The desktop app's accounts are not available here; export them in the app \
             (Accounts → Export) and pass the file with --accounts"
        );
    }
    let dir = belka_store::default_dir().context("Cannot find the app's settings folder")?;
    Ok(AccountStore::new(
        &dir,
        belka_store::secrets::Secrets::system_or(dir.join("secrets.json")),
    ))
}

/// The cloud folder with placeholders expanded, and the folder rotation
/// works in: the parent of the first templated segment, else the target.
pub struct Target {
    pub remote_dir: String,
    pub rotate_dir: String,
}

impl Target {
    pub fn expand(template: &str, now: NaiveDateTime) -> Result<Self> {
        let template = normalize_remote_dir(template);
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
                "Unknown placeholder '{}' in the cloud path; use {{date}}, {{time}} or {{datetime}}",
                &expanded[start..end]
            );
        }
        let remote_dir = normalize_remote_dir(&expanded);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sources_and_paths() {
        let r: Remote = "yandex:/Backups/db".parse().unwrap();
        assert_eq!((r.source.as_str(), r.path.as_str()), ("yandex", "/Backups/db"));
        let r: Remote = "My NAS:".parse().unwrap();
        assert_eq!((r.source.as_str(), r.path.as_str()), ("My NAS", "/"));
        let r: Remote = "s3:a/b:c".parse().unwrap();
        assert_eq!(r.path, "a/b:c");
        assert!("/Backups".parse::<Remote>().is_err());
        assert!(":/x".parse::<Remote>().is_err());
    }

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    #[test]
    fn expands_placeholders() {
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
}
