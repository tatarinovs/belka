//! Explicit opt-in round trips in freshly named folders of saved accounts.
//! Never uses the fixed /cu-test or /crypt-test folders of older live tests.
use anyhow::{Context, Result};
use belka_core::storage::copy::copy_file;
use belka_core::Progress;
use belka_store::accounts::{AccountSource, AccountStore};
use belka_store::secrets::Secrets;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "writes only its own temporary cloud folders; requires saved accounts and BELKA_LIVE_TEST=1"]
async fn saved_accounts_round_trip() -> Result<()> {
    anyhow::ensure!(
        std::env::var("BELKA_LIVE_TEST").as_deref() == Ok("1"),
        "Set BELKA_LIVE_TEST=1"
    );
    let dir = belka_store::default_dir().context("No config directory")?;
    let store = AccountStore::new(&dir, Secrets::system_or(dir.join("secrets.json")));
    let accounts = store.accounts()?;
    anyhow::ensure!(!accounts.is_empty(), "No saved accounts");
    let local = tempfile::tempdir()?;
    let data: Vec<_> = (0..131_072u32).map(|n| (n % 251) as u8).collect();
    let source = local.path().join("source.bin");
    std::fs::write(&source, &data)?;
    let mut failures = Vec::new();
    for account in accounts {
        if std::env::var("BELKA_LIVE_ACCOUNT_KIND").is_ok_and(|kind| kind != account.kind) {
            continue;
        }
        let provider = store.provider(&account.id)?;
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let name = format!("belka-review-{stamp}-{}", std::process::id());
        let base = if account.kind == "synology" {
            let entries =
                tokio::time::timeout(Duration::from_secs(30), provider.list_dir("/")).await??;
            entries
                .into_iter()
                .find(|e| e.is_dir)
                .context("No NAS shared folder")?
                .path
        } else {
            String::new()
        };
        let root = format!("{base}/{name}");
        let test = async {
            println!("{}: create temporary folder", account.kind);
            provider.ensure_dir(&root).await?;
            let path = format!("{root}/round trip.bin");
            if provider.supports_create_only() {
                provider
                    .upload_new_file(&source, &path, &Progress::new())
                    .await?;
            } else {
                provider
                    .upload_file(&source, &path, &Progress::new())
                    .await?;
            }
            println!("{}: upload complete, listing", account.kind);
            let listing = provider.list_dir(&root).await?;
            let file = listing
                .iter()
                .find(|f| f.path == path)
                .context("Uploaded file not listed")?;
            anyhow::ensure!(file.size == data.len() as u64, "Wrong listed size");
            // Drive creates distinct objects of the same name; ambiguity is
            // rejected separately instead of overwriting either object.
            if account.kind != "google" && account.kind != "crypt" {
                let conflict = provider
                    .upload_new_file(&source, &path, &Progress::new())
                    .await;
                anyhow::ensure!(
                    conflict.is_err(),
                    "create-only write unexpectedly overwrote an existing file"
                );
            }
            if !provider.supports_create_only() {
                use belka_core::sync::{self, Filter, Index, UploadOptions};
                use std::sync::Mutex;
                println!("{}: unique-name backups", account.kind);
                let tree = sync::scan(&source, &Filter::all())?;
                let opts = UploadOptions {
                    keep_both: true,
                    ..UploadOptions::default()
                };
                let a = Mutex::new(Index::load(
                    local.path().join(format!("fallback-{stamp}-a.json")),
                    "a",
                ));
                let b = Mutex::new(Index::load(
                    local.path().join(format!("fallback-{stamp}-b.json")),
                    "b",
                ));
                let target = format!("{root}/shared-backup");
                for index in [&a, &b] {
                    let summary =
                        sync::upload(&*provider, &tree, &target, Some(index), &opts, &()).await?;
                    anyhow::ensure!(
                        summary.failed == 0 && summary.transferred == 1,
                        "Unique-name backup failed: {summary:?}"
                    );
                }
                let first = a.lock().unwrap().cloud_path("source.bin").to_string();
                let second = b.lock().unwrap().cloud_path("source.bin").to_string();
                anyhow::ensure!(
                    first != second,
                    "Independent backup indexes picked the same name"
                );
                let files = provider.list_dir(&target).await?;
                anyhow::ensure!(files.len() == 2, "Shared backup lost a file");
                a.lock().unwrap().save()?;
                let reloaded = Mutex::new(Index::load(
                    local.path().join(format!("fallback-{stamp}-a.json")),
                    "a",
                ));
                let summary =
                    sync::upload(&*provider, &tree, &target, Some(&reloaded), &opts, &()).await?;
                anyhow::ensure!(
                    summary.unchanged == 1,
                    "Backup index did not preserve the unique name"
                );
            }
            println!("{}: download round trip", account.kind);
            let dest = local.path().join(format!("back-{}.bin", account.id));
            provider
                .download_file(&path, &dest, &Progress::new())
                .await?;
            anyhow::ensure!(std::fs::read(&dest)? == data, "Downloaded content differs");
            println!("{}: server copy", account.kind);
            copy_file(
                &provider,
                file,
                &*provider,
                &format!("{root}/copy.bin"),
                true,
                &local.path().join(format!("copy-{}", account.id)),
                &Progress::new(),
            )
            .await?;
            anyhow::ensure!(provider.get_file_info(&path).await?.is_some(), "The copy removed its source");
            println!("{}: rename", account.kind);
            provider.rename(&path, "renamed.bin", false).await?;
            anyhow::ensure!(
                provider
                    .get_file_info(&format!("{root}/renamed.bin"))
                    .await?
                    .is_some(),
                "Renamed file missing"
            );
            Ok::<_, anyhow::Error>(())
        };
        let result = tokio::time::timeout(Duration::from_secs(180), test).await;
        // Only the freshly generated root can be removed by this test.
        anyhow::ensure!(
            root.ends_with(&format!("/{name}")) && name.starts_with("belka-review-"),
            "Invalid cleanup root"
        );
        let cleanup =
            tokio::time::timeout(Duration::from_secs(60), provider.delete(&root, true)).await;
        match result {
            Ok(Ok(())) => println!(
                "{}: upload/list/create-only/download/copy/rename OK",
                account.kind
            ),
            other => failures.push(format!("{}: {other:?}", account.kind)),
        }
        if !matches!(cleanup, Ok(Ok(()))) {
            failures.push(format!(
                "{}: cleanup failed at {root}: {cleanup:?}",
                account.kind
            ));
        }
    }
    anyhow::ensure!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}
