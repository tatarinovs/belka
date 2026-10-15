//! Credentials storage. On desktops it is the OS keyring (Windows Credential
//! Manager, macOS Keychain, Secret Service on Linux). Android has no keyring
//! the crate can use, so there the secrets are kept in a JSON file in the
//! app's private folder, which no other app can read.

#[cfg(desktop)]
use anyhow::Context;
use anyhow::Result;

#[cfg(desktop)]
mod imp {
    use super::*;

    const KEYRING_SERVICE: &str = "belka-gui";

    fn entry(id: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(KEYRING_SERVICE, id).context("System keyring is unavailable")
    }

    pub fn get(id: &str) -> Result<Option<String>> {
        match entry(id)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(err).context("Failed to read credentials from the system keyring"),
        }
    }

    pub fn set(id: &str, secret: &str) -> Result<()> {
        entry(id)?
            .set_password(secret)
            .context("Failed to save credentials to the system keyring")
    }

    pub fn delete(id: &str) -> Result<()> {
        match entry(id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err).context("Failed to delete credentials from the system keyring"),
        }
    }

    pub fn init(_dir: std::path::PathBuf) {}
}

#[cfg(mobile)]
mod imp {
    use super::*;
    use anyhow::Context;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};

    static FILE: OnceLock<PathBuf> = OnceLock::new();
    /// Serializes read-modify-write of the file.
    static LOCK: Mutex<()> = Mutex::new(());

    pub fn init(dir: PathBuf) {
        let _ = FILE.set(dir.join("secrets.json"));
    }

    fn file() -> Result<&'static PathBuf> {
        FILE.get().context("Credentials storage is not initialized")
    }

    fn load(file: &PathBuf) -> Result<BTreeMap<String, String>> {
        match std::fs::read_to_string(file) {
            Ok(text) => serde_json::from_str(&text).context("Stored credentials are corrupted"),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(err) => Err(err).context("Failed to read credentials"),
        }
    }

    fn store(file: &PathBuf, secrets: &BTreeMap<String, String>) -> Result<()> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).context("Failed to save credentials")?;
        }
        let tmp = file.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string(secrets)?)
            .context("Failed to save credentials")?;
        std::fs::rename(&tmp, file).context("Failed to save credentials")
    }

    pub fn get(id: &str) -> Result<Option<String>> {
        let _guard = LOCK.lock().unwrap();
        Ok(load(file()?)?.remove(id))
    }

    pub fn set(id: &str, secret: &str) -> Result<()> {
        let _guard = LOCK.lock().unwrap();
        let file = file()?;
        let mut secrets = load(file)?;
        secrets.insert(id.to_string(), secret.to_string());
        store(file, &secrets)
    }

    pub fn delete(id: &str) -> Result<()> {
        let _guard = LOCK.lock().unwrap();
        let file = file()?;
        let mut secrets = load(file)?;
        if secrets.remove(id).is_some() {
            store(file, &secrets)?;
        }
        Ok(())
    }
}

pub use imp::{delete, get, init, set};
