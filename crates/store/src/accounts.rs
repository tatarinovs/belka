//! Saved cloud accounts. The public part (id, name, provider kind) is kept in
//! `accounts.json`; credentials are kept by [`Secrets`].
//!
//! An encrypted folder is an account of kind [`CRYPT_KIND`] that refers to
//! another account and keeps its password with the other credentials.
//!
//! All of it can be exported into one file sealed with a password
//! ([`belka_core::crypt::seal`]) and imported on another computer, or used
//! by the CLI as it is ([`Backup::open`]).

use crate::secrets::Secrets;
use anyhow::{bail, Context, Result};
use belka_core::crypt::CryptConfig;
use belka_core::oauth::{OAuthApp, Service};
use belka_core::storage::crypt::CryptProvider;
use belka_core::{ProviderConfig, StorageProvider};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

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

/// Anything that has accounts with their credentials: the saved accounts
/// or an export file.
pub trait AccountSource {
    fn accounts(&self) -> Result<Vec<Account>>;
    /// The stored secret: `ProviderConfig` or `CryptFolder` in JSON.
    fn secret(&self, id: &str) -> Result<String>;

    fn account(&self, id: &str) -> Result<Account> {
        self.accounts()?
            .into_iter()
            .find(|a| a.id == id)
            .context("Account not found")
    }

    /// An account by its name (any case) or id.
    fn find(&self, name_or_id: &str) -> Result<Account> {
        let accounts = self.accounts()?;
        let wanted = name_or_id.trim();
        accounts
            .iter()
            .find(|a| a.id == wanted)
            .or_else(|| accounts.iter().find(|a| a.name == wanted))
            .or_else(|| {
                accounts
                    .iter()
                    .find(|a| a.name.to_lowercase() == wanted.to_lowercase())
            })
            .cloned()
            .with_context(|| {
                let names: Vec<String> = accounts.iter().map(|a| format!("'{}'", a.name)).collect();
                if names.is_empty() {
                    format!("No account '{wanted}': there are no accounts")
                } else {
                    format!("No account '{wanted}'; accounts: {}", names.join(", "))
                }
            })
    }

    fn credentials(&self, id: &str) -> Result<ProviderConfig> {
        serde_json::from_str(&self.secret(id)?).context("Stored credentials are corrupted")
    }

    fn crypt_folder(&self, id: &str) -> Result<CryptFolder> {
        serde_json::from_str(&self.secret(id)?).context("Stored encryption settings are corrupted")
    }

    /// Builds the provider of an account; an encrypted folder wraps the
    /// provider of the account it is stored in. Key derivation of an
    /// encrypted folder takes a moment by design.
    fn provider(&self, id: &str) -> Result<Arc<dyn StorageProvider>> {
        if self.account(id)?.kind == CRYPT_KIND {
            let folder = self.crypt_folder(id)?;
            if self.account(&folder.base)?.kind == CRYPT_KIND {
                bail!("An encrypted folder cannot be placed inside another one");
            }
            let base = self.provider(&folder.base)?;
            Ok(Arc::new(CryptProvider::new(
                base,
                &folder.root,
                &folder.config,
            )?))
        } else {
            Ok(self.credentials(id)?.build()?.into())
        }
    }
}

pub struct AccountStore {
    file: PathBuf,
    secrets: Secrets,
}

impl AccountStore {
    pub fn new(config_dir: &Path, secrets: Secrets) -> Self {
        Self {
            file: config_dir.join("accounts.json"),
            secrets,
        }
    }

    pub fn secrets(&self) -> &Secrets {
        &self.secrets
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
        crate::write_json(&self.file, &accounts)
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
        self.secrets.set(&account.id, secret)?;
        accounts.push(account.clone());
        if let Err(err) = self.save(&accounts) {
            let _ = self.secrets.delete(&account.id);
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
            self.secrets.set(id, &serde_json::to_string(config)?)?;
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
        self.secrets.delete(id)
    }

    /// Names of the encrypted folders stored in account `id`.
    pub fn crypt_folders_in(&self, id: &str) -> Vec<String> {
        self.list()
            .unwrap_or_default()
            .into_iter()
            .filter(|a| a.kind == CRYPT_KIND)
            .filter(|a| self.crypt_folder(&a.id).is_ok_and(|f| f.base == id))
            .map(|a| a.name)
            .collect()
    }

    /// All accounts with their credentials, and the user's own OAuth apps.
    pub fn export(&self) -> Result<Backup> {
        let accounts = self
            .list()?
            .into_iter()
            .map(|account| {
                let secret = self
                    .secret(&account.id)
                    .with_context(|| format!("Account «{}»", account.name))?;
                Ok(SavedAccount { account, secret })
            })
            .collect::<Result<_>>()?;
        let oauth_apps = [Service::Google, Service::Yandex]
            .into_iter()
            .filter_map(|service| {
                self.load_oauth_app(service)
                    .map(|app| SavedOAuthApp { service, app })
            })
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
        backup.check()?;
        let mut accounts = self.list()?;
        let mut new: Vec<&SavedAccount> = Vec::new();
        let mut skipped = 0;
        for saved in &backup.accounts {
            if accounts
                .iter()
                .chain(new.iter().map(|s| &s.account))
                .any(|a| a.id == saved.account.id)
            {
                skipped += 1;
                continue;
            }
            new.push(saved);
        }

        let added = new.len();
        let mut written = Vec::new();
        let result = new.iter().try_for_each(|saved| {
            self.secrets.set(&saved.account.id, &saved.secret)?;
            written.push(saved.account.id.as_str());
            Ok(())
        });
        let result = result.and_then(|()| {
            accounts.extend(new.iter().map(|s| s.account.clone()));
            self.save(&accounts)
        });
        if let Err(err) = result {
            for id in written {
                let _ = self.secrets.delete(id);
            }
            return Err(err);
        }
        for SavedOAuthApp { service, app } in &backup.oauth_apps {
            if self.load_oauth_app(*service).is_none() {
                self.save_oauth_app(*service, app)?;
            }
        }
        Ok(Imported { added, skipped })
    }

    /// The user's own OAuth app for `service`, so it is entered only once.
    pub fn load_oauth_app(&self, service: Service) -> Option<OAuthApp> {
        let secret = self.secrets.get(&oauth_app_key(service)).ok()??;
        serde_json::from_str(&secret).ok()
    }

    pub fn save_oauth_app(&self, service: Service, app: &OAuthApp) -> Result<()> {
        self.secrets
            .set(&oauth_app_key(service), &serde_json::to_string(app)?)
    }
}

impl AccountSource for AccountStore {
    fn accounts(&self) -> Result<Vec<Account>> {
        self.list()
    }

