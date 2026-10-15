//! Google Drive API v3 with resumable uploads.

use super::{as_md5, parse_rfc3339, rename_target, RemoteFileInfo, StorageProvider, TokenCache};
use crate::client::{
    client_builder, download_into, ensure_success, http_error, json_response, pause_before_retry,
    send, source_body, upload_timeout, Failure, Sink, Source, API_TIMEOUT, MAX_ATTEMPTS,
};
use crate::progress::Progress;
use crate::utils::path::{join_remote, normalize_remote_dir, split_remote};
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, LOCATION, RANGE};
use reqwest::{redirect, Client, Response, StatusCode};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use tokio::sync::Mutex;
use tracing::debug;

const FILES_API: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD_API: &str = "https://www.googleapis.com/upload/drive/v3/files";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const FILE_FIELDS: &str = "id,name,size,mimeType,md5Checksum,modifiedTime";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum GoogleAuth {
    /// A short-lived access token (valid for about an hour).
    AccessToken { access_token: String },
    /// OAuth client credentials plus a refresh token: access tokens are renewed automatically.
    RefreshToken {
        client_id: String,
        client_secret: String,
        refresh_token: String,
    },
}

pub struct GoogleDriveProvider {
    client: Client,
    auth: GoogleAuth,
    token: TokenCache,
    /// Folder path -> id. The lock also serializes folder creation so that
    /// parallel uploads never create duplicate folders.
    folders: Mutex<HashMap<String, String>>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: Option<u64>,
}

#[derive(Deserialize)]
struct FileList {
    #[serde(default)]
    files: Vec<DriveFile>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct DriveFile {
    id: String,
    name: String,
    size: Option<String>,
    #[serde(rename = "mimeType")]
    mime_type: Option<String>,
    #[serde(rename = "md5Checksum")]
    md5_checksum: Option<String>,
    #[serde(rename = "modifiedTime")]
    modified_time: Option<String>,
}

impl DriveFile {
    fn is_folder(&self) -> bool {
        self.mime_type.as_deref() == Some(FOLDER_MIME)
    }

