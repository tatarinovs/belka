//! `belka ls SOURCE:PATH`: what is in a cloud folder. Also the quickest way
//! to check that a source is set up right.

use anyhow::{Context, Result};
use belka_core::utils::format_bytes;
use belka_core::utils::path::normalize_remote_dir;
use belka_core::{RemoteFileInfo, StorageProvider};

pub async fn run(provider: &dyn StorageProvider, path: &str, recursive: bool) -> Result<()> {
    let root = normalize_remote_dir(path);
    let mut pending = vec![(root.clone(), String::new())];
    let mut count = 0usize;
    while let Some((dir, prefix)) = pending.pop() {
        let mut items = provider
            .list_dir(&dir)
            .await
            .with_context(|| format!("Cannot list '{dir}' on {}", provider.name()))?;
        items.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
        let mut subdirs = Vec::new();
        for item in &items {
            println!("{}", line(item, &prefix));
            count += 1;
            if recursive && item.is_dir {
                subdirs.push((item.path.clone(), format!("{prefix}{}/", item.name)));
            }
        }
        pending.extend(subdirs.into_iter().rev());
    }
    if count == 0 {
        eprintln!("'{root}' is empty");
    }
    Ok(())
}

fn line(item: &RemoteFileInfo, prefix: &str) -> String {
    let date = item
        .modified
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| " ".repeat(16));
    let size = if item.is_dir {
        "<DIR>".to_string()
    } else {
        format_bytes(item.size)
    };
    let slash = if item.is_dir { "/" } else { "" };
    format!("{size:>10}  {date}  {prefix}{}{slash}", item.name)
}
