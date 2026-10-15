//! A folder of another provider whose content (and optionally names) is
//! encrypted, like an rclone crypt remote. Data is encrypted while it is
//! uploaded and decrypted while it is downloaded, without temporary files.

use super::{check_name, RemoteFileInfo, StorageProvider};
use crate::client::{MemorySink, Sink, Source};
use crate::crypt::{decrypted_size, Cipher, CryptConfig, DecryptSink, EncryptedSource};
use crate::progress::Progress;
use crate::utils::path::{join_remote, normalize_remote_dir, split_remote};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::warn;

pub struct CryptProvider {
    inner: Arc<dyn StorageProvider>,
    /// The folder of `inner` that holds the encrypted tree.
    root: String,
    cipher: Arc<Cipher>,
    name: &'static str,
}

impl CryptProvider {
    /// Key derivation takes a moment by design, so build once and reuse.
    pub fn new(inner: Arc<dyn StorageProvider>, root: &str, config: &CryptConfig) -> Result<Self> {
        // Built once per account or run, so leaking the name is fine.
        let name = Box::leak(format!("{} (encrypted)", inner.name()).into_boxed_str());
        Ok(Self {
            inner,
            name,
            root: normalize_remote_dir(root),
            cipher: Arc::new(Cipher::new(config)?),
        })
    }

    fn inner_dir(&self, path: &str) -> String {
        normalize_remote_dir(path)
            .split('/')
            .filter(|s| !s.is_empty())
            .fold(self.root.clone(), |acc, segment| {
                join_remote(&acc, &self.cipher.encrypt_dir_name(segment))
            })
    }

    fn inner_file(&self, path: &str) -> String {
        let (parent, name) = split_remote(&normalize_remote_dir(path));
        join_remote(
            &self.inner_dir(&parent),
            &self.cipher.encrypt_file_name(&name),
        )
    }

    fn inner_path(&self, path: &str, is_dir: bool) -> String {
        if is_dir {
            self.inner_dir(path)
        } else {
            self.inner_file(path)
        }
    }

    /// Maps an item of `inner` listed in folder `dir`; `None` if it is not
    /// encrypted with this key.
    fn decrypt_info(&self, dir: &str, item: RemoteFileInfo) -> Option<RemoteFileInfo> {
        let (name, size) = if item.is_dir {
            (self.cipher.decrypt_dir_name(&item.name)?, 0)
        } else {
            (
                self.cipher.decrypt_file_name(&item.name)?,
                decrypted_size(item.size)?,
            )
        };
        Some(RemoteFileInfo {
            path: join_remote(dir, &name),
            name,
            is_dir: item.is_dir,
            size,
            md5: None,
            modified: item.modified,
        })
    }

    /// Fails if the root holds items but none of them can be decrypted:
    /// most likely the password (or the name setting) is wrong.
    pub async fn verify_password(&self) -> Result<()> {
        let items = self.inner.list_dir(&self.root).await?;
        if items.is_empty() {
            return Ok(());
        }
        let readable = items
            .into_iter()
            .any(|item| self.decrypt_info("/", item).is_some());
        if !readable {
            bail!(
                "'{}' contains files that cannot be decrypted with this password. \
                 Check the password, or choose an empty folder",
                self.root
            );
        }
        Ok(())
    }
}

#[async_trait]
impl StorageProvider for CryptProvider {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn check_access(&self, remote_dir: &str) -> Result<()> {
        self.inner.check_access(&self.inner_dir(remote_dir)).await
    }

    async fn ensure_dir(&self, remote_dir: &str) -> Result<()> {
        self.inner.ensure_dir(&self.inner_dir(remote_dir)).await
    }

    async fn get_file_info(&self, remote_path: &str) -> Result<Option<RemoteFileInfo>> {
        let path = normalize_remote_dir(remote_path);
        let (parent, _) = split_remote(&path);
        if path == "/" {
            return Ok(Some(RemoteFileInfo {
                name: String::new(),
                path,
                is_dir: true,
                size: 0,
                md5: None,
                modified: None,
            }));
        }
        let file = self.inner_file(&path);
        let dir = self.inner_dir(&path);
        let mut found = self.inner.get_file_info(&file).await?;
        if found.as_ref().is_some_and(|f| f.is_dir) && file != dir {
            found = None;
        }
        if found.is_none() && file != dir {
            found = self.inner.get_file_info(&dir).await?.filter(|f| f.is_dir);
        }
        Ok(found.and_then(|item| self.decrypt_info(&parent, item)))
    }

    async fn upload(
        &self,
        source: &dyn Source,
        remote_path: &str,
        progress: &Progress,
    ) -> Result<()> {
        let encrypted = EncryptedSource::new(source, self.cipher.clone())?;
        self.inner
            .upload(&encrypted, &self.inner_file(remote_path), progress)
            .await
    }

