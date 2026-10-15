use anyhow::{bail, Context, Result};
use belka_core::oauth::{OAuthApp, Service};
use belka_core::provider::{DEFAULT_S3_REGION, MAILRU_WEBDAV_URL};
use belka_core::storage::google::GoogleAuth;
use belka_core::{ProviderConfig, StorageProvider};
use clap::ValueEnum;
use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ProviderKind {
    #[value(alias = "ya", alias = "yadisk")]
    Yandex,
    #[value(alias = "mail", alias = "mail.ru")]
    Mailru,
    #[value(alias = "gdrive", alias = "drive")]
    Google,
    #[value(alias = "nextcloud", alias = "owncloud")]
    Webdav,
    #[value(alias = "r2", alias = "minio")]
    S3,
}

/// Extracts `--env-file` before clap runs, so that values from the file can
/// serve as defaults for `env = "..."` arguments such as CLOUD_PROVIDER.
pub fn env_file_from_args() -> Option<PathBuf> {
    find_env_file_arg(env::args_os().skip(1))
}

fn find_env_file_arg(args: impl Iterator<Item = OsString>) -> Option<PathBuf> {
    let mut args = args;
    while let Some(arg) = args.next() {
        if arg == "--" {
            break;
        }
        if arg == "--env-file" {
            return args.next().map(PathBuf::from);
        }
        if let Some(value) = arg.to_str().and_then(|s| s.strip_prefix("--env-file=")) {
            return Some(PathBuf::from(value));
        }
    }
    None
}