    fn secret(&self, id: &str) -> Result<String> {
        self.secrets
            .get(id)?
            .context("Credentials are missing; sign in again")
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

    /// Seals the accounts with `password` into the bytes of a `.belka` file.
    /// Key derivation is deliberately slow.
    pub fn seal(&self, password: &str) -> Result<Vec<u8>> {
        belka_core::crypt::seal(password, &serde_json::to_vec(self)?)
    }

    /// Opens the bytes of a `.belka` file.
    pub fn open(file: &[u8], password: &str) -> Result<Self> {
        let data = belka_core::crypt::unseal(password, file)?;
        let mut backup: Self = serde_json::from_slice(&data).context("The file is damaged")?;
        backup.check()?;
        for saved in &mut backup.accounts {
            if saved.account.kind == CRYPT_KIND {
                let folder: CryptFolder = serde_json::from_str(&saved.secret)?;
                saved.account.crypt = Some(CryptLocation {
                    base: folder.base,
                    root: folder.root,
                });
            }
        }
        Ok(backup)
    }

    /// Every entry must be readable before anything is used.
    fn check(&self) -> Result<()> {
        for SavedAccount { account, secret } in &self.accounts {
            let corrupted = || format!("The settings of account «{}» are corrupted", account.name);
            if account.name.trim().is_empty() || account.id.is_empty() {
                bail!(corrupted());
            }
            if account.kind == CRYPT_KIND {
                serde_json::from_str::<CryptFolder>(secret).with_context(corrupted)?;
            } else {
                let config: ProviderConfig =
                    serde_json::from_str(secret).with_context(corrupted)?;
                if config.kind() != account.kind {
                    bail!(corrupted());
                }
            }
        }
        Ok(())
    }
}

impl AccountSource for Backup {
    fn accounts(&self) -> Result<Vec<Account>> {
        Ok(self.accounts.iter().map(|s| s.account.clone()).collect())
    }

    fn secret(&self, id: &str) -> Result<String> {
        self.accounts
            .iter()
            .find(|s| s.account.id == id)
            .map(|s| s.secret.clone())
            .context("Account not found")
    }
}

#[derive(Serialize, Deserialize)]
struct SavedAccount {
    #[serde(flatten)]
    account: Account,
    /// The stored secret as is: `ProviderConfig` or `CryptFolder` in JSON.
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

fn oauth_app_key(service: Service) -> String {
    match service {
        Service::Google => "oauth-app-google".to_string(),
        Service::Yandex => "oauth-app-yandex".to_string(),
    }
}

fn new_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{nanos:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn webdav(url: &str) -> ProviderConfig {
        ProviderConfig::Webdav {
            url: url.into(),
            user: "u".into(),
            password: "p".into(),
            etag_is_md5: false,
        }
    }

    #[test]
    fn export_round_trip_and_lookup_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let store = AccountStore::new(dir.path(), Secrets::memory());
        let a = store.add("Яндекс", &webdav("https://a")).unwrap();
        store.add("NAS", &webdav("https://b")).unwrap();
        assert_eq!(store.find("яндекс").unwrap().id, a.id);
        assert_eq!(store.find(&a.id).unwrap().name, "Яндекс");
        assert!(store
            .find("nope")
            .unwrap_err()
            .to_string()
            .contains("'NAS'"));

        let sealed = store.export().unwrap().seal("pw").unwrap();
        let backup = Backup::open(&sealed, "pw").unwrap();
        assert_eq!(backup.account_count(), 2);
        assert!(matches!(
            backup.credentials(&backup.find("nas").unwrap().id).unwrap(),
            ProviderConfig::Webdav { url, .. } if url == "https://b"
        ));
        assert!(Backup::open(&sealed, "wrong").is_err());

        let other = tempfile::tempdir().unwrap();
        let copy = AccountStore::new(other.path(), Secrets::memory());
        let imported = copy.import(Backup::open(&sealed, "pw").unwrap()).unwrap();
        assert_eq!((imported.added, imported.skipped), (2, 0));
        let again = copy.import(Backup::open(&sealed, "pw").unwrap()).unwrap();
        assert_eq!((again.added, again.skipped), (0, 2));
    }
}
