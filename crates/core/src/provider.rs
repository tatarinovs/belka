//! Provider settings in a serializable form, independent of where they come
//! from (environment variables for the CLI, saved accounts for the GUI).

use crate::storage::google::{GoogleAuth, GoogleDriveProvider};
use crate::storage::mailru::MailruProvider;
use crate::storage::s3::{region_for_endpoint, S3Provider};
use crate::storage::synology::SynologyProvider;
use crate::storage::webdav::WebDavProvider;
use crate::storage::yandex::YandexDiskProvider;
use crate::storage::StorageProvider;
use anyhow::Result;
use serde::{Deserialize, Serialize};

pub const MAILRU_WEBDAV_URL: &str = "https://webdav.mail.ru";
pub const DEFAULT_S3_REGION: &str = "us-east-1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ProviderConfig {
    Yandex {
        token: String,
    },
    Mailru {
        user: String,
        password: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
    },
    Google(GoogleAuth),
    Webdav {
        url: String,
        user: String,
        password: String,
        #[serde(default)]
        etag_is_md5: bool,
    },
    S3 {
        endpoint: String,
        bucket: String,
        access_key_id: String,
        secret_access_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        region: Option<String>,
    },
    /// Synology DSM through the File Station API.
    Synology {
        /// A host, or a URL of DSM.
        url: String,
        /// Overrides the port of `url`; by default DSM's 5000 or 5001.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
        /// A bare host is reached over HTTPS.
        #[serde(default)]
        https: bool,
        user: String,
        password: String,
        /// Accept any HTTPS certificate, such as DSM's self-signed one.
        #[serde(default)]
        insecure: bool,
    },
}

impl ProviderConfig {
    /// The `kind` tag used in the serialized form.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Yandex { .. } => "yandex",
            Self::Mailru { .. } => "mailru",
            Self::Google(_) => "google",
            Self::Webdav { .. } => "webdav",
            Self::S3 { .. } => "s3",
            Self::Synology { .. } => "synology",
        }
    }

    pub fn build(&self) -> Result<Box<dyn StorageProvider>> {
        Ok(match self.clone() {
            Self::Yandex { token } => Box::new(YandexDiskProvider::new(&token)?),
            Self::Mailru {
                user,
                password,
                url,
            } => Box::new(MailruProvider::new(user, password, url)?),
            Self::Google(auth) => Box::new(GoogleDriveProvider::new(auth)?),
            Self::Webdav {
                url,
                user,
                password,
                etag_is_md5,
            } => Box::new(WebDavProvider::new(
                "WebDAV",
                &url,
                user,
                password,
                etag_is_md5,
            )?),
            Self::S3 {
                endpoint,
                bucket,
                access_key_id,
                secret_access_key,
                region,
            } => {
                let region = region
                    .or_else(|| region_for_endpoint(&endpoint))
                    .unwrap_or_else(|| DEFAULT_S3_REGION.to_string());
                Box::new(S3Provider::new(
                    &endpoint,
                    bucket,
                    access_key_id,
                    secret_access_key,
                    region,
                )?)
            }
            Self::Synology {
                url,
                port,
                https,
                user,
                password,
                insecure,
            } => Box::new(SynologyProvider::new(
                &url, port, https, insecure, user, password,
            )?),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_with_kind_tag() {
        let cfg = ProviderConfig::Google(GoogleAuth::RefreshToken {
            client_id: "id".into(),
            client_secret: "secret".into(),
            refresh_token: "rt".into(),
        });
        let json = serde_json::to_value(&cfg).unwrap();
        assert_eq!(json["kind"], cfg.kind());
        assert_eq!(json["refresh_token"], "rt");
        assert_eq!(serde_json::from_value::<ProviderConfig>(json).unwrap(), cfg);
    }
}
