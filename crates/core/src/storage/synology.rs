//! Synology DSM File Station API:
//! https://global.download.synology.com/download/Document/Software/DeveloperGuide/Package/FileStation/All/enu/Synology_File_Station_API_Guide.pdf
//!
//! Paths are the ones File Station shows: the first segment is a shared
//! folder (`/home`, `/photo`, ...). Shared folders are managed in DSM itself,
//! so they cannot be created, renamed or deleted here.

use super::{rename_target, RemoteFileInfo, ShareOptions, StorageProvider};
use crate::client::{
    client_builder, download_into, json_response, retry_transient, send, source_body,
    upload_timeout, Failure, Sink, Source, API_TIMEOUT,
};
use crate::progress::Progress;
use crate::utils::path::{normalize_remote_dir, split_remote};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use chrono::DateTime;
use reqwest::multipart::{Form, Part};
use reqwest::{Client, Response};
use serde::de::{DeserializeOwned, IgnoredAny};
use serde::Deserialize;
use std::fmt;
use std::time::Duration;
use tokio::sync::Mutex;
use url::Url;

const DEFAULT_HTTP_PORT: u16 = 5000;
const DEFAULT_HTTPS_PORT: u16 = 5001;
const PAGE_SIZE: usize = 1000;
/// Deleting a large folder is a single blocking request.
const DELETE_TIMEOUT: Duration = Duration::from_secs(600);

/// File Station error codes this provider reacts to.
const NO_SUCH_FILE: i64 = 408;
const FILE_EXISTS: i64 = 414;
/// The session expired or was replaced: signing in again helps.
const SESSION_ERRORS: [i64; 3] = [106, 107, 119];

pub struct SynologyProvider {
    /// `http(s)://host:port/webapi`
    base: String,
    user: String,
    password: String,
    client: Client,
    /// Session id; signed in lazily and again once the session expires.
    sid: Mutex<Option<String>>,
}

/// A request DSM answered with `"success": false`.
#[derive(Debug)]
struct ApiError {
    what: String,
    code: i64,
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match describe(self.code) {
            Some(text) => write!(f, "{}: {text} (DSM error {})", self.what, self.code),
            None => write!(f, "{}: DSM error {}", self.what, self.code),
        }
    }
}

impl std::error::Error for ApiError {}

fn describe(code: i64) -> Option<&'static str> {
    Some(match code {
        105 => "permission denied",
        106 => "session timed out",
        107 => "session interrupted by another sign-in",
        119 => "session not found",
        400 => "invalid parameter",
        402 => "the system is too busy",
        407 => "operation not permitted",
        408 => "no such file or folder",
        411 => "read-only file system",
        412 | 413 => "file name too long",
        414 => "already exists",
        415 => "disk quota exceeded",
        416 => "no space left on the device",
        417 => "input/output error",
        418 => "illegal name or path",
        419 => "illegal file name",
        421 => "device or resource busy",
        1100 => "failed to create the folder",
        1200 => "failed to rename",
        _ => return None,
    })
}

/// Sign-in errors of `SYNO.API.Auth`, which reuse codes of other meaning.
fn describe_auth(code: i64) -> String {
    match code {
        400 => "wrong user name or password".into(),
        401 => "the account is disabled".into(),
        402 => "permission denied".into(),
        403 | 404 | 406 => "two-factor authentication is not supported".into(),
        407 => "too many failed attempts: this IP address is blocked in DSM".into(),
        408..=410 => "the password has expired, change it in DSM".into(),
        _ => format!("DSM error {code}"),
    }
}

fn error_code(err: &anyhow::Error) -> Option<i64> {
    err.downcast_ref::<ApiError>().map(|e| e.code)
}

fn is_session_error(err: &anyhow::Error) -> bool {
    error_code(err).is_some_and(|code| SESSION_ERRORS.contains(&code))
}

#[derive(Deserialize)]
struct Envelope {
    success: bool,
    data: Option<serde_json::Value>,
    error: Option<ErrorBody>,
}

#[derive(Deserialize)]
struct ErrorBody {
    code: i64,
}

