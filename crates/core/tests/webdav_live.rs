//! Round trip through a real WebDAV server. Skipped unless
//! `BELKA_TEST_WEBDAV_URL` points to a disposable server, e.g.
//! `wsgidav --host 127.0.0.1 --port 8765 --root ./dav --auth anonymous`.

use belka_core::{Progress, ProviderConfig};

#[tokio::test]
async fn upload_list_download_delete() {
    let Ok(url) = std::env::var("BELKA_TEST_WEBDAV_URL") else {
        eprintln!("BELKA_TEST_WEBDAV_URL is not set, skipping");
        return;
    };
    let provider = ProviderConfig::Webdav {
        url,
        user: std::env::var("BELKA_TEST_WEBDAV_USER").unwrap_or_default(),
        password: std::env::var("BELKA_TEST_WEBDAV_PASSWORD").unwrap_or_default(),
        etag_is_md5: false,
    }
    .build()
    .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("big file.bin");
    let data: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&local, &data).unwrap();

    let remote_dir = "/cu-test/вложенная папка";
    let remote = format!("{remote_dir}/big file.bin");
    provider.ensure_dir(remote_dir).await.unwrap();

    let up = Progress::new();
    provider.upload_file(&local, &remote, &up).await.unwrap();
    assert_eq!(
        (up.done(), up.total()),
        (data.len() as u64, data.len() as u64)
    );

    let listing = provider.list_dir(remote_dir).await.unwrap();
    let entry = listing
        .iter()
        .find(|e| e.path == remote)
        .expect("uploaded file listed");
    assert_eq!(entry.size, data.len() as u64);

    let back = dir.path().join("back.bin");
    let down = Progress::new();
    let size = provider.download_file(&remote, &back, &down).await.unwrap();
    assert_eq!(size, data.len() as u64);
    assert_eq!(down.done(), size);
    assert_eq!(std::fs::read(&back).unwrap(), data);

    let missing = provider
        .download_file(
            "/cu-test/missing.bin",
            &dir.path().join("x"),
            &Progress::new(),
        )
        .await;
    assert!(missing.is_err());
    assert!(!dir.path().join("x").exists());
    assert!(!dir.path().join("x.part").exists());

    // Renaming a file, refusing to overwrite, and renaming a folder.
    provider
        .rename(&remote, "renamed.bin", false)
        .await
        .unwrap();
    std::fs::write(dir.path().join("other.bin"), b"x").unwrap();
    let other = format!("{remote_dir}/other.bin");
    provider
        .upload_file(&dir.path().join("other.bin"), &other, &Progress::new())
        .await
        .unwrap();
    assert!(provider.rename(&other, "renamed.bin", false).await.is_err());
    provider
        .rename(remote_dir, "переименована", true)
        .await
        .unwrap();
    let names: Vec<String> = provider
        .list_dir("/cu-test/переименована")
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert!(names.contains(&"renamed.bin".to_string()), "{names:?}");
    assert!(provider.list_dir(remote_dir).await.unwrap().is_empty());

    provider.delete("/cu-test", true).await.unwrap();
    assert!(provider.list_dir("/cu-test").await.unwrap().is_empty());
}
