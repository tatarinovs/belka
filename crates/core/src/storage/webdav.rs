//! Generic WebDAV (RFC 4918): Nextcloud, ownCloud, Synology, Mail.ru Cloud, etc.

use super::{
    as_md5, base_name, create_dirs, parse_http_date, rename_target, Mkdir, RemoteFileInfo,
    StorageProvider,
};
use crate::client::{
    build_http_client, download_into, ensure_success, http_error, send, send_with, source_body,
    upload_timeout, RetryPolicy, Sink, Source, API_TIMEOUT,
};
use crate::progress::Progress;
use crate::utils::path::{encode_component, normalize_remote_dir};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use percent_encoding::percent_decode_str;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;
use quick_xml::Reader;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE};
use reqwest::{Client, Method, RequestBuilder, StatusCode};
use tracing::{debug, warn};
use url::Url;

const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:">
  <D:prop>
    <D:resourcetype/>
    <D:getcontentlength/>
    <D:getetag/>
    <D:getlastmodified/>
  </D:prop>
</D:propfind>"#;

pub struct WebDavProvider {
    display_name: &'static str,
    /// Base URL as configured (already encoded), without trailing slash.
    base_url: String,
    /// Decoded path component of the base URL without trailing slash ("" for root).
    base_path: String,
    user: String,
    password: String,
    /// Whether ETags are MD5 content hashes. True for very few servers.
    trust_etag_as_md5: bool,
    client: Client,
}

impl WebDavProvider {
    pub fn new(
        display_name: &'static str,
        base_url: &str,
        user: String,
        password: String,
        trust_etag_as_md5: bool,
    ) -> Result<Self> {
        let mut url = Url::parse(base_url.trim())
            .with_context(|| format!("Invalid WebDAV URL '{base_url}'"))?;
        match url.scheme() {
            "https" => {}
            "http" => {
                warn!("WebDAV URL uses plain HTTP: credentials and data are sent unencrypted")
            }
            other => bail!("Unsupported WebDAV URL scheme '{other}'"),
        }
        url.set_query(None);
        url.set_fragment(None);

        let base_path = percent_decode_str(url.path())
            .decode_utf8_lossy()
            .trim_end_matches('/')
            .to_string();

        Ok(Self {
            display_name,
            base_url: url.as_str().trim_end_matches('/').to_string(),
            base_path,
            user,
            password,
            trust_etag_as_md5,
            client: build_http_client()?,
        })
    }

    fn url_for(&self, remote_path: &str, collection: bool) -> String {
        let mut url = self.base_url.clone();
        let mut has_segments = false;
        for segment in remote_path.split('/').filter(|s| !s.is_empty()) {
            url.push('/');
            url.push_str(&encode_component(segment));
            has_segments = true;
        }
        if collection || !has_segments {
            url.push('/');
        }
        url
    }

    fn request(&self, method: Method, url: &str) -> RequestBuilder {
        self.client
            .request(method, url)
            .basic_auth(&self.user, Some(&self.password))
    }

    fn propfind(&self, url: &str, depth: &'static str) -> RequestBuilder {
        self.request(Method::from_bytes(b"PROPFIND").expect("valid method"), url)
            .header("Depth", depth)
            .header(CONTENT_TYPE, "application/xml; charset=utf-8")
            .body(PROPFIND_BODY)
            .timeout(API_TIMEOUT)
    }

    /// Maps an `href` from a multistatus response to a normalized remote path.
    fn href_to_remote(&self, href: &str) -> Option<String> {
        let raw_path = match Url::parse(href) {
            Ok(url) => url.path().to_string(),
            Err(_) => href
                .split(['?', '#'])
                .next()
                .unwrap_or_default()
                .to_string(),
        };
        let decoded = percent_decode_str(&raw_path).decode_utf8().ok()?;
        let relative = decoded.strip_prefix(self.base_path.as_str())?;
        if !relative.is_empty() && !relative.starts_with('/') {
            return None;
        }
        Some(normalize_remote_dir(relative))
    }

