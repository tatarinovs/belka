//! Saved cloud accounts. The public part (id, name, provider kind) is kept in
//! `accounts.json`; credentials are stored in the OS keyring (Windows
//! Credential Manager, macOS Keychain, Secret Service on Linux).
//!
//! An encrypted folder is an account of kind [`CRYPT_KIND`] that refers to
//! another account and keeps its password in the keyring.
//!
//! All of it can be exported into one file sealed with a password
//! ([`belka_core::crypt::seal`]) and imported on another computer.

use anyhow::{bail, Context, Result};
use belka_core::crypt::CryptConfig;
use belka_core::oauth::{OAuthApp, Service};
use belka_core::ProviderConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const KEYRING_SERVICE: &str = "belka-gui";
pub const CRYPT_KIND: &str = "crypt";

/// An encrypted folder in another account.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CryptFolder {
    /// The account that stores the encrypted files.
    pub base: String,
    /// The folder of that account that holds the encrypted tree.
    pub root: String,
    #[serde(flatten)]
    pub config: CryptConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub kind: String,
    /// Where an encrypted folder lives; not secret, so the UI can mark it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crypt: Option<CryptLocation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CryptLocation {
    pub base: String,
    pub root: String,
}

pub struct AccountStore {
    file: PathBuf,
}

impl AccountStore {
    pub fn new(config_dir: PathBuf) -> Self {
        Self {
            file: config_dir.join("accounts.json"),
        }
    }

    pub fn list(&self) -> Result<Vec<Account>> {
        match std::fs::read_to_string(&self.file) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("Failed to parse '{}'", self.file.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(err) => {
                Err(err).with_context(|| format!("Failed to read '{}'", self.file.display()))
            }
        }
    }

    /// Writes through a temporary file so a crash never leaves a truncated list.
    fn save(&self, accounts: &[Account]) -> Result<()> {
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("Failed to create '{}'", dir.display()))?;
        }
        let tmp = self.file.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(accounts)?)
            .with_context(|| format!("Failed to write '{}'", tmp.display()))?;
        std::fs::rename(&tmp, &self.file)
            .with_context(|| format!("Failed to write '{}'", self.file.display()))
    }

    pub fn add(&self, name: &str, config: &ProviderConfig) -> Result<Account> {
        self.add_secret(name, config.kind(), None, &serde_json::to_string(config)?)
    }

    pub fn add_crypt(&self, name: &str, folder: &CryptFolder) -> Result<Account> {
        let location = CryptLocation {
            base: folder.base.clone(),
            root: folder.root.clone(),
        };
        self.add_secret(
            name,
            CRYPT_KIND,
            Some(location),
            &serde_json::to_string(folder)?,
        )
    }

    fn add_secret(
        &self,
        name: &str,
        kind: &str,
        crypt: Option<CryptLocation>,
        secret: &str,
    ) -> Result<Account> {
        let name = name.trim();
        if name.is_empty() {
            bail!("Account name is empty");
        }
        let mut accounts = self.list()?;
        let account = Account {
            id: new_id(),
            name: name.to_string(),
            kind: kind.to_string(),
            crypt,
        };
        entry(&account.id)?
            .set_password(secret)
            .context("Failed to save credentials to the system keyring")?;
        accounts.push(account.clone());
        if let Err(err) = self.save(&accounts) {
            if let Ok(e) = entry(&account.id) {
                let _ = e.delete_credential();
            }
            return Err(err);
        }
        Ok(account)
    }

    /// Renames an account and, with `config`, replaces its credentials.
    pub fn update(&self, id: &str, name: &str, config: Option<&ProviderConfig>) -> Result<Account> {
        let name = name.trim();
        if name.is_empty() {
            bail!("Account name is empty");
        }
        let mut accounts = self.list()?;
        let account = accounts
            .iter_mut()
            .find(|a| a.id == id)
            .context("Account not found")?;
        if let Some(config) = config {
            if config.kind() != account.kind {
                bail!("The account type cannot be changed");
            }
            entry(id)?
                .set_password(&serde_json::to_string(config)?)
                .context("Failed to save credentials to the system keyring")?;
        }
        account.name = name.to_string();
        let account = account.clone();
        self.save(&accounts)?;
        Ok(account)
    }

    /// Puts the accounts in the order of `ids`; ones not listed keep their
    /// relative order at the end.
    pub fn reorder(&self, ids: &[String]) -> Result<()> {
        let mut accounts = self.list()?;
        accounts.sort_by_key(|a| ids.iter().position(|id| *id == a.id).unwrap_or(usize::MAX));
        self.save(&accounts)
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        let mut accounts = self.list()?;
        accounts.retain(|a| a.id != id);
        self.save(&accounts)?;
        match entry(id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err).context("Failed to delete credentials from the system keyring"),
        }
    }

    pub fn credentials(&self, id: &str) -> Result<ProviderConfig> {
        serde_json::from_str(&secret(id)?).context("Stored credentials are corrupted")
    }

    pub fn crypt_folder(&self, id: &str) -> Result<CryptFolder> {
        serde_json::from_str(&secret(id)?).context("Stored encryption settings are corrupted")
    }

    /// All accounts with their credentials, and the user's own OAuth apps.
    pub fn export(&self) -> Result<Backup> {
        let accounts = self
            .list()?
            .into_iter()
            .map(|account| {
                let secret =
                    secret(&account.id).with_context(|| format!("Account «{}»", account.name))?;
                Ok(SavedAccount { account, secret })
            })
            .collect::<Result<_>>()?;
        let oauth_apps = [Service::Google, Service::Yandex]
            .into_iter()
            .filter_map(|service| load_oauth_app(service).map(|app| SavedOAuthApp { service, app }))
            .collect();
        Ok(Backup {
            accounts,
            oauth_apps,
        })
    }

    /// Adds the accounts of `backup` that are not here yet; ones with the same
    /// id are the same account and stay as they are. OAuth apps are taken only
    /// if there is none for the service. Every entry is checked before
    /// anything is saved.
    pub fn import(&self, backup: Backup) -> Result<Imported> {
        let mut accounts = self.list()?;
        let mut new: Vec<(Account, String)> = Vec::new();
        let mut skipped = 0;
        for SavedAccount {
            mut account,
            secret,
        } in backup.accounts
        {
            if accounts
                .iter()
                .chain(new.iter().map(|(a, _)| a))
                .any(|a| a.id == account.id)
            {
                skipped += 1;
                continue;
            }
            let corrupted = || format!("The settings of account «{}» are corrupted", account.name);
            if account.kind == CRYPT_KIND {
                let folder: CryptFolder = serde_json::from_str(&secret).with_context(corrupted)?;
                account.crypt = Some(CryptLocation {
                    base: folder.base,
                    root: folder.root,
                });
            } else {
                let config: ProviderConfig =
                    serde_json::from_str(&secret).with_context(corrupted)?;
                if config.kind() != account.kind {
                    bail!(corrupted());
                }
            }
            if account.name.trim().is_empty() || account.id.is_empty() {
                bail!(corrupted());
            }
            new.push((account, secret));
        }

        let added = new.len();
        let mut written = Vec::new();
        let result = new.iter().try_for_each(|(account, secret)| {
            entry(&account.id)?
                .set_password(secret)
                .context("Failed to save credentials to the system keyring")?;
            written.push(account.id.as_str());
            Ok(())
        });
        let result = result.and_then(|()| {
            accounts.extend(new.iter().map(|(a, _)| a.clone()));
            self.save(&accounts)
        });
        if let Err(err) = result {
            for id in written {
                if let Ok(e) = entry(id) {
                    let _ = e.delete_credential();
                }
            }
            return Err(err);
        }
        for SavedOAuthApp { service, app } in backup.oauth_apps {
            if load_oauth_app(service).is_none() {
                save_oauth_app(service, &app)?;
            }
        }
        Ok(Imported { added, skipped })
    }
}

