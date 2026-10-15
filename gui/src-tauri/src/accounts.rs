//! Saved cloud accounts. The public part (id, name, provider kind) is kept in
//! `accounts.json`; credentials are stored in the OS keyring (Windows
//! Credential Manager, macOS Keychain, Secret Service on Linux).

use anyhow::{bail, Context, Result};
use cloud_uploader_core::ProviderConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const KEYRING_SERVICE: &str = "cloud-uploader-gui";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub kind: String,
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
        let name = name.trim();
        if name.is_empty() {
            bail!("Account name is empty");
        }
        let mut accounts = self.list()?;
        let account = Account {
            id: new_id(),
            name: name.to_string(),
            kind: config.kind().to_string(),
        };
        entry(&account.id)?
            .set_password(&serde_json::to_string(config)?)
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
        let secret = entry(id)?
            .get_password()
            .context("Failed to read credentials from the system keyring")?;
        serde_json::from_str(&secret).context("Stored credentials are corrupted")
    }
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
