//! Yandex Disk REST API: https://yandex.ru/dev/disk-api/doc/

use super::{
    as_md5, create_dirs, parse_rfc3339, rename_target, Mkdir, RemoteFileInfo, StorageProvider,
};
use crate::client::{
    build_http_client, download_into, ensure_success, http_error, json_response, retry_transient,
    send, source_body, upload_timeout, Failure, Sink, Source, API_TIMEOUT,
};
use crate::progress::Progress;
use crate::utils::path::normalize_remote_dir;
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_LENGTH};
use reqwest::{Client, Method, RequestBuilder, StatusCode};
use serde::Deserialize;
use std::time::{Duration, Instant};

const API: &str = "https://cloud-api.yandex.net/v1/disk/resources";
const PAGE_SIZE: usize = 1000;
const ITEM_FIELDS: &str = "name,path,type,size,md5,modified";
const LIST_FIELDS: &str = "_embedded.items.name,_embedded.items.path,_embedded.items.type,\
                           _embedded.items.size,_embedded.items.md5,_embedded.items.modified";
const OPERATION_TIMEOUT: Duration = Duration::from_secs(600);

pub struct YandexDiskProvider {
    auth: String,
    client: Client,
}

#[derive(Deserialize)]
struct Link {
    href: String,
}

#[derive(Deserialize)]
struct Resource {
    name: String,
    path: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    size: u64,
    md5: Option<String>,
    modified: Option<String>,
}

impl Resource {
    fn into_info(self) -> RemoteFileInfo {
        let path = self.path.strip_prefix("disk:").unwrap_or(&self.path);
        RemoteFileInfo {
            path: normalize_remote_dir(path),
            is_dir: self.kind == "dir",
            size: self.size,
            md5: self.md5.as_deref().and_then(as_md5),
            modified: self.modified.as_deref().and_then(parse_rfc3339),
            name: self.name,
        }
    }
}

#[derive(Deserialize)]
struct Listing {
    #[serde(rename = "_embedded")]
    embedded: Option<ListingItems>,
}

#[derive(Deserialize)]
struct ListingItems {
    #[serde(default)]
    items: Vec<Resource>,
}

#[derive(Deserialize)]
struct Operation {
    status: String,
}

#[derive(Deserialize)]
struct Published {
    public_url: Option<String>,
}

impl YandexDiskProvider {
    pub fn new(token: &str) -> Result<Self> {
        Ok(Self {
            auth: format!("OAuth {token}"),
            client: build_http_client()?,
        })
    }

    fn api(&self, method: Method, endpoint: &str) -> RequestBuilder {
        self.client
            .request(method, format!("{API}{endpoint}"))
            .header(AUTHORIZATION, &self.auth)
            .timeout(API_TIMEOUT)
    }

    /// Requests a fresh upload URL and streams the file to it.
    async fn upload_once(
        &self,
        source: &dyn Source,
        remote: &str,
        progress: &Progress,
    ) -> Result<(), Failure> {
        let resp = send("yandex: request upload URL", || {
            Ok(self
                .api(Method::GET, "/upload")
                .query(&[("path", remote), ("overwrite", "true")]))
        })
        .await?;
        if !resp.status().is_success() {
            return Err(Failure::from_response(
                resp,
                format!("Failed to get upload URL for '{remote}'"),
            )
            .await);
        }
        let link: Link = resp.json().await.context("Invalid upload URL response")?;

        let len = source.len();
        let body = source_body(source, 0, progress)?;
        let resp = self
            .client
            .put(&link.href)
            .header(CONTENT_LENGTH, len)
            .timeout(upload_timeout(len))
            .body(body)
            .send()
            .await?;

        match resp.status() {
            StatusCode::CREATED | StatusCode::OK | StatusCode::ACCEPTED => Ok(()),
            _ => Err(Failure::from_response(resp, format!("Upload of '{remote}' failed")).await),
        }
    }

