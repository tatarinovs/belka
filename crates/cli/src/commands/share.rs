//! `belka share`: a public link to a cloud file or folder, copied to the
//! clipboard. Besides `SOURCE:PATH` it takes a file on a Synology network
//! share (`\\NAS\docs\x.zip` or a mapped drive): the share is the top
//! folder of File Station, and the NAS is found among the accounts by host.

use crate::remote::{Accounts, Remote};
use anyhow::{bail, Context, Result};
use belka_core::storage::ShareOptions;
use belka_core::utils::path::normalize_remote_dir;
use belka_core::{ProviderConfig, StorageProvider};
use chrono::{Local, NaiveDate};
use std::collections::HashSet;
use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, error, info};

pub struct Options {
    pub share: ShareOptions,
    pub unshare: bool,
    /// The Synology account for local paths, instead of finding it by host.
    pub account: Option<String>,
    pub clipboard: bool,
}

/// `--expires`: a number of days from today, or a date.
pub fn parse_expiry(value: &str) -> Result<NaiveDate, String> {
    let value = value.trim();
    if let Ok(days) = value.trim_end_matches(['d', 'д']).parse::<u32>() {
        if days == 0 {
            return Err("the link must work at least one day".into());
        }
        return Ok(Local::now().date_naive() + chrono::Days::new(u64::from(days)));
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| format!("'{value}' is neither a number of days nor a date like 2026-12-31"))
}

/// Shares every target; returns how many failed.
pub async fn run(accounts: &mut Accounts, targets: &[String], opts: &Options) -> Result<u64> {
    let mut links = Vec::new();
    let mut failed = 0;
    for target in targets {
        match share_one(accounts, target, opts).await {
            Ok(Some((name, link))) => {
                println!("{link}");
                links.push((name, link));
            }
            Ok(None) => info!("'{target}' is no longer shared"),
            Err(err) => {
                error!("{target}: {err:#}");
                failed += 1;
            }
        }
    }
    if opts.clipboard {
        copy_links(&links);
    }
    Ok(failed)
}

/// A link to `path`. Without settings an existing link is reused; with
/// them it is replaced.
pub async fn link(
    provider: &dyn StorageProvider,
    path: &str,
    is_dir: bool,
    options: &ShareOptions,
) -> Result<String> {
    debug!("Sharing '{path}' on {}", provider.name());
    if options.is_empty() {
        provider.share(path, is_dir).await
    } else {
        provider.share_with(path, is_dir, options).await
    }
}

/// Puts the links on the clipboard: a single link as it is, several as
/// `name: link` lines.
pub fn copy_links(links: &[(String, String)]) {
    let text = match links {
        [] => return,
        [(_, link)] => link.clone(),
        _ => links
            .iter()
            .map(|(name, link)| format!("{name}: {link}"))
            .collect::<Vec<_>>()
            .join("\r\n"),
    };
    match crate::clipboard::set_text(&text) {
        Ok(()) => info!(
            "{} copied to the clipboard",
            if links.len() == 1 { "Link" } else { "Links" }
        ),
        Err(err) => error!("{err:#}"),
    }
}

/// The file name and the link, or `None` once unshared.
async fn share_one(
    accounts: &mut Accounts,
    target: &str,
    opts: &Options,
) -> Result<Option<(String, String)>> {
    let (provider, path, is_dir) = if let Some(local) = local_path(target) {
        let meta = std::fs::metadata(local)
            .with_context(|| format!("'{}' is not accessible", local.display()))?;
        let (provider, path) = synology_location(accounts, local, opts.account.as_deref()).await?;
        (provider, path, meta.is_dir())
    } else {
        let remote: Remote = target.parse().map_err(anyhow::Error::msg)?;
        let (provider, _) = accounts.provider(&remote.source)?;
        let path = normalize_remote_dir(&remote.path);
        let info = provider
            .get_file_info(&path)
            .await?
            .with_context(|| format!("'{path}' does not exist on {}", provider.name()))?;
        (provider, path, info.is_dir)
    };
    if opts.unshare {
        provider.unshare(&path, is_dir).await?;
        return Ok(None);
    }
    let link = link(&*provider, &path, is_dir, &opts.share).await?;
    let name = path.rsplit('/').next().unwrap_or_default().to_string();
    Ok(Some((name, link)))
}

/// A local Windows path: `C:\...`, `C:/...` or `\\server\...`. Anything
/// else is `SOURCE:PATH`.
fn local_path(target: &str) -> Option<&Path> {
    let bytes = target.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    let unc = target.starts_with(r"\\") || target.starts_with("//");
    (drive || unc).then(|| Path::new(target))
}

