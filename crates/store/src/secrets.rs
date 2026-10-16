//! Where credentials are kept:
//! * the OS keyring on desktops (Windows Credential Manager, Keychain,
//!   Secret Service), shared by the GUI and the CLI on the same computer;
//! * a JSON file in the app's private folder on Android, which has no
//!   keyring the crate can use and where no other app can read the folder;
//! * memory, for accounts read from an export file.

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// The keyring service the GUI has always used.
#[cfg(feature = "keyring")]
pub const KEYRING_SERVICE: &str = "belka-gui";

pub enum Secrets {
    #[cfg(feature = "keyring")]
    Keyring,
    /// A JSON object id -> secret.
    File(PathBuf),
    Memory(Mutex<BTreeMap<String, String>>),
}

/// Serializes read-modify-write of a secrets file: the page and a
/// background backup may both use it.
static FILE_LOCK: Mutex<()> = Mutex::new(());

impl Secrets {
    /// The system keyring where there is one, else `fallback`.
    pub fn system_or(fallback: PathBuf) -> Self {
        #[cfg(feature = "keyring")]
        {
            let _ = fallback;
            Self::Keyring
        }
        #[cfg(not(feature = "keyring"))]
        Self::File(fallback)
    }

    pub fn memory() -> Self {
        Self::Memory(Mutex::new(BTreeMap::new()))
    }

    pub fn get(&self, id: &str) -> Result<Option<String>> {
        match self {
            #[cfg(feature = "keyring")]
            Self::Keyring => keyring_impl::get(id),
            Self::File(file) => {
                let _guard = FILE_LOCK.lock().unwrap();
                Ok(load(file)?.remove(id))
            }
            Self::Memory(map) => Ok(map.lock().unwrap().get(id).cloned()),
        }
    }

    pub fn set(&self, id: &str, secret: &str) -> Result<()> {
        match self {
            #[cfg(feature = "keyring")]
            Self::Keyring => keyring_impl::set(id, secret),
            Self::File(file) => {
                let _guard = FILE_LOCK.lock().unwrap();
                let mut secrets = load(file)?;
                secrets.insert(id.to_string(), secret.to_string());
                store(file, &secrets)
            }
            Self::Memory(map) => {
                map.lock()
                    .unwrap()
                    .insert(id.to_string(), secret.to_string());
                Ok(())
            }
        }
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        match self {
            #[cfg(feature = "keyring")]
            Self::Keyring => keyring_impl::delete(id),
            Self::File(file) => {
                let _guard = FILE_LOCK.lock().unwrap();
                let mut secrets = load(file)?;
                if secrets.remove(id).is_some() {
                    store(file, &secrets)?;
                }
                Ok(())
            }
            Self::Memory(map) => {
                map.lock().unwrap().remove(id);
                Ok(())
            }
        }
    }
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
    std::fs::write(&tmp, serde_json::to_string(secrets)?).context("Failed to save credentials")?;
    std::fs::rename(&tmp, file).context("Failed to save credentials")
}

#[cfg(feature = "keyring")]
mod keyring_impl {
    use super::KEYRING_SERVICE;
    use anyhow::{Context, Result};

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
}