    /// Creates one folder with MKCOL.
    async fn mkcol(&self, path: String) -> Result<Mkdir> {
        let url = self.url_for(&path, true);
        let mkcol = Method::from_bytes(b"MKCOL").expect("valid method");
        let resp = send("webdav: MKCOL", || {
            Ok(self.request(mkcol.clone(), &url).timeout(API_TIMEOUT))
        })
        .await?;
        match resp.status() {
            // 405: the collection already exists.
            StatusCode::CREATED | StatusCode::OK | StatusCode::METHOD_NOT_ALLOWED => {
                Ok(Mkdir::Done)
            }
            StatusCode::CONFLICT => Ok(Mkdir::ParentMissing),
            status => {
                // Some servers answer oddly for existing folders: double-check.
                if matches!(self.get_file_info(&path).await, Ok(Some(i)) if i.is_dir) {
                    debug!("MKCOL '{path}' returned {status}, but the folder exists");
                    return Ok(Mkdir::Done);
                }
                Err(http_error(resp, format!("Failed to create folder '{path}'")).await)
            }
        }
    }

    fn to_info(&self, entry: DavEntry, path: String) -> RemoteFileInfo {
        let md5 = if self.trust_etag_as_md5 {
            entry.etag.as_deref().and_then(as_md5)
        } else {
            None
        };
        RemoteFileInfo {
            name: base_name(&path).to_string(),
            path,
            is_dir: entry.is_dir,
            size: entry.size.unwrap_or(0),
            md5,
            modified: entry.modified.as_deref().and_then(parse_http_date),
        }
    }
}

#[async_trait]
impl StorageProvider for WebDavProvider {
    fn name(&self) -> &str {
        self.display_name
    }

    async fn ensure_dir(&self, remote_dir: &str) -> Result<()> {
        let dir = normalize_remote_dir(remote_dir);
        if dir == "/" {
            return Ok(());
        }
        match self.get_file_info(&dir).await? {
            Some(info) if info.is_dir => return Ok(()),
            Some(_) => bail!("'{dir}' exists but is a file"),
            None => {}
        }

        create_dirs(&dir, |path| self.mkcol(path)).await
    }

    async fn get_file_info(&self, remote_path: &str) -> Result<Option<RemoteFileInfo>> {
        let path = normalize_remote_dir(remote_path);
        let url = self.url_for(&path, false);
        let policy = RetryPolicy {
            retry_server_errors: false,
            ..RetryPolicy::default()
        };
        let resp = send_with("webdav: PROPFIND", policy, || Ok(self.propfind(&url, "0"))).await?;

        let status = resp.status();
        // Mail.ru answers 500 (and some servers 400) for paths that do not exist.
        if matches!(
            status,
            StatusCode::NOT_FOUND
                | StatusCode::GONE
                | StatusCode::BAD_REQUEST
                | StatusCode::INTERNAL_SERVER_ERROR
        ) {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(http_error(resp, format!("PROPFIND '{path}' failed")).await);
        }

        let xml = resp
            .text()
            .await
            .context("Failed to read PROPFIND response")?;
        let entry = parse_multistatus(&xml)?.into_iter().next();
        Ok(entry.map(|e| self.to_info(e, path)))
    }