    fn into_info(self, path: String) -> RemoteFileInfo {
        RemoteFileInfo {
            is_dir: self.is_folder(),
            size: self
                .size
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            md5: self.md5_checksum.as_deref().and_then(as_md5),
            modified: self.modified_time.as_deref().and_then(parse_rfc3339),
            name: self.name,
            path,
        }
    }
}

enum UploadState {
    Done,
    /// Server has persisted bytes up to (excluding) this offset.
    Incomplete(u64),
}

/// Escapes a value for use inside single quotes in a Drive `q` expression.
fn escape_query(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

impl GoogleDriveProvider {
    pub fn new(auth: GoogleAuth) -> Result<Self> {
        // 308 is "Resume Incomplete" for resumable uploads, not a redirect;
        // other redirects (media downloads) are followed.
        let client = client_builder()
            .redirect(redirect::Policy::custom(|attempt| {
                if attempt.status() == StatusCode::PERMANENT_REDIRECT
                    || attempt.previous().len() >= 10
                {
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .context("Failed to initialize HTTP client")?;
        Ok(Self {
            client,
            auth,
            token: TokenCache::default(),
            folders: Mutex::new(HashMap::new()),
        })
    }

    async fn access_token(&self) -> Result<String> {
        let (client_id, client_secret, refresh_token) = match &self.auth {
            GoogleAuth::AccessToken { access_token } => return Ok(access_token.clone()),
            GoogleAuth::RefreshToken {
                client_id,
                client_secret,
                refresh_token,
            } => (client_id, client_secret, refresh_token),
        };

        self.token
            .get(|| self.refresh(client_id, client_secret, refresh_token))
            .await
    }

    async fn refresh(
        &self,
        client_id: &str,
        client_secret: &str,
        refresh_token: &str,
    ) -> Result<(String, Option<u64>)> {
        let resp = send("google: refresh access token", || {
            Ok(self
                .client
                .post(TOKEN_URL)
                .form(&[
                    ("client_id", client_id),
                    ("client_secret", client_secret),
                    ("refresh_token", refresh_token),
                    ("grant_type", "refresh_token"),
                ])
                .timeout(API_TIMEOUT))
        })
        .await?;
        let token: TokenResponse =
            json_response(resp, "Failed to refresh Google access token").await?;
        debug!("Obtained a new Google access token");
        Ok((token.access_token, token.expires_in))
    }

    async fn query_files(&self, q: &str, limit: Option<usize>) -> Result<Vec<DriveFile>> {
        let token = self.access_token().await?;
        let fields = format!("nextPageToken,files({FILE_FIELDS})");
        let page_size = limit.unwrap_or(1000).min(1000).to_string();
        let mut files = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let resp = send("google: list files", || {
                let mut req = self
                    .client
                    .get(FILES_API)
                    .bearer_auth(&token)
                    .timeout(API_TIMEOUT)
                    .query(&[
                        ("q", q),
                        ("fields", fields.as_str()),
                        ("pageSize", page_size.as_str()),
                        ("spaces", "drive"),
                    ]);
                if let Some(t) = page_token.as_deref() {
                    req = req.query(&[("pageToken", t)]);
                }
                Ok(req)
            })
            .await?;
            let page: FileList = json_response(resp, "Google Drive query failed").await?;
            files.extend(page.files);
            if limit.is_some_and(|l| files.len() >= l) {
                break;
            }
            match page.next_page_token {
                Some(next) => page_token = Some(next),
                None => break,
            }
        }
        Ok(files)
    }

    /// Finds a direct child by name. `folder` restricts the kind of item.
    async fn find_child(
        &self,
        parent_id: &str,
        name: &str,
        folder: Option<bool>,
    ) -> Result<Option<DriveFile>> {
        let mut q = format!(
            "'{}' in parents and name = '{}' and trashed = false",
            escape_query(parent_id),
            escape_query(name)
        );
        match folder {
            Some(true) => q.push_str(&format!(" and mimeType = '{FOLDER_MIME}'")),
            Some(false) => q.push_str(&format!(" and mimeType != '{FOLDER_MIME}'")),
            None => {}
        }
        Ok(self.query_files(&q, Some(1)).await?.into_iter().next())
    }

    /// Resolves a folder path to its id, optionally creating missing folders.
    async fn folder_id(&self, dir: &str, create: bool) -> Result<Option<String>> {
        let dir = normalize_remote_dir(dir);
        if dir == "/" {
            return Ok(Some("root".to_string()));
        }
        let mut cache = self.folders.lock().await;
        if let Some(id) = cache.get(&dir) {
            return Ok(Some(id.clone()));
        }

        let mut parent = "root".to_string();
        let mut current = String::new();
        for segment in dir.split('/').filter(|s| !s.is_empty()) {
            current.push('/');
            current.push_str(segment);
            if let Some(id) = cache.get(&current) {
                parent = id.clone();
                continue;
            }
            let id = match self.find_child(&parent, segment, Some(true)).await? {
                Some(folder) => folder.id,
                None if create => self.create_folder(segment, &parent).await?,
                None => return Ok(None),
            };
            cache.insert(current.clone(), id.clone());
            parent = id;
        }
        Ok(Some(parent))
    }

    async fn create_folder(&self, name: &str, parent_id: &str) -> Result<String> {
        let token = self.access_token().await?;
        let metadata = json!({ "name": name, "mimeType": FOLDER_MIME, "parents": [parent_id] });
        let resp = send("google: create folder", || {
            Ok(self
                .client
                .post(FILES_API)
                .bearer_auth(&token)
                .query(&[("fields", "id")])
                .json(&metadata)
                .timeout(API_TIMEOUT))
        })
        .await?;
        #[derive(Deserialize)]
        struct Created {
            id: String,
        }
        let created: Created =
            json_response(resp, format!("Failed to create folder '{name}'")).await?;
        Ok(created.id)
    }

    /// Locates an item by path without creating anything.
    async fn find_item(
        &self,
        remote_path: &str,
        folder: Option<bool>,
    ) -> Result<Option<DriveFile>> {
        let (parent, name) = split_remote(remote_path);
        if name.is_empty() {
            return Ok(None);
        }
        match self.folder_id(&parent, false).await? {
            Some(parent_id) => self.find_child(&parent_id, &name, folder).await,
            None => Ok(None),
        }
    }

    /// Forgets the cached ids of `dir` and the folders in it.
    async fn forget_folders(&self, dir: &str) {
        let prefix = format!("{dir}/");
        self.folders
            .lock()
            .await
            .retain(|cached, _| cached != dir && !cached.starts_with(&prefix));
    }

    async fn start_session(
        &self,
        existing_id: Option<&str>,
        parent_id: &str,
        name: &str,
        size: u64,
    ) -> Result<String> {
        let token = self.access_token().await?;
        let resp = send("google: start upload session", || {
            let req = match existing_id {
                Some(id) => self
                    .client
                    .patch(format!("{UPLOAD_API}/{id}"))
                    .json(&json!({})),
                None => self
                    .client
                    .post(UPLOAD_API)
                    .json(&json!({ "name": name, "parents": [parent_id] })),
            };
            Ok(req
                .bearer_auth(&token)
                .query(&[("uploadType", "resumable")])
                .header("X-Upload-Content-Type", "application/octet-stream")
                .header("X-Upload-Content-Length", size)
                .timeout(API_TIMEOUT))
        })
        .await?;
        let resp = ensure_success(resp, format!("Failed to start upload of '{name}'")).await?;
        resp.headers()
            .get(LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .context("Google Drive did not return an upload session URL")
    }

    /// Sends the data from `offset` to the end into the upload session.
    async fn put_from(
        &self,
        session: &str,
        source: &dyn Source,
        offset: u64,
        size: u64,
        progress: &Progress,
    ) -> Result<UploadState, Failure> {
        let body = source_body(source, offset, progress)?;
        let remaining = size - offset;
        let mut req = self
            .client
            .put(session)
            .header(CONTENT_LENGTH, remaining)
            .timeout(upload_timeout(remaining));
        if size > 0 {
            req = req.header(CONTENT_RANGE, format!("bytes {offset}-{}/{size}", size - 1));
        }
        let resp = req.body(body).send().await?;
        classify_upload_response(resp).await
    }

    /// Asks the server how much of the upload it has already received.
    async fn query_offset(&self, session: &str, size: u64) -> Result<UploadState> {
        let resp = send("google: query upload status", || {
            Ok(self
                .client
                .put(session)
                .header(CONTENT_LENGTH, 0)
                .header(CONTENT_RANGE, format!("bytes */{size}"))
                .timeout(API_TIMEOUT))
        })
        .await?;
        classify_upload_response(resp)
            .await
            .map_err(Failure::into_inner)
    }

    async fn upload_resumable(
        &self,
        session: &str,
        source: &dyn Source,
        size: u64,
        progress: &Progress,
    ) -> Result<()> {
        progress.set_total(size);
        progress.set_attempt(1);
        let mut offset = 0;
        let mut attempt = 1;
        loop {
            match self.put_from(session, source, offset, size, progress).await {
                Ok(UploadState::Done) => return Ok(()),
                Ok(UploadState::Incomplete(next)) => {
                    if next <= offset {
                        attempt += 1;
                        if attempt > MAX_ATTEMPTS {
                            bail!("Upload is not making progress at byte {offset}");
                        }
                    }
                    offset = next;
                }
                Err(Failure::Fatal(err)) => return Err(err),
                Err(Failure::Transient(err)) => {
                    if attempt >= MAX_ATTEMPTS {
                        return Err(err);
                    }
                    pause_before_retry("google: upload interrupted", &err, attempt, progress).await;
                    attempt += 1;
                    match self.query_offset(session, size).await? {
                        UploadState::Done => return Ok(()),
                        UploadState::Incomplete(next) => {
                            debug!("google: resuming upload at byte {next} of {size}");
                            offset = next;
                        }
                    }
                }
            }
        }
    }
}

async fn classify_upload_response(resp: Response) -> Result<UploadState, Failure> {
    match resp.status().as_u16() {
        200 | 201 => Ok(UploadState::Done),
        308 => {
            // "Range: bytes=0-12345" lists what the server has persisted.
            let next = resp
                .headers()
                .get(RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(|r| r.rsplit('-').next())
                .and_then(|end| end.trim().parse::<u64>().ok())
                .map_or(0, |end| end + 1);
            Ok(UploadState::Incomplete(next))
        }
        404 | 410 => Err(Failure::Fatal(anyhow!(
            "Google Drive upload session expired"
        ))),
        _ => Err(Failure::from_response(resp, "Google Drive upload failed").await),
    }
}

#[async_trait]
impl StorageProvider for GoogleDriveProvider {
    fn name(&self) -> &str {
        "Google Drive"
    }

    async fn ensure_dir(&self, remote_dir: &str) -> Result<()> {
        self.folder_id(remote_dir, true).await?;
        Ok(())
    }

    async fn get_file_info(&self, remote_path: &str) -> Result<Option<RemoteFileInfo>> {
        let path = normalize_remote_dir(remote_path);
        Ok(self
            .find_item(&path, None)
            .await?
            .map(|file| file.into_info(path)))
    }

    async fn upload(
        &self,
        source: &dyn Source,
        remote_path: &str,
        progress: &Progress,
    ) -> Result<()> {
        let (parent, name) = split_remote(remote_path);
        if name.is_empty() {
            bail!("Invalid remote file path '{remote_path}'");
        }
        let parent_id = self
            .folder_id(&parent, true)
            .await?
            .with_context(|| format!("Failed to resolve folder '{parent}'"))?;
        let existing = self.find_child(&parent_id, &name, Some(false)).await?;
        let size = source.len();

        let session = self
            .start_session(
                existing.as_ref().map(|f| f.id.as_str()),
                &parent_id,
                &name,
                size,
            )
            .await?;
        self.upload_resumable(&session, source, size, progress)
            .await
    }

    async fn download_to(
        &self,
        remote_path: &str,
        sink: &mut dyn Sink,
        progress: &Progress,
    ) -> Result<u64> {
        let path = normalize_remote_dir(remote_path);
        let item = self
            .find_item(&path, Some(false))
            .await?
            .with_context(|| format!("'{path}' does not exist"))?;
        if item
            .mime_type
            .as_deref()
            .is_some_and(|m| m.starts_with("application/vnd.google-apps."))
        {
            bail!("'{path}' is a Google Docs file and cannot be downloaded as is");
        }
        let url = format!("{FILES_API}/{}", item.id);
        download_into(
            &format!("google: download '{path}'"),
            sink,
            progress,
            || async {
                let token = self.access_token().await?;
                Ok(self
                    .client
                    .get(&url)
                    .bearer_auth(token)
                    .query(&[("alt", "media")]))
            },
        )
        .await
    }

    async fn delete(&self, remote_path: &str, is_dir: bool) -> Result<()> {
        let path = normalize_remote_dir(remote_path);
        if path == "/" {
            bail!("Refusing to delete the root folder");
        }
        let Some(item) = self.find_item(&path, Some(is_dir)).await? else {
            return Ok(());
        };
        let token = self.access_token().await?;
        let url = format!("{FILES_API}/{}", item.id);
        let resp = send("google: delete", || {
            Ok(self
                .client
                .delete(&url)
                .bearer_auth(&token)
                .timeout(API_TIMEOUT))
        })
        .await?;
        if !resp.status().is_success() && resp.status() != StatusCode::NOT_FOUND {
            return Err(http_error(resp, format!("Failed to delete '{path}'")).await);
        }
        if is_dir {
            self.forget_folders(&path).await;
        }
        Ok(())
    }

    async fn rename(&self, remote_path: &str, new_name: &str, is_dir: bool) -> Result<()> {
        let path = normalize_remote_dir(remote_path);
        let (parent, new_name) = split_remote(&rename_target(&path, new_name)?);
        let item = self
            .find_item(&path, Some(is_dir))
            .await?
            .with_context(|| format!("'{path}' does not exist"))?;
        // Drive allows equal names in one folder; refuse to create such a pair.
        let parent_id = self
            .folder_id(&parent, false)
            .await?
            .with_context(|| format!("Failed to resolve folder '{parent}'"))?;
        if let Some(other) = self.find_child(&parent_id, &new_name, None).await? {
            if other.id != item.id {
                bail!("'{new_name}' already exists");
            }
        }

        let token = self.access_token().await?;
        let url = format!("{FILES_API}/{}", item.id);
        let resp = send("google: rename", || {
            Ok(self
                .client
                .patch(&url)
                .bearer_auth(&token)
                .json(&json!({ "name": new_name }))
                .timeout(API_TIMEOUT))
        })
        .await?;
        ensure_success(resp, format!("Failed to rename '{path}'")).await?;
        if is_dir {
            self.forget_folders(&path).await;
        }
        Ok(())
    }

    async fn list_dir(&self, remote_dir: &str) -> Result<Vec<RemoteFileInfo>> {
        let dir = normalize_remote_dir(remote_dir);
        let Some(folder_id) = self.folder_id(&dir, false).await? else {
            return Ok(Vec::new());
        };
        let q = format!(
            "'{}' in parents and trashed = false",
            escape_query(&folder_id)
        );
        Ok(self
            .query_files(&q, None)
            .await?
            .into_iter()
            .map(|file| {
                let path = join_remote(&dir, &file.name);
                file.into_info(path)
            })
            .collect())
    }

    async fn share(&self, remote_path: &str, is_dir: bool) -> Result<String> {
        let path = normalize_remote_dir(remote_path);
        let item = self
            .find_item(&path, Some(is_dir))
            .await?
            .with_context(|| format!("'{path}' does not exist"))?;
        let token = self.access_token().await?;
        // Creating the same "anyone" permission again does not duplicate it.
        let permission = json!({ "type": "anyone", "role": "reader" });
        let url = format!("{FILES_API}/{}/permissions", item.id);
        let resp = send("google: share", || {
            Ok(self
                .client
                .post(&url)
                .bearer_auth(&token)
                .query(&[("fields", "id")])
                .json(&permission)
                .timeout(API_TIMEOUT))
        })
        .await?;
        ensure_success(resp, format!("Failed to share '{path}'")).await?;

        let url = format!("{FILES_API}/{}", item.id);
        let resp = send("google: get link", || {
            Ok(self
                .client
                .get(&url)
                .bearer_auth(&token)
                .query(&[("fields", "webViewLink")])
                .timeout(API_TIMEOUT))
        })
        .await?;
        #[derive(Deserialize)]
        struct Link {
            #[serde(rename = "webViewLink")]
            web_view_link: Option<String>,
        }
        let link: Link = json_response(resp, format!("Failed to get the link to '{path}'")).await?;
        link.web_view_link
            .context("Google Drive did not return a link")
    }

    async fn unshare(&self, remote_path: &str, is_dir: bool) -> Result<()> {
        let path = normalize_remote_dir(remote_path);
        let item = self
            .find_item(&path, Some(is_dir))
            .await?
            .with_context(|| format!("'{path}' does not exist"))?;
        let token = self.access_token().await?;
        // The permission for "anyone with the link" has this fixed id.
        let url = format!("{FILES_API}/{}/permissions/anyoneWithLink", item.id);
        let resp = send("google: unshare", || {
            Ok(self
                .client
                .delete(&url)
                .bearer_auth(&token)
                .timeout(API_TIMEOUT))
        })
        .await?;
        if !resp.status().is_success() && resp.status() != StatusCode::NOT_FOUND {
            return Err(http_error(resp, format!("Failed to stop sharing '{path}'")).await);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_query_values() {
        assert_eq!(escape_query("O'Brien.txt"), "O\\'Brien.txt");
        assert_eq!(escape_query("a\\b"), "a\\\\b");
    }
}