/// Reads a DSM response: the `data` of a successful call or an [`ApiError`].
async fn parse<T: DeserializeOwned>(resp: Response, what: &str) -> Result<T> {
    let envelope: Envelope = json_response(resp, what).await?;
    if !envelope.success {
        return Err(ApiError {
            what: what.to_string(),
            code: envelope.error.map_or(100, |e| e.code),
        }
        .into());
    }
    serde_json::from_value(envelope.data.unwrap_or_default())
        .with_context(|| format!("{what}: invalid response from DSM"))
}

#[derive(Deserialize)]
struct ApiInfo {
    path: String,
    #[serde(rename = "maxVersion")]
    max_version: u32,
}

#[derive(Deserialize)]
struct Session {
    sid: String,
}

#[derive(Deserialize)]
struct File {
    path: String,
    name: String,
    #[serde(default)]
    isdir: bool,
    #[serde(default)]
    additional: Additional,
    /// Set instead of the details for a path that cannot be read.
    code: Option<i64>,
}

#[derive(Default, Deserialize)]
struct Additional {
    size: Option<u64>,
    time: Option<Times>,
}

#[derive(Deserialize)]
struct Times {
    mtime: Option<i64>,
}

impl File {
    fn into_info(self) -> RemoteFileInfo {
        RemoteFileInfo {
            path: normalize_remote_dir(&self.path),
            is_dir: self.isdir,
            size: if self.isdir {
                0
            } else {
                self.additional.size.unwrap_or(0)
            },
            md5: None,
            modified: self
                .additional
                .time
                .and_then(|t| t.mtime)
                .and_then(|secs| DateTime::from_timestamp(secs, 0)),
            name: self.name,
        }
    }
}

#[derive(Deserialize)]
struct Files {
    #[serde(default)]
    files: Vec<File>,
    #[serde(default)]
    total: usize,
}

#[derive(Deserialize)]
struct Shares {
    #[serde(default)]
    shares: Vec<File>,
    #[serde(default)]
    total: usize,
}

#[derive(Deserialize)]
struct Link {
    #[serde(default)]
    id: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    error: i64,
}

#[derive(Deserialize)]
struct Links {
    #[serde(default)]
    links: Vec<Link>,
    #[serde(default)]
    total: usize,
}

/// A parameter that holds a list of paths or names. Plain values are split
/// at commas by DSM, so a JSON array is the only safe form.
fn json_list(items: &[&str]) -> String {
    serde_json::to_string(items).expect("strings serialize")
}