    async fn download_to(
        &self,
        remote_path: &str,
        sink: &mut dyn Sink,
        progress: &Progress,
    ) -> Result<u64> {
        let mut decrypted = DecryptSink::new(self.cipher.clone(), sink);
        self.inner
            .download_to(&self.inner_file(remote_path), &mut decrypted, progress)
            .await
            .with_context(|| format!("Failed to download '{remote_path}'"))?;
        decrypted
            .finish()
            .await
            .with_context(|| format!("Failed to decrypt '{remote_path}'"))
    }

    async fn delete(&self, remote_path: &str, is_dir: bool) -> Result<()> {
        if normalize_remote_dir(remote_path) == "/" {
            bail!("Refusing to delete the root folder");
        }
        self.inner
            .delete(&self.inner_path(remote_path, is_dir), is_dir)
            .await
    }

    async fn rename(&self, remote_path: &str, new_name: &str, is_dir: bool) -> Result<()> {
        let new_name = check_name(new_name)?;
        let encrypted = if is_dir {
            self.cipher.encrypt_dir_name(new_name)
        } else {
            self.cipher.encrypt_file_name(new_name)
        };
        self.inner
            .rename(&self.inner_path(remote_path, is_dir), &encrypted, is_dir)
            .await
            .map_err(|err| {
                // Errors would otherwise quote the encrypted name.
                if format!("{err:#}").contains("already exists") {
                    anyhow::anyhow!("'{new_name}' already exists")
                } else {
                    err
                }
            })
    }

    async fn read_text_file(&self, remote_path: &str) -> Result<Option<String>> {
        let file = self.inner_file(remote_path);
        if self.inner.get_file_info(&file).await?.is_none() {
            return Ok(None);
        }
        let mut data = MemorySink::default();
        self.download_to(remote_path, &mut data, &Progress::new())
            .await?;
        String::from_utf8(data.0)
            .map(Some)
            .with_context(|| format!("'{remote_path}' is not a text file"))
    }

