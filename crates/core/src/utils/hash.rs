use anyhow::{Context, Result};
use md5::{Digest, Md5};
use std::io::Read;
use std::path::Path;

/// Streams a file through MD5 and returns the lowercase hex digest. Runs on
/// a blocking thread: hashing gigabytes would otherwise hold up the async
/// runtime that the transfers and the media server share.
pub async fn compute_md5_file(path: &Path) -> Result<String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || md5_file(&path))
        .await
        .context("Hashing was interrupted")?
}

fn md5_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("Failed to open '{}' for hashing", path.display()))?;

    let mut hasher = Md5::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let n = file
            .read(&mut buffer)
            .with_context(|| format!("Failed to read '{}'", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }

    Ok(hex::encode(hasher.finalize()))
}