/// Loads `.env` from the explicit path, else from the current directory, else
/// next to the executable. Variables already set in the environment win.
/// Returns the file that was loaded, if any.
pub fn load_environment(explicit: Option<&Path>) -> Result<Option<PathBuf>> {
    if let Some(path) = explicit {
        if !path.is_file() {
            bail!("Environment file not found: {}", path.display());
        }
        dotenvy::from_path(path)
            .with_context(|| format!("Failed to load environment file {}", path.display()))?;
        return Ok(Some(path.to_path_buf()));
    }

    let mut candidates = vec![PathBuf::from(".env")];
    if let Some(dir) = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        candidates.push(dir.join(".env"));
    }
    for candidate in candidates {
        if candidate.is_file() {
            dotenvy::from_path(&candidate).with_context(|| {
                format!("Failed to load environment file {}", candidate.display())
            })?;
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

fn var(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Reads all required variables, reporting every missing one at once.
fn require<const N: usize>(provider: &str, names: [&str; N]) -> Result<[String; N]> {
    let values = names.map(var);
    let missing: Vec<&str> = names
        .iter()
        .zip(&values)
        .filter(|(_, v)| v.is_none())
        .map(|(n, _)| *n)
        .collect();
    if !missing.is_empty() {
        bail!(
            "{provider} is not configured: set {} in .env or the environment (see --list-providers)",
            missing.join(", ")
        );
    }
    Ok(values.map(Option::unwrap_or_default))
}

fn flag(name: &str) -> bool {
    var(name)
        .is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
}

/// The OAuth app from GOOGLE_CLIENT_ID / YANDEX_CLIENT_ID and the matching
/// secret, else the one built into this binary. A refresh token only works
/// with the app it was issued to.
pub fn oauth_app(service: Service) -> Result<Option<OAuthApp>> {
    let prefix = match service {
        Service::Google => "GOOGLE",
        Service::Yandex => "YANDEX",
    };
    let id_var = format!("{prefix}_CLIENT_ID");
    let secret_var = format!("{prefix}_CLIENT_SECRET");
    match (var(&id_var), var(&secret_var)) {
        (Some(_), None) if service == Service::Google => {
            bail!("{id_var} is set without {secret_var}")
        }
        (Some(client_id), secret) => Ok(Some(OAuthApp {
            client_id,
            client_secret: secret.unwrap_or_default(),
        })),
        (None, Some(_)) => bail!("{secret_var} is set without {id_var}"),
        (None, None) => Ok(service.builtin_app()),
    }
}

pub fn init_provider(kind: ProviderKind) -> Result<Box<dyn StorageProvider>> {
    provider_config(kind)?.build()
}

/// Reads the settings of `kind` from the environment.
fn provider_config(kind: ProviderKind) -> Result<ProviderConfig> {
    Ok(match kind {
        ProviderKind::Yandex => {
            let [token] = require("Yandex Disk", ["YANDEX_TOKEN"])?;
            ProviderConfig::Yandex { token }
        }
        ProviderKind::Mailru => {
            let [user, password] = require("Mail.ru Cloud", ["MAILRU_USER", "MAILRU_PASSWORD"])?;
            ProviderConfig::Mailru {
                user,
                password,
                url: var("MAILRU_WEBDAV_URL"),
            }
        }
        ProviderKind::Google => {
            let auth = match (var("GOOGLE_REFRESH_TOKEN"), var("GOOGLE_DRIVE_TOKEN")) {
                (Some(refresh_token), _) => {
                    let app = oauth_app(Service::Google)?.context(
                        "GOOGLE_REFRESH_TOKEN needs GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET: \
                         this build has no built-in OAuth app",
                    )?;
                    GoogleAuth::RefreshToken {
                        client_id: app.client_id,
                        client_secret: app.client_secret,
                        refresh_token,
                    }
                }
                (None, Some(access_token)) => GoogleAuth::AccessToken { access_token },
                (None, None) => bail!(
                    "Google Drive is not configured: run `belka --login google` to get \
                     GOOGLE_REFRESH_TOKEN, or set a short-lived GOOGLE_DRIVE_TOKEN"
                ),
            };
            ProviderConfig::Google(auth)
        }
        ProviderKind::Webdav => {
            let [url, user, password] =
                require("WebDAV", ["WEBDAV_URL", "WEBDAV_USER", "WEBDAV_PASSWORD"])?;
            ProviderConfig::Webdav {
                url,
                user,
                password,
                etag_is_md5: flag("WEBDAV_ETAG_IS_MD5"),
            }
        }
        ProviderKind::S3 => {
            let [endpoint, bucket, access_key_id, secret_access_key] = require(
                "S3",
                [
                    "S3_ENDPOINT",
                    "S3_BUCKET",
                    "S3_ACCESS_KEY_ID",
                    "S3_SECRET_ACCESS_KEY",
                ],
            )?;
            ProviderConfig::S3 {
                endpoint,
                bucket,
                access_key_id,
                secret_access_key,
                region: var("S3_REGION"),
            }
        }
    })
}

pub fn print_providers() {
    let builtin = if Service::Google.builtin_app().is_some() {
        ""
    } else {
        " (not in this build)"
    };
    println!(
        "\
Supported cloud providers (-p, --provider):

  yandex   Yandex Disk (REST API)
           YANDEX_TOKEN                OAuth token with Disk REST API access,
                                       `belka --login yandex` prints one

  mailru   Mail.ru Cloud (WebDAV)
           MAILRU_USER, MAILRU_PASSWORD  e-mail and an app password
           MAILRU_WEBDAV_URL           optional, default {MAILRU_WEBDAV_URL}

  google   Google Drive (API v3)
           GOOGLE_REFRESH_TOKEN        recommended, `belka --login google` prints one;
                                       access tokens are renewed automatically
           GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET
                                       your own OAuth app; default: the built-in one{builtin}
           GOOGLE_DRIVE_TOKEN          alternative: access token, valid ~1 hour

  webdav   Any WebDAV server (Nextcloud, ownCloud, Synology, ...)
           WEBDAV_URL, WEBDAV_USER, WEBDAV_PASSWORD
           WEBDAV_ETAG_IS_MD5          optional, 'true' if the server's ETag is the file MD5

  s3       S3 compatible (AWS, Cloudflare R2, MinIO, Yandex Object Storage)
           S3_ENDPOINT, S3_BUCKET, S3_ACCESS_KEY_ID, S3_SECRET_ACCESS_KEY
           S3_REGION                   optional, default {DEFAULT_S3_REGION} (R2: auto)

Optional for URL mirroring: GITHUB_TOKEN (raises the GitHub API rate limit).

Variables are read from the environment and from a .env file (current folder,
next to the executable, or --env-file). Real environment variables take precedence."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> impl Iterator<Item = OsString> {
        list.iter()
            .map(OsString::from)
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn finds_env_file_argument() {
        assert_eq!(
            find_env_file_arg(args(&["-p", "s3", "--env-file", "a.env", "x"])),
            Some(PathBuf::from("a.env"))
        );
        assert_eq!(
            find_env_file_arg(args(&["--env-file=b.env"])),
            Some(PathBuf::from("b.env"))
        );
        assert_eq!(find_env_file_arg(args(&["--", "--env-file", "c"])), None);
        assert_eq!(find_env_file_arg(args(&["dir"])), None);
    }
}