/// `http(s)://host:port/webapi` for what the user entered: a host, or a URL
/// of DSM or of its `/webapi`. A set `port` wins over the one in the address.
/// A bare host uses HTTPS if `https` is set, and DSM's default port.
pub fn api_base(address: &str, port: Option<u16>, https: bool) -> Result<String> {
    let input = address.trim().trim_end_matches('/');
    let input = input.strip_suffix("/webapi").unwrap_or(input);
    let bare = !input.contains("://");
    let scheme = if https { "https" } else { "http" };
    let mut url = Url::parse(&if bare {
        format!("{scheme}://{input}")
    } else {
        input.to_string()
    })
    .with_context(|| format!("Invalid DSM address '{input}'"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("Invalid DSM address '{input}': expected a host or http(s)://host:port");
    }
    let default = (bare && url.port().is_none()).then_some(if https {
        DEFAULT_HTTPS_PORT
    } else {
        DEFAULT_HTTP_PORT
    });
    if let Some(port) = port.or(default) {
        let _ = url.set_port(Some(port));
    }
    Ok(format!("{}/webapi", url.as_str().trim_end_matches('/')))
}

/// Depth of a normalized path: 0 for `/`, 1 for a shared folder.
fn depth(path: &str) -> usize {
    path.split('/').filter(|s| !s.is_empty()).count()
}

impl SynologyProvider {
    /// See [`api_base`] for `address`, `port` and `https`. `insecure` accepts
    /// any HTTPS certificate, such as DSM's self-signed one.
    pub fn new(
        address: &str,
        port: Option<u16>,
        https: bool,
        insecure: bool,
        user: String,
        password: String,
    ) -> Result<Self> {
        Ok(Self {
            base: api_base(address, port, https)?,
            user,
            password,
            client: client_builder()
                .danger_accept_invalid_certs(insecure)
                .build()
                .context("Failed to initialize HTTP client")?,
            sid: Mutex::new(None),
        })
    }

    fn entry(&self) -> String {
        format!("{}/entry.cgi", self.base)
    }

    async fn login(&self) -> Result<String> {
        let resp = send("synology: query API", || {
            Ok(self
                .client
                .get(format!("{}/query.cgi", self.base))
                .query(&[
                    ("api", "SYNO.API.Info"),
                    ("version", "1"),
                    ("method", "query"),
                    ("query", "SYNO.API.Auth"),
                ])
                .timeout(API_TIMEOUT))
        })
        .await
        .map_err(|err| {
            let hint = if format!("{err:#}").contains("certificate") {
                "the HTTPS certificate is not trusted; turn off certificate checks \
                 if this is DSM's own one"
            } else {
                "check the address and the DSM port (Control Panel → Login Portal)"
            };
            err.context(format!("Cannot reach DSM at {}: {hint}", self.base))
        })?;
        let mut apis: std::collections::HashMap<String, ApiInfo> =
            parse(resp, "Failed to query DSM APIs")
                .await
                .with_context(|| format!("{} does not look like Synology DSM", self.base))?;
        let auth = apis
            .remove("SYNO.API.Auth")
            .context("DSM does not offer the sign-in API")?;

        let version = auth.max_version.min(6).to_string();
        let resp = send("synology: sign in", || {
            Ok(self
                .client
                .post(format!("{}/{}", self.base, auth.path))
                .form(&[
                    ("api", "SYNO.API.Auth"),
                    ("version", version.as_str()),
                    ("method", "login"),
                    ("account", self.user.as_str()),
                    ("passwd", self.password.as_str()),
                    ("session", "FileStation"),
                    ("format", "sid"),
                ])
                .timeout(API_TIMEOUT))
        })
        .await?;
        match parse::<Session>(resp, "Sign-in to DSM failed").await {
            Ok(session) => Ok(session.sid),
            Err(err) => match error_code(&err) {
                Some(code) => bail!("Sign-in to DSM failed: {}", describe_auth(code)),
                None => Err(err),
            },
        }
    }

    async fn sid(&self) -> Result<String> {
        let mut sid = self.sid.lock().await;
        if let Some(sid) = &*sid {
            return Ok(sid.clone());
        }
        let fresh = self.login().await?;
        *sid = Some(fresh.clone());
        Ok(fresh)
    }

    /// Forgets the session unless another request has already renewed it.
    async fn drop_sid(&self, stale: &str) {
        let mut sid = self.sid.lock().await;
        if sid.as_deref() == Some(stale) {
            *sid = None;
        }
    }

    /// Calls a File Station method, signing in again once if the session expired.
    async fn call<T: DeserializeOwned>(
        &self,
        what: &str,
        api: &str,
        version: u32,
        method: &str,
        params: &[(&str, String)],
        timeout: Duration,
    ) -> Result<T> {
        let version = version.to_string();
        let mut form: Vec<(&str, &str)> =
            vec![("api", api), ("version", &version), ("method", method)];
        form.extend(params.iter().map(|(k, v)| (*k, v.as_str())));
        let mut renewed = false;
        loop {
            let sid = self.sid().await?;
            let resp = send(&format!("synology: {what}"), || {
                Ok(self
                    .client
                    .post(self.entry())
                    .query(&[("_sid", sid.as_str())])
                    .form(&form)
                    .timeout(timeout))
            })
            .await?;
            match parse(resp, what).await {
                Err(err) if !renewed && is_session_error(&err) => {
                    self.drop_sid(&sid).await;
                    renewed = true;
                }
                result => return result,
            }
        }
    }

    async fn upload_once(
        &self,
        source: &dyn Source,
        remote: &str,
        progress: &Progress,
    ) -> Result<(), Failure> {
        let (dir, name) = split_remote(remote);
        // The name goes into the multipart header as is, as browsers send it.
        if name.contains(['"', '\r', '\n']) {
            return Err(anyhow::anyhow!("DSM cannot upload a file named '{name}'").into());
        }
        let sid = self.sid().await?;
        let len = source.len();
        let part = Part::stream_with_length(source_body(source, 0, progress)?, len)
            .file_name(name)
            .mime_str("application/octet-stream")
            .map_err(anyhow::Error::from)?;
        let form = Form::new()
            .percent_encode_noop()
            .text("path", dir)
            .text("create_parents", "true")
            .text("overwrite", "true")
            .part("file", part);
        let resp = self
            .client
            .post(self.entry())
            .query(&[
                ("api", "SYNO.FileStation.Upload"),
                ("version", "2"),
                ("method", "upload"),
                ("_sid", sid.as_str()),
            ])
            .multipart(form)
            .timeout(upload_timeout(len))
            .send()
            .await?;
        let what = format!("Upload of '{remote}' failed");
        if !resp.status().is_success() {
            return Err(Failure::from_response(resp, what).await);
        }
        match parse::<IgnoredAny>(resp, &what).await {
            Ok(_) => Ok(()),
            Err(err) if is_session_error(&err) => {
                self.drop_sid(&sid).await;
                Err(Failure::Transient(err))
            }
            Err(err) => Err(Failure::Fatal(err)),
        }
    }

    /// The links to `path` the user has created.
    async fn links_of(&self, path: &str) -> Result<Vec<Link>> {
        let mut found = Vec::new();
        let mut offset = 0;
        loop {
            let page: Links = self
                .call(
                    "Failed to list shared links",
                    "SYNO.FileStation.Sharing",
                    3,
                    "list",
                    &[
                        ("offset", offset.to_string()),
                        ("limit", PAGE_SIZE.to_string()),
                    ],
                    API_TIMEOUT,
                )
                .await?;
            let count = page.links.len();
            offset += count;
            found.extend(
                page.links
                    .into_iter()
                    .filter(|l| normalize_remote_dir(&l.path) == path),
            );
            if count == 0 || offset >= page.total {
                return Ok(found);
            }
        }
    }

    async fn create_link(&self, path: &str, options: &ShareOptions) -> Result<String> {
        let mut params = vec![("path", json_list(&[path]))];
        if let Some(password) = &options.password {
            params.push(("password", password.clone()));
        }
        if let Some(date) = options.expires {
            params.push(("date_expired", date.format("%Y-%m-%d").to_string()));
        }
        let what = format!("Failed to share '{path}'");
        let created: Links = self
            .call(
                &what,
                "SYNO.FileStation.Sharing",
                3,
                "create",
                &params,
                API_TIMEOUT,
            )
            .await?;
        let link = created
            .links
            .into_iter()
            .next()
            .context("DSM did not return a link")?;
        if link.error != 0 {
            return Err(ApiError {
                what,
                code: link.error,
            }
            .into());
        }
        if link.url.is_empty() {
            bail!("DSM did not return a link");
        }
        Ok(link.url)
    }

    async fn delete_links(&self, links: &[Link]) -> Result<()> {
        if links.is_empty() {
            return Ok(());
        }
        let ids: Vec<&str> = links.iter().map(|l| l.id.as_str()).collect();
        self.call::<IgnoredAny>(
            "Failed to delete the link",
            "SYNO.FileStation.Sharing",
            3,
            "delete",
            &[("id", ids.join(","))],
            API_TIMEOUT,
        )
        .await?;
        Ok(())
    }
}

#[async_trait]
impl StorageProvider for SynologyProvider {
    fn name(&self) -> &str {
        "Synology"
    }

    async fn ensure_dir(&self, remote_dir: &str) -> Result<()> {
        let dir = normalize_remote_dir(remote_dir);
        match depth(&dir) {
            0 => Ok(()),
            1 => match self.get_file_info(&dir).await? {
                Some(info) if info.is_dir => Ok(()),
                _ => {
                    bail!("Shared folder '{dir}' does not exist: shared folders are created in DSM")
                }
            },
            _ => {
                let (parent, name) = split_remote(&dir);
                self.call::<IgnoredAny>(
                    &format!("Failed to create folder '{dir}'"),
                    "SYNO.FileStation.CreateFolder",
                    2,
                    "create",
                    &[
                        ("folder_path", json_list(&[&parent])),
                        ("name", json_list(&[&name])),
                        ("force_parent", "true".into()),
                    ],
                    API_TIMEOUT,
                )
                .await?;
                Ok(())
            }
        }
    }

    async fn get_file_info(&self, remote_path: &str) -> Result<Option<RemoteFileInfo>> {
        let path = normalize_remote_dir(remote_path);
        if path == "/" {
            return Ok(Some(RemoteFileInfo::dir(path)));
        }
        let result = self
            .call::<Files>(
                &format!("Failed to get info for '{path}'"),
                "SYNO.FileStation.List",
                2,
                "getinfo",
                &[
                    ("path", json_list(&[&path])),
                    ("additional", json_list(&["size", "time"])),
                ],
                API_TIMEOUT,
            )
            .await;
        match result {
            Ok(files) => Ok(files
                .files
                .into_iter()
                .find(|f| f.code.is_none())
                .map(File::into_info)),
            Err(err) if error_code(&err) == Some(NO_SUCH_FILE) => Ok(None),
            Err(err) => Err(err),
        }
    }

    async fn upload(
        &self,
        source: &dyn Source,
        remote_path: &str,
        progress: &Progress,
    ) -> Result<()> {
        let path = normalize_remote_dir(remote_path);
        if depth(&path) < 2 {
            bail!("Files are stored inside shared folders, not next to them");
        }
        retry_transient(&format!("synology: upload '{path}'"), progress, || {
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
        // DSM reports a missing file as JSON with status 200, which would
        // otherwise end up in the file; this also renews an expired session.
        match self.get_file_info(&path).await? {
            None => bail!("'{path}' does not exist"),
            Some(info) if info.is_dir => bail!("'{path}' is a folder"),
            Some(_) => {}
        }
        let sid = self.sid().await?;
        let paths = json_list(&[&path]);
        download_into(
            &format!("synology: download '{path}'"),
            sink,
            progress,
            || async {
                Ok(self.client.get(self.entry()).query(&[
                    ("api", "SYNO.FileStation.Download"),
                    ("version", "2"),
                    ("method", "download"),
                    ("path", paths.as_str()),
                    ("mode", "download"),
                    ("_sid", sid.as_str()),
                ]))
            },
        )
        .await
    }

    async fn delete(&self, remote_path: &str, _is_dir: bool) -> Result<()> {
        let path = normalize_remote_dir(remote_path);
        if depth(&path) < 2 {
            bail!("Shared folders are deleted in DSM");
        }
        let result = self
            .call::<IgnoredAny>(
                &format!("Failed to delete '{path}'"),
                "SYNO.FileStation.Delete",
                2,
                "delete",
                &[("path", json_list(&[&path])), ("recursive", "true".into())],
                DELETE_TIMEOUT,
            )
            .await;
        match result {
            Err(err) if error_code(&err) != Some(NO_SUCH_FILE) => Err(err),
            _ => Ok(()),
        }
    }

    async fn rename(&self, remote_path: &str, new_name: &str, _is_dir: bool) -> Result<()> {
        let from = normalize_remote_dir(remote_path);
        if depth(&from) < 2 {
            bail!("Shared folders are renamed in DSM");
        }
        let (_, name) = split_remote(&rename_target(&from, new_name)?);
        let result = self
            .call::<IgnoredAny>(
                &format!("Failed to rename '{from}'"),
                "SYNO.FileStation.Rename",
                2,
                "rename",
                &[("path", json_list(&[&from])), ("name", json_list(&[&name]))],
                API_TIMEOUT,
            )
            .await;
        match result {
            Err(err) if error_code(&err) == Some(FILE_EXISTS) => bail!("'{name}' already exists"),
            Err(err) => Err(err),
            Ok(_) => Ok(()),
        }
    }

    async fn list_dir(&self, remote_dir: &str) -> Result<Vec<RemoteFileInfo>> {
        let dir = normalize_remote_dir(remote_dir);
        let mut items = Vec::new();
        loop {
            let offset = items.len().to_string();
            let limit = PAGE_SIZE.to_string();
            let (page, total) = if dir == "/" {
                let shares: Shares = self
                    .call(
                        "Failed to list shared folders",
                        "SYNO.FileStation.List",
                        2,
                        "list_share",
                        &[
                            ("offset", offset),
                            ("limit", limit),
                            ("additional", json_list(&["time"])),
                        ],
                        API_TIMEOUT,
                    )
                    .await?;
                (shares.shares, shares.total)
            } else {
                let result = self
                    .call::<Files>(
                        &format!("Failed to list '{dir}'"),
                        "SYNO.FileStation.List",
                        2,
                        "list",
                        &[
                            ("folder_path", dir.clone()),
                            ("offset", offset),
                            ("limit", limit),
                            ("additional", json_list(&["size", "time"])),
                        ],
                        API_TIMEOUT,
                    )
                    .await;
                match result {
                    Ok(files) => (files.files, files.total),
                    Err(err) if error_code(&err) == Some(NO_SUCH_FILE) => break,
                    Err(err) => return Err(err),
                }
            };
            let count = page.len();
            items.extend(page.into_iter().map(File::into_info));
            if count == 0 || items.len() >= total {
                break;
            }
        }
        Ok(items)
    }

    async fn share(&self, remote_path: &str, _is_dir: bool) -> Result<String> {
        let path = normalize_remote_dir(remote_path);
        if let Some(link) = self.links_of(&path).await?.into_iter().next() {
            return Ok(link.url);
        }
        self.create_link(&path, &ShareOptions::default()).await
    }

    async fn share_with(
        &self,
        remote_path: &str,
        _is_dir: bool,
        options: &ShareOptions,
    ) -> Result<String> {
        let path = normalize_remote_dir(remote_path);
        self.delete_links(&self.links_of(&path).await?).await?;
        self.create_link(&path, options).await
    }

    async fn unshare(&self, remote_path: &str, _is_dir: bool) -> Result<()> {
        let path = normalize_remote_dir(remote_path);
        self.delete_links(&self.links_of(&path).await?).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_base_accepts_addresses_and_urls() {
        let base = |address, port, https| api_base(address, port, https).unwrap();
        assert_eq!(
            base("192.168.1.10", None, false),
            "http://192.168.1.10:5000/webapi"
        );
        assert_eq!(
            base("192.168.1.10", None, true),
            "https://192.168.1.10:5001/webapi"
        );
        assert_eq!(
            base(" 192.168.1.10/ ", Some(5601), true),
            "https://192.168.1.10:5601/webapi"
        );
        assert_eq!(
            base("nas.local:5600", None, false),
            "http://nas.local:5600/webapi"
        );
        assert_eq!(
            base("https://nas.example.com/", None, false),
            "https://nas.example.com/webapi"
        );
        assert_eq!(
            base("http://nas:5000/webapi/", Some(5600), true),
            "http://nas:5600/webapi"
        );
        assert!(api_base("ftp://nas", None, false).is_err());
        assert!(api_base("", None, false).is_err());
    }

    #[test]
    fn lists_are_json_arrays() {
        assert_eq!(json_list(&["/home/a,b \"c\""]), r#"["/home/a,b \"c\""]"#);
    }

    #[test]
    fn parses_file_station_items() {
        let files: Files = serde_json::from_str(
            r#"{"files":[{"isdir":false,"name":"a.txt","path":"/home/a.txt",
                "additional":{"size":12,"time":{"mtime":1700000000}}},
                {"code":408,"name":"x","path":"/home/x"}],"total":2}"#,
        )
        .unwrap();
        let info: Vec<_> = files.files.into_iter().map(File::into_info).collect();
        assert_eq!(info[0].size, 12);
        assert_eq!(info[0].modified.unwrap().timestamp(), 1_700_000_000);
        assert!(!info[1].is_dir);
    }
}