/// A UNC path split into the server and the File Station path, which
/// starts with the share: `\\nas\docs\a\b.zip` → (`nas`, `/docs/a/b.zip`).
fn split_unc(unc: &str) -> Option<(String, String)> {
    let rest = unc
        .strip_prefix(r"\\?\UNC\")
        .or_else(|| unc.strip_prefix(r"\\"))
        .or_else(|| unc.strip_prefix("//"))?;
    let (server, path) = rest.split_once(['\\', '/'])?;
    let path = normalize_remote_dir(path);
    if server.is_empty() || path == "/" {
        return None;
    }
    Some((server.to_string(), path))
}

/// The host of a Synology account's address: a bare host, `host:port` or a URL.
fn account_host(address: &str) -> String {
    let address = address.trim();
    let rest = address.split_once("://").map_or(address, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => authority.split(':').next().unwrap_or_default(),
    };
    host.to_lowercase()
}

/// The Synology account the network share of `local` belongs to, and the
/// File Station path of `local`.
async fn synology_location(
    accounts: &mut Accounts,
    local: &Path,
    account: Option<&str>,
) -> Result<(Arc<dyn StorageProvider>, String)> {
    let unc = crate::netpath::to_unc(local)?;
    let (server, path) = split_unc(&unc)
        .with_context(|| format!("'{}' is not a file on a network share", local.display()))?;
    debug!("'{}' is '{path}' on \\\\{server}", local.display());

    let source = accounts.source()?;
    if let Some(wanted) = account {
        let found = source.find(wanted)?;
        if found.kind != "synology" {
            bail!("'{}' is not a Synology account", found.name);
        }
        return Ok((source.provider(&found.id)?, path));
    }
    let mut nas = Vec::new();
    for found in source
        .accounts()?
        .into_iter()
        .filter(|a| a.kind == "synology")
    {
        if let Ok(ProviderConfig::Synology { url, .. }) = source.credentials(&found.id) {
            nas.push((found, account_host(&url)));
        }
    }
    if nas.is_empty() {
        bail!("There is no Synology account; add the NAS in the app first");
    }
    let server = server.to_lowercase();
    let mut matched = nas.iter().find(|(_, host)| *host == server);
    if matched.is_none() {
        // \\NAS and 192.168.1.10 may well be the same machine.
        let server_ips = resolve(&server).await;
        for candidate in &nas {
            if !server_ips.is_disjoint(&resolve(&candidate.1).await) {
                matched = Some(candidate);
                break;
            }
        }
    }
    let (found, _) = matched.with_context(|| {
        let hosts: Vec<String> = nas
            .iter()
            .map(|(a, h)| format!("'{}' ({h})", a.name))
            .collect();
        format!(
            "No Synology account is at '{server}'; accounts: {}. Name one with --account",
            hosts.join(", ")
        )
    })?;
    debug!("'{server}' is the account '{}'", found.name);
    Ok((source.provider(&found.id)?, path))
}

async fn resolve(host: &str) -> HashSet<IpAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return HashSet::from([ip]);
    }
    match tokio::net::lookup_host((host, 0)).await {
        Ok(addrs) => addrs.map(|a| a.ip()).collect(),
        Err(err) => {
            debug!("Cannot resolve '{host}': {err}");
            HashSet::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_local_paths_from_cloud_ones() {
        assert!(local_path(r"Z:\disk\a.7z").is_some());
        assert!(local_path("z:/disk/a.7z").is_some());
        assert!(local_path(r"\\nas\docs\a.7z").is_some());
        assert!(local_path("Мой NAS:/docs/a.7z").is_none());
        assert!(local_path("z:/").is_some());
        assert!(local_path("z:").is_none());
        assert!(local_path("nas:/docs").is_none());
    }

    #[test]
    fn splits_unc_paths() {
        let split = |s| split_unc(s).unwrap();
        assert_eq!(
            split(r"\\192.168.1.10\public\disk\Иванов.7z"),
            ("192.168.1.10".into(), "/public/disk/Иванов.7z".into())
        );
        assert_eq!(split(r"\\?\UNC\Nas\ssd\x"), ("Nas".into(), "/ssd/x".into()));
        assert_eq!(split("//nas/docs/"), ("nas".into(), "/docs".into()));
        assert!(split_unc(r"\\nas").is_none());
        assert!(split_unc(r"\\nas\").is_none());
        assert!(split_unc(r"C:\x").is_none());
    }

    #[test]
    fn finds_account_hosts() {
        assert_eq!(account_host("192.168.1.10"), "192.168.1.10");
        assert_eq!(account_host("NAS:5001"), "nas");
        assert_eq!(
            account_host("https://nas.example.com:5001/"),
            "nas.example.com"
        );
        assert_eq!(account_host("http://[fe80::1]:5000"), "fe80::1");
    }

    #[test]
    fn parses_expiry() {
        let today = Local::now().date_naive();
        assert_eq!(parse_expiry("7").unwrap(), today + chrono::Days::new(7));
        assert_eq!(parse_expiry("7d").unwrap(), today + chrono::Days::new(7));
        assert_eq!(
            parse_expiry("2026-12-31").unwrap(),
            NaiveDate::from_ymd_opt(2026, 12, 31).unwrap()
        );
        assert!(parse_expiry("0").is_err());
        assert!(parse_expiry("week").is_err());
    }
}