    async fn download_link(&self, path: &str) -> Result<String> {
        let resp = send("yandex: request download URL", || {
            Ok(self.api(Method::GET, "/download").query(&[("path", path)]))
        })
        .await?;
        if resp.status() == StatusCode::NOT_FOUND {
            bail!("'{path}' does not exist");
        }
        let link: Link =
            json_response(resp, format!("Failed to get download URL for '{path}'")).await?;
        Ok(link.href)
    }

    /// Creates one folder; Yandex tells "exists" from "no parent" only in the body.
    async fn mkdir(&self, path: String) -> Result<Mkdir> {
        let resp = send("yandex: create folder", || {
            Ok(self.api(Method::PUT, "").query(&[("path", path.as_str())]))
        })
        .await?;
        match resp.status() {
            StatusCode::CREATED | StatusCode::OK => Ok(Mkdir::Done),
            StatusCode::CONFLICT => {
                let body = resp.text().await.unwrap_or_default();
                if body.contains("DiskPathPointsToExistentDirectoryError") {
                    Ok(Mkdir::Done)
                } else if body.contains("DiskPathDoesntExistsError") {
                    Ok(Mkdir::ParentMissing)
                } else {
                    bail!(
                        "Failed to create folder '{path}': HTTP 409: {}",
                        body.trim()
                    )
                }
            }
            _ => Err(http_error(resp, format!("Failed to create folder '{path}'")).await),
        }
    }

    async fn wait_for_operation(&self, href: &str) -> Result<()> {
        let started = Instant::now();
        loop {
            let resp = send("yandex: operation status", || {
                Ok(self
                    .client
                    .get(href)
                    .header(AUTHORIZATION, &self.auth)
                    .timeout(API_TIMEOUT))
            })
            .await?;
            let op: Operation = json_response(resp, "Failed to query operation status").await?;
            match op.status.as_str() {
                "success" => return Ok(()),
                "failed" => bail!("Yandex Disk reported that the operation failed"),
                _ if started.elapsed() > OPERATION_TIMEOUT => {
                    bail!("Timed out waiting for a Yandex Disk operation to finish")
                }
                _ => tokio::time::sleep(Duration::from_secs(2)).await,
            }
        }
    }
}

#[async_trait]
impl StorageProvider for YandexDiskProvider {
    fn name(&self) -> &str {
        "Yandex Disk"
    }

    async fn ensure_dir(&self, remote_dir: &str) -> Result<()> {
        let dir = normalize_remote_dir(remote_dir);
        if dir == "/" {
            return Ok(());
        }
        match self.get_file_info(&dir).await? {
            Some(info) if info.is_dir => return Ok(()),
            Some(_) => bail!("'{dir}' exists on Yandex Disk but is a file"),
            None => {}
        }

        create_dirs(&dir, |path| self.mkdir(path)).await
    }

    async fn get_file_info(&self, remote_path: &str) -> Result<Option<RemoteFileInfo>> {
        let path = normalize_remote_dir(remote_path);
        let resp = send("yandex: get info", || {
            Ok(self
                .api(Method::GET, "")
                .query(&[("path", path.as_str()), ("fields", ITEM_FIELDS)]))
        })
        .await?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let resource: Resource =
            json_response(resp, format!("Failed to get info for '{path}'")).await?;
        Ok(Some(resource.into_info()))
    }

    async fn upload(
        &self,
        source: &dyn Source,
        remote_path: &str,
        progress: &Progress,
    ) -> Result<()> {
        let path = normalize_remote_dir(remote_path);
        retry_transient(&format!("yandex: upload '{path}'"), progress, || {
            self.upload_once(source, &path, progress)
        })
        .await
    }

    async fn download_to(
        &self,
        remote_path: &str,
        sink: &mut dyn Sink,
        progress: &Progress,
    ) -> Result<u64> {
        let path = normalize_remote_dir(remote_path);
        // Download links expire, so every attempt asks for a fresh one.
        download_into(
            &format!("yandex: download '{path}'"),
            sink,
            progress,
            || async {
                let link = self.download_link(&path).await?;
                Ok(self.client.get(link))
            },
        )
        .await
    }