/// What an export of accounts holds, before it is sealed with a password.
#[derive(Serialize, Deserialize)]
pub struct Backup {
    accounts: Vec<SavedAccount>,
    #[serde(default)]
    oauth_apps: Vec<SavedOAuthApp>,
}

impl Backup {
    pub fn account_count(&self) -> usize {
        self.accounts.len()
    }
}

#[derive(Serialize, Deserialize)]
struct SavedAccount {
    #[serde(flatten)]
    account: Account,
    /// The keyring entry as is: `ProviderConfig` or `CryptFolder` in JSON.
    secret: String,
}

#[derive(Serialize, Deserialize)]
struct SavedOAuthApp {
    service: Service,
    app: OAuthApp,
}

#[derive(Serialize)]
pub struct Imported {
    pub added: usize,
    /// Accounts that are here already.
    pub skipped: usize,
}

/// The user's own OAuth app for `service`, so it is entered only once.
pub fn load_oauth_app(service: Service) -> Option<OAuthApp> {
    let secret = entry(&oauth_app_key(service)).ok()?.get_password().ok()?;
    serde_json::from_str(&secret).ok()
}

pub fn save_oauth_app(service: Service, app: &OAuthApp) -> Result<()> {
    entry(&oauth_app_key(service))?
        .set_password(&serde_json::to_string(app)?)
        .context("Failed to save the OAuth app to the system keyring")
}

fn oauth_app_key(service: Service) -> String {
    match service {
        Service::Google => "oauth-app-google".to_string(),
        Service::Yandex => "oauth-app-yandex".to_string(),
    }
}

fn secret(id: &str) -> Result<String> {
    entry(id)?
        .get_password()
        .context("Failed to read credentials from the system keyring")
}

fn entry(id: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, id).context("System keyring is unavailable")
}

fn new_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{nanos:x}")
}