    async fn list_dir(&self, remote_dir: &str) -> Result<Vec<RemoteFileInfo>> {
        let dir = normalize_remote_dir(remote_dir);
        let items = self.inner.list_dir(&self.inner_dir(&dir)).await?;
        let mut skipped = 0;
        let result: Vec<_> = items
            .into_iter()
            .filter_map(|item| {
                let decrypted = self.decrypt_info(&dir, item);
                skipped += usize::from(decrypted.is_none());
                decrypted
            })
            .collect();
        if skipped > 0 {
            warn!("Skipped {skipped} item(s) in '{dir}' that are not encrypted with this key");
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use tokio::io::AsyncReadExt;

    /// A provider over a local folder, standing in for a cloud.
    struct LocalDir(PathBuf);

    impl LocalDir {
        fn at(&self, path: &str) -> PathBuf {
            self.0
                .join(normalize_remote_dir(path).trim_start_matches('/'))
        }

        fn info(&self, path: &str) -> Option<RemoteFileInfo> {
            let meta = std::fs::metadata(self.at(path)).ok()?;
            let path = normalize_remote_dir(path);
            Some(RemoteFileInfo {
                name: split_remote(&path).1,
                path,
                is_dir: meta.is_dir(),
                size: if meta.is_dir() { 0 } else { meta.len() },
                md5: None,
                modified: None,
            })
        }
    }

    #[async_trait]
    impl StorageProvider for LocalDir {
        fn name(&self) -> &'static str {
            "local"
        }
        async fn ensure_dir(&self, dir: &str) -> Result<()> {
            Ok(std::fs::create_dir_all(self.at(dir))?)
        }
        async fn get_file_info(&self, path: &str) -> Result<Option<RemoteFileInfo>> {
            Ok(self.info(path))
        }
        async fn upload(&self, source: &dyn Source, remote: &str, _: &Progress) -> Result<()> {
            let mut data = Vec::new();
            source.open_at(0)?.read_to_end(&mut data).await?;
            std::fs::write(self.at(remote), data)?;
            Ok(())
        }
        async fn download_to(
            &self,
            remote: &str,
            sink: &mut dyn Sink,
            _: &Progress,
        ) -> Result<u64> {
            let data = std::fs::read(self.at(remote))?;
            sink.reset().await?;
            // Small pieces exercise blocks that span several writes.
            for chunk in data.chunks(1000) {
                sink.write(chunk).await?;
            }
            Ok(data.len() as u64)
        }
        async fn delete(&self, path: &str, is_dir: bool) -> Result<()> {
            if is_dir {
                std::fs::remove_dir_all(self.at(path))?;
            } else {
                std::fs::remove_file(self.at(path))?;
            }
            Ok(())
        }
        async fn rename(&self, path: &str, new_name: &str, _: bool) -> Result<()> {
            let from = self.at(path);
            std::fs::rename(&from, from.with_file_name(new_name))?;
            Ok(())
        }
        async fn read_text_file(&self, path: &str) -> Result<Option<String>> {
            Ok(std::fs::read_to_string(self.at(path)).ok())
        }
        async fn list_dir(&self, dir: &str) -> Result<Vec<RemoteFileInfo>> {
            let Ok(entries) = std::fs::read_dir(self.at(dir)) else {
                return Ok(Vec::new());
            };
            Ok(entries
                .filter_map(|e| {
                    let name = e.ok()?.file_name().to_string_lossy().to_string();
                    self.info(&join_remote(&normalize_remote_dir(dir), &name))
                })
                .collect())
        }
    }

    fn crypt(store: &Path, encrypt_names: bool) -> CryptProvider {
        let config = CryptConfig {
            password: "password".into(),
            salt: String::new(),
            encrypt_names,
        };
        CryptProvider::new(Arc::new(LocalDir(store.to_owned())), "/Encrypted", &config).unwrap()
    }

    /// Fills `store` with an encrypted tree: `/Docs/Отчёт.txt` and `/a.bin`.
    async fn fill(provider: &CryptProvider, work: &Path) {
        let plain = work.join("plain.txt");
        std::fs::write(&plain, "secret report").unwrap();
        provider.ensure_dir("/Docs").await.unwrap();
        let p = Progress::new();
        provider
            .upload_file(&plain, "/Docs/Отчёт.txt", &p)
            .await
            .unwrap();
        std::fs::write(&plain, vec![7u8; 70_000]).unwrap();
        provider.upload_file(&plain, "/a.bin", &p).await.unwrap();
    }

    #[tokio::test]
    async fn stores_files_encrypted_and_reads_them_back() {
        for encrypt_names in [true, false] {
            let (store, work) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
            let provider = crypt(store.path(), encrypt_names);
            fill(&provider, work.path()).await;

            let mut root: Vec<_> = provider.list_dir("/").await.unwrap();
            root.sort_by(|a, b| a.name.cmp(&b.name));
            let names: Vec<_> = root.iter().map(|i| (i.name.as_str(), i.size)).collect();
            assert_eq!(names, [("Docs", 0), ("a.bin", 70_000)]);

            let report = provider.list_dir("/Docs").await.unwrap();
            assert_eq!(report[0].path, "/Docs/Отчёт.txt");
            assert_eq!(
                provider
                    .read_text_file("/Docs/Отчёт.txt")
                    .await
                    .unwrap()
                    .as_deref(),
                Some("secret report")
            );
            let info = provider
                .get_file_info("/Docs/Отчёт.txt")
                .await
                .unwrap()
                .unwrap();
            assert!(!info.is_dir);
            assert!(
                provider
                    .get_file_info("/Docs")
                    .await
                    .unwrap()
                    .unwrap()
                    .is_dir
            );

            // Nothing readable is stored, and names are hidden when asked to.
            let raw = std::fs::read_dir(store.path().join("Encrypted")).unwrap();
            let raw: Vec<_> = raw
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            assert_eq!(raw.iter().any(|n| n == "Docs"), !encrypt_names);

            let out = work.path().join("out.bin");
            let size = provider
                .download_file("/a.bin", &out, &Progress::new())
                .await
                .unwrap();
            assert_eq!(size, 70_000);
            assert_eq!(std::fs::read(&out).unwrap(), vec![7u8; 70_000]);

            provider
                .rename("/Docs/Отчёт.txt", "Итог.txt", false)
                .await
                .unwrap();
            let renamed = provider.list_dir("/Docs").await.unwrap();
            assert_eq!(renamed[0].name, "Итог.txt");
            provider.delete("/Docs", true).await.unwrap();
            assert_eq!(provider.list_dir("/").await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn wrong_password_is_reported() {
        let (store, work) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let provider = crypt(store.path(), true);
        provider.verify_password().await.unwrap();
        fill(&provider, work.path()).await;
        provider.verify_password().await.unwrap();

        let other = CryptConfig {
            password: "other".into(),
            salt: String::new(),
            encrypt_names: true,
        };
        let wrong = CryptProvider::new(
            Arc::new(LocalDir(store.path().to_owned())),
            "/Encrypted",
            &other,
        )
        .unwrap();
        assert!(wrong.verify_password().await.is_err());
        assert!(wrong.list_dir("/").await.unwrap().is_empty());
    }

    /// Leaves an encrypted tree in `$BELKA_CRYPT_OUT` to check with rclone:
    /// `cargo test -p belka-core write_tree_for_rclone -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn write_tree_for_rclone() {
        let out = PathBuf::from(std::env::var("BELKA_CRYPT_OUT").unwrap());
        let work = tempfile::tempdir().unwrap();
        fill(&crypt(&out, true), work.path()).await;
    }
}