    async fn delete(&self, remote_path: &str, _is_dir: bool) -> Result<()> {
        let path = normalize_remote_dir(remote_path);
        if path == "/" {
            bail!("Refusing to delete the root folder");
        }
        let resp = send("yandex: delete", || {
            Ok(self
                .api(Method::DELETE, "")
                .query(&[("path", path.as_str()), ("permanently", "true")]))
        })
        .await?;
        match resp.status() {
            StatusCode::NO_CONTENT | StatusCode::OK | StatusCode::NOT_FOUND => Ok(()),
            StatusCode::ACCEPTED => {
                // Large folders are deleted asynchronously.
                let link: Link = resp.json().await.context("Invalid delete response")?;
                self.wait_for_operation(&link.href).await
            }
            _ => Err(http_error(resp, format!("Failed to delete '{path}'")).await),
        }
    }

    async fn rename(&self, remote_path: &str, new_name: &str, _is_dir: bool) -> Result<()> {
        let from = normalize_remote_dir(remote_path);
        let to = rename_target(&from, new_name)?;
        let resp = send("yandex: rename", || {
            Ok(self.api(Method::POST, "/move").query(&[
                ("from", from.as_str()),
                ("path", to.as_str()),
                ("overwrite", "false"),
            ]))
        })
        .await?;
        match resp.status() {
            StatusCode::CREATED => Ok(()),
            StatusCode::ACCEPTED => {
                // Large folders are moved asynchronously.
                let link: Link = resp.json().await.context("Invalid move response")?;
                self.wait_for_operation(&link.href).await
            }
            StatusCode::CONFLICT => bail!("'{}' already exists", new_name.trim()),
            _ => Err(http_error(resp, format!("Failed to rename '{from}'")).await),
        }
    }

    async fn list_dir(&self, remote_dir: &str) -> Result<Vec<RemoteFileInfo>> {
        let dir = normalize_remote_dir(remote_dir);
        let limit = PAGE_SIZE.to_string();
        let mut items = Vec::new();
        loop {
            let offset = items.len().to_string();
            let resp = send("yandex: list folder", || {
                Ok(self.api(Method::GET, "").query(&[
                    ("path", dir.as_str()),
                    ("limit", limit.as_str()),
                    ("offset", offset.as_str()),
                    ("fields", LIST_FIELDS),
                ]))
            })
            .await?;
            if resp.status() == StatusCode::NOT_FOUND {
                break;
            }
            let listing: Listing = json_response(resp, format!("Failed to list '{dir}'")).await?;
            let page = listing.embedded.map(|e| e.items).unwrap_or_default();
            let count = page.len();
            items.extend(page.into_iter().map(Resource::into_info));
            if count < PAGE_SIZE {
                break;
            }
        }
        Ok(items)
    }

    async fn share(&self, remote_path: &str, _is_dir: bool) -> Result<String> {
        let path = normalize_remote_dir(remote_path);
        let resp = send("yandex: publish", || {
            Ok(self
                .api(Method::PUT, "/publish")
                .query(&[("path", path.as_str())]))
        })
        .await?;
        if resp.status() == StatusCode::NOT_FOUND {
            bail!("'{path}' does not exist");
        }
        ensure_success(resp, format!("Failed to share '{path}'")).await?;

        let resp = send("yandex: get public link", || {
            Ok(self
                .api(Method::GET, "")
                .query(&[("path", path.as_str()), ("fields", "public_url")]))
        })
        .await?;
        let published: Published =
            json_response(resp, format!("Failed to get the link to '{path}'")).await?;
        published
            .public_url
            .context("Yandex Disk did not return a public link")
    }

    async fn unshare(&self, remote_path: &str, _is_dir: bool) -> Result<()> {
        let path = normalize_remote_dir(remote_path);
        let resp = send("yandex: unpublish", || {
            Ok(self
                .api(Method::PUT, "/unpublish")
                .query(&[("path", path.as_str())]))
        })
        .await?;
        if resp.status() == StatusCode::NOT_FOUND {
            bail!("'{path}' does not exist");
        }
        ensure_success(resp, format!("Failed to stop sharing '{path}'")).await?;
        Ok(())
    }
}