    async fn upload(
        &self,
        source: &dyn Source,
        remote_path: &str,
        progress: &Progress,
    ) -> Result<()> {
        let path = normalize_remote_dir(remote_path);
        let url = self.url_for(&path, false);
        // Nextcloud and ownCloud keep the original file's date from this
        // header; other servers ignore it.
        let mtime = source
            .modified()
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs().to_string());
        let resp = send(&format!("webdav: upload '{path}'"), || {
            let (body, len) = (source_body(source, 0, progress)?, source.len());
            let mut req = self
                .request(Method::PUT, &url)
                .header(CONTENT_TYPE, "application/octet-stream")
                .header(CONTENT_LENGTH, len)
                .timeout(upload_timeout(len));
            if let Some(mtime) = &mtime {
                req = req.header("X-OC-Mtime", mtime);
            }
            Ok(req.body(body))
        })
        .await?;
        match resp.status() {
            StatusCode::CREATED | StatusCode::OK | StatusCode::NO_CONTENT => Ok(()),
            _ => Err(http_error(resp, format!("Upload of '{path}' failed")).await),
        }
    }

    async fn download_to(
        &self,
        remote_path: &str,
        sink: &mut dyn Sink,
        progress: &Progress,
    ) -> Result<u64> {
        let path = normalize_remote_dir(remote_path);
        let url = self.url_for(&path, false);
        download_into(
            &format!("webdav: download '{path}'"),
            sink,
            progress,
            || async { Ok(self.request(Method::GET, &url)) },
        )
        .await
    }

    async fn delete(&self, remote_path: &str, is_dir: bool) -> Result<()> {
        let path = normalize_remote_dir(remote_path);
        if path == "/" {
            bail!("Refusing to delete the root folder");
        }
        let url = self.url_for(&path, is_dir);
        let resp = send("webdav: DELETE", || {
            Ok(self.request(Method::DELETE, &url).timeout(API_TIMEOUT))
        })
        .await?;
        match resp.status() {
            StatusCode::OK
            | StatusCode::NO_CONTENT
            | StatusCode::ACCEPTED
            | StatusCode::NOT_FOUND => Ok(()),
            _ => Err(http_error(resp, format!("Failed to delete '{path}'")).await),
        }
    }

    async fn rename(&self, remote_path: &str, new_name: &str, is_dir: bool) -> Result<()> {
        let from = normalize_remote_dir(remote_path);
        let to = rename_target(&from, new_name)?;
        let source = self.url_for(&from, is_dir);
        let destination = self.url_for(&to, is_dir);
        let resp = send("webdav: MOVE", || {
            Ok(self
                .request(Method::from_bytes(b"MOVE").expect("valid method"), &source)
                .header("Destination", &destination)
                .header("Overwrite", "F")
                .timeout(API_TIMEOUT))
        })
        .await?;
        match resp.status() {
            StatusCode::CREATED | StatusCode::NO_CONTENT | StatusCode::OK => Ok(()),
            StatusCode::PRECONDITION_FAILED => bail!("'{}' already exists", new_name.trim()),
            _ => Err(http_error(resp, format!("Failed to rename '{from}'")).await),
        }
    }

    async fn list_dir(&self, remote_dir: &str) -> Result<Vec<RemoteFileInfo>> {
        let dir = normalize_remote_dir(remote_dir);
        let url = self.url_for(&dir, true);
        let resp = send("webdav: PROPFIND", || Ok(self.propfind(&url, "1"))).await?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        let resp = ensure_success(resp, format!("Failed to list '{dir}'")).await?;
        let xml = resp
            .text()
            .await
            .context("Failed to read PROPFIND response")?;

        let mut items = Vec::new();
        for entry in parse_multistatus(&xml)? {
            let Some(path) = self.href_to_remote(&entry.href) else {
                warn!(
                    "Ignoring unexpected href '{}' outside of '{}'",
                    entry.href, self.base_url
                );
                continue;
            };
            // The collection itself is part of a Depth: 1 response.
            if path == dir {
                continue;
            }
            items.push(self.to_info(entry, path));
        }
        Ok(items)
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
struct DavEntry {
    href: String,
    is_dir: bool,
    size: Option<u64>,
    etag: Option<String>,
    modified: Option<String>,
}

#[derive(Default)]
struct PropStat {
    ok: bool,
    props: DavEntry,
}

/// Parses a `207 Multi-Status` body. Namespace prefixes are ignored and only
/// properties from `propstat` blocks with a 2xx status are taken into account.
fn parse_multistatus(xml: &str) -> Result<Vec<DavEntry>> {
    let mut reader = Reader::from_str(xml);

    let mut entries = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut entry: Option<DavEntry> = None;
    let mut propstat: Option<PropStat> = None;
    // Text of the current element: the reader splits it into Text/CData
    // pieces and separate events for entity references (`&quot;`, `&#38;`).
    let mut text = String::new();

    loop {
        let event = reader
            .read_event()
            .context("Malformed WebDAV XML response")?;
        let (name, closes) = match &event {
            Event::Start(e) => (local_name(e.local_name().as_ref()), false),
            Event::Empty(e) => (local_name(e.local_name().as_ref()), true),
            Event::End(_) => {
                if !text.trim().is_empty() {
                    set_text(&stack, &text, &mut entry, &mut propstat);
                }
                text.clear();
                let name = stack.pop().unwrap_or_default();
                close_element(&name, &mut entry, &mut propstat, &mut entries);
                continue;
            }
            Event::Text(t) => {
                text.push_str(&t.xml10_content());
                continue;
            }
            Event::CData(c) => {
                text.push_str(&c.xml10_content());
                continue;
            }
            Event::GeneralRef(r) => {
                match r.resolve_char_ref().context("Malformed WebDAV XML text")? {
                    Some(ch) => text.push(ch),
                    None => text.push_str(
                        resolve_predefined_entity(r)
                            .with_context(|| format!("Unknown XML entity &{};", &**r))?,
                    ),
                }
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };

        text.clear();
        match name.as_str() {
            "response" => entry = Some(DavEntry::default()),
            "propstat" => {
                propstat = Some(PropStat {
                    ok: true,
                    ..PropStat::default()
                })
            }
            "collection" if stack.last().is_some_and(|p| p == "resourcetype") => {
                if let Some(ps) = propstat.as_mut() {
                    ps.props.is_dir = true;
                }
            }
            _ => {}
        }
        if closes {
            close_element(&name, &mut entry, &mut propstat, &mut entries);
        } else {
            stack.push(name);
        }
    }
    Ok(entries)
}

fn local_name(raw: &str) -> String {
    raw.to_ascii_lowercase()
}

fn close_element(
    name: &str,
    entry: &mut Option<DavEntry>,
    propstat: &mut Option<PropStat>,
    entries: &mut Vec<DavEntry>,
) {
    match name {
        "propstat" => {
            if let (Some(ps), Some(e)) = (propstat.take(), entry.as_mut()) {
                if ps.ok {
                    e.is_dir |= ps.props.is_dir;
                    e.size = e.size.or(ps.props.size);
                    e.etag = e.etag.take().or(ps.props.etag);
                    e.modified = e.modified.take().or(ps.props.modified);
                }
            }
        }
        "response" => {
            if let Some(e) = entry.take() {
                entries.push(e);
            }
        }
        _ => {}
    }
}

fn set_text(
    stack: &[String],
    text: &str,
    entry: &mut Option<DavEntry>,
    propstat: &mut Option<PropStat>,
) {
    let Some(current) = stack.last() else { return };
    let parent = stack.len().checked_sub(2).map(|i| stack[i].as_str());
    match (current.as_str(), propstat.as_mut()) {
        ("href", _) if parent == Some("response") => {
            if let Some(e) = entry.as_mut() {
                e.href = text.trim().to_string();
            }
        }
        ("status", Some(ps)) if parent == Some("propstat") => {
            // e.g. "HTTP/1.1 200 OK"
            ps.ok = text
                .split_whitespace()
                .nth(1)
                .is_some_and(|code| code.starts_with('2'));
        }
        ("getcontentlength", Some(ps)) => ps.props.size = text.trim().parse().ok(),
        ("getetag", Some(ps)) => ps.props.etag = Some(text.trim().to_string()),
        ("getlastmodified", Some(ps)) => ps.props.modified = Some(text.trim().to_string()),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(base: &str) -> WebDavProvider {
        WebDavProvider::new("test", base, "u".into(), "p".into(), false).unwrap()
    }

    #[test]
    fn builds_encoded_urls() {
        let p = provider("https://cloud.example.com/remote.php/dav/files/user/");
        assert_eq!(
            p.url_for("/Upload/[v1.0] my app.apk", false),
            "https://cloud.example.com/remote.php/dav/files/user/Upload/%5Bv1.0%5D%20my%20app.apk"
        );
        assert_eq!(
            p.url_for("/Папка", true),
            "https://cloud.example.com/remote.php/dav/files/user/%D0%9F%D0%B0%D0%BF%D0%BA%D0%B0/"
        );
        assert_eq!(
            p.url_for("/", false),
            "https://cloud.example.com/remote.php/dav/files/user/"
        );
        assert_eq!(
            provider("https://webdav.mail.ru").url_for("/a+b", false),
            "https://webdav.mail.ru/a%2Bb"
        );
    }

    #[test]
    fn maps_hrefs_relative_to_base() {
        let p = provider("https://cloud.example.com/remote.php/dav/files/user");
        assert_eq!(
            p.href_to_remote("/remote.php/dav/files/user/Upload/%5Bv1%5D%20a.apk")
                .as_deref(),
            Some("/Upload/[v1] a.apk")
        );
        assert_eq!(
            p.href_to_remote("https://cloud.example.com/remote.php/dav/files/user/Upload/")
                .as_deref(),
            Some("/Upload")
        );
        assert_eq!(p.href_to_remote("/remote.php/dav/files/user2/x"), None);

        let root = provider("https://webdav.mail.ru");
        assert_eq!(
            root.href_to_remote("/Upload/a%2Bb.txt").as_deref(),
            Some("/Upload/a+b.txt")
        );
    }

    #[test]
    fn parses_entities_inside_text() {
        let xml = r#"<d:multistatus xmlns:d="DAV:"><d:response>
    <d:href>/Upload/Tom &amp; Jerry &#x2014; <![CDATA[<1>]]>.txt</d:href>
    <d:propstat><d:prop><d:getetag></d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
</d:response></d:multistatus>"#;
        let entries = parse_multistatus(xml).unwrap();
        assert_eq!(entries[0].href, "/Upload/Tom & Jerry \u{2014} <1>.txt");
        assert_eq!(entries[0].etag, None);
    }

    #[test]
    fn parses_multistatus_with_multiple_propstats() {
        let xml = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns">
  <d:response>
    <d:href>/remote.php/dav/files/user/Upload/</d:href>
    <d:propstat>
      <d:prop><d:resourcetype><d:collection/></d:resourcetype><d:getetag>"abc"</d:getetag></d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
    <d:propstat>
      <d:prop><d:getcontentlength/></d:prop>
      <d:status>HTTP/1.1 404 Not Found</d:status>
    </d:propstat>
  </d:response>
  <d:response>
    <d:href>/remote.php/dav/files/user/Upload/a%20b.zip</d:href>
    <d:propstat>
      <d:prop>
        <d:resourcetype/>
        <d:getcontentlength>1234</d:getcontentlength>
        <d:getlastmodified>Tue, 15 Nov 1994 12:45:26 GMT</d:getlastmodified>
        <d:getetag>&quot;d41d8cd98f00b204e9800998ecf8427e&quot;</d:getetag>
      </d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
  </d:response>
</d:multistatus>"#;
        let entries = parse_multistatus(xml).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].is_dir);
        assert_eq!(entries[0].size, None);
        assert!(!entries[1].is_dir);
        assert_eq!(entries[1].size, Some(1234));
        assert_eq!(
            entries[1].etag.as_deref(),
            Some("\"d41d8cd98f00b204e9800998ecf8427e\"")
        );

        let p = provider("https://cloud.example.com/remote.php/dav/files/user");
        let info = p.to_info(entries[1].clone(), "/Upload/a b.zip".into());
        assert_eq!(info.name, "a b.zip");
        assert!(info.modified.is_some());
        assert_eq!(info.md5, None, "ETag must not be trusted as MD5 by default");
    }
}
