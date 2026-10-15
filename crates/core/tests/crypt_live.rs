//! Encrypted round trip through a real WebDAV server. Skipped unless
//! `BELKA_TEST_WEBDAV_URL` points to a disposable server, e.g.
//! `rclone serve webdav ./dav --addr 127.0.0.1:8099`. The file is left in
//! `/crypt-test` (password "password") to be checked with rclone.

use belka_core::crypt::CryptConfig;
use belka_core::storage::crypt::CryptProvider;
use belka_core::{Progress, ProviderConfig, StorageProvider};
use std::sync::Arc;

#[tokio::test]
async fn encrypted_upload_and_download() {
    let Ok(url) = std::env::var("BELKA_TEST_WEBDAV_URL") else {
        eprintln!("BELKA_TEST_WEBDAV_URL is not set, skipping");
        return;
    };
    let dav: Arc<dyn StorageProvider> = ProviderConfig::Webdav {
        url,
        user: std::env::var("BELKA_TEST_WEBDAV_USER").unwrap_or_default(),
        password: std::env::var("BELKA_TEST_WEBDAV_PASSWORD").unwrap_or_default(),
        etag_is_md5: false,
    }
    .build()
    .unwrap()
    .into();
    let config = CryptConfig {
        password: "password".into(),
        salt: String::new(),
        encrypt_names: true,
    };
    let provider = CryptProvider::new(dav, "/crypt-test", &config).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("big file.bin");
    let data: Vec<u8> = (0..3_000_000u32).map(|i| (i * 31 % 251) as u8).collect();
    std::fs::write(&local, &data).unwrap();

    provider.ensure_dir("/Папка").await.unwrap();
    let up = Progress::new();
    provider
        .upload_file(&local, "/Папка/big file.bin", &up)
        .await
        .unwrap();
    assert_eq!(up.done(), up.total());

    let listed = provider.list_dir("/Папка").await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (listed[0].name.as_str(), listed[0].size),
        ("big file.bin", 3_000_000)
    );

    let back = dir.path().join("back.bin");
    let size = provider
        .download_file("/Папка/big file.bin", &back, &Progress::new())
        .await
        .unwrap();
    assert_eq!(size, 3_000_000);
    assert_eq!(std::fs::read(&back).unwrap(), data);
}
