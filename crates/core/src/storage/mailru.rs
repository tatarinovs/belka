//! Mail.ru Cloud: files go through WebDAV, public links through the cloud's
//! own API, which accepts the same app password.

use super::webdav::WebDavProvider;
use super::{RemoteFileInfo, StorageProvider, TokenCache};
use crate::client::{
    build_http_client, ensure_success, json_response, send, Sink, Source, API_TIMEOUT,
};
use crate::progress::Progress;
use crate::provider::MAILRU_WEBDAV_URL;
use crate::utils::path::normalize_remote_dir;
use anyhow::{bail, Result};
use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde::Deserialize;

const TOKEN_URL: &str = "https://o2.mail.ru/token";
/// The client id of Mail.ru's own desktop app, which allows password sign-in.
const CLIENT_ID: &str = "cloud-win";
const API: &str = "https://cloud.mail.ru/api/m1";
const PUBLIC_URL: &str = "https://cloud.mail.ru/public/";

pub struct MailruProvider {
    dav: WebDavProvider,
    user: String,
    password: String,
    client: Client,
    token: TokenCache,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    expires_in: Option<u64>,
    /// Errors come with status 200 and these fields instead of the token.
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct Envelope<T> {
    body: T,
}

#[derive(Deserialize)]
struct ItemInfo {
    weblink: Option<String>,
}

impl MailruProvider {
    pub fn new(user: String, password: String, url: Option<String>) -> Result<Self> {
        let url = url.unwrap_or_else(|| MAILRU_WEBDAV_URL.to_string());
        // Mail.ru ETags are internal revision ids, not MD5 hashes.
        let dav =
            WebDavProvider::new("Mail.ru Cloud", &url, user.clone(), password.clone(), false)?;
        Ok(Self {
            dav,
            user,
            password,
            client: build_http_client()?,
            token: TokenCache::default(),
        })
    }

    async fn access_token(&self) -> Result<String> {
        self.token.get(|| self.sign_in()).await
    }

    async fn sign_in(&self) -> Result<(String, Option<u64>)> {
        let resp = send("mailru: sign in", || {
            Ok(self
                .client
                .post(TOKEN_URL)
                .form(&[
                    ("client_id", CLIENT_ID),
                    ("grant_type", "password"),
                    ("username", self.user.as_str()),
                    ("password", self.password.as_str()),
                ])
                .timeout(API_TIMEOUT))
        })
        .await?;
        let token: TokenResponse = json_response(resp, "Mail.ru sign-in failed").await?;
        let Some(access_token) = token.access_token else {
            let reason = token.error_description.or(token.error).unwrap_or_default();
            // A password made for WebDAV only works for files but not here.
            bail!(
                "Mail.ru refused to sign in for public links: {reason}. Links need an app                  password with full access to Mail, Cloud and Calendar (all protocols)"
            );
        };
        Ok((access_token, token.expires_in))
    }

    /// Calls a method of the cloud API with form parameters.
    async fn call(
        &self,
        op: &str,
        method: &str,
        form: &[(&str, &str)],
    ) -> Result<reqwest::Response> {
        let token = self.access_token().await?;
        let url = format!("{API}{method}");
        send(op, || {
            Ok(self
                .client
                .post(&url)
                .query(&[("access_token", token.as_str())])
                .form(form)
                .timeout(API_TIMEOUT))
        })
        .await
    }

    /// The public link id of the item, if it is shared.
    async fn weblink(&self, path: &str) -> Result<Option<String>> {
        let token = self.access_token().await?;
        let url = format!("{API}/file");
        let resp = send("mailru: get info", || {
            Ok(self
                .client
                .get(&url)
                .query(&[("access_token", token.as_str()), ("home", path)])
                .timeout(API_TIMEOUT))
        })
        .await?;
        if resp.status() == StatusCode::NOT_FOUND {
            bail!("'{path}' does not exist");
        }
        let info: Envelope<ItemInfo> =
            json_response(resp, format!("Failed to get info for '{path}'")).await?;
        Ok(info.body.weblink.filter(|w| !w.is_empty()))
    }
}

#[async_trait]
impl StorageProvider for MailruProvider {
    fn name(&self) -> &str {
        self.dav.name()
    }

    async fn check_access(&self, remote_dir: &str) -> Result<()> {
        self.dav.check_access(remote_dir).await
    }

    async fn ensure_dir(&self, remote_dir: &str) -> Result<()> {
        self.dav.ensure_dir(remote_dir).await
    }

    async fn get_file_info(&self, remote_path: &str) -> Result<Option<RemoteFileInfo>> {
        self.dav.get_file_info(remote_path).await
    }

    async fn upload(
        &self,
        source: &dyn Source,
        remote_path: &str,
        progress: &Progress,
    ) -> Result<()> {
        self.dav.upload(source, remote_path, progress).await
    }

    async fn download_to(
        &self,
        remote_path: &str,
        sink: &mut dyn Sink,
        progress: &Progress,
    ) -> Result<u64> {
        self.dav.download_to(remote_path, sink, progress).await
    }

    async fn delete(&self, remote_path: &str, is_dir: bool) -> Result<()> {
        self.dav.delete(remote_path, is_dir).await
    }

    async fn rename(&self, remote_path: &str, new_name: &str, is_dir: bool) -> Result<()> {
        self.dav.rename(remote_path, new_name, is_dir).await
    }

    async fn list_dir(&self, remote_dir: &str) -> Result<Vec<RemoteFileInfo>> {
        self.dav.list_dir(remote_dir).await
    }

    async fn share(&self, remote_path: &str, _is_dir: bool) -> Result<String> {
        let path = normalize_remote_dir(remote_path);
        let resp = self
            .call(
                "mailru: publish",
                "/file/publish",
                &[("home", path.as_str())],
            )
            .await?;
        if resp.status() == StatusCode::NOT_FOUND {
            bail!("'{path}' does not exist");
        }
        let published: Envelope<String> =
            json_response(resp, format!("Failed to share '{path}'")).await?;
        Ok(format!("{PUBLIC_URL}{}", published.body))
    }

    async fn unshare(&self, remote_path: &str, _is_dir: bool) -> Result<()> {
        let path = normalize_remote_dir(remote_path);
        let Some(weblink) = self.weblink(&path).await? else {
            return Ok(());
        };
        let resp = self
            .call(
                "mailru: unpublish",
                "/file/unpublish",
                &[("weblink", weblink.as_str())],
            )
            .await?;
        ensure_success(resp, format!("Failed to stop sharing '{path}'")).await?;
        Ok(())
    }
}
