//! Interactive OAuth sign-in for Google Drive and Yandex Disk: the user signs
//! in in the browser, which redirects back to a short-lived listener on
//! 127.0.0.1 (RFC 8252, with PKCE).
//!
//! Both services need a registered OAuth app. Its id can be built in through
//! the `BELKA_GOOGLE_CLIENT_ID` / `..._SECRET` and
//! `BELKA_YANDEX_CLIENT_ID` / `..._SECRET` variables at compile time,
//! or supplied by the user.

use crate::client::{build_http_client, ensure_success, send, API_TIMEOUT};
use crate::storage::google::GoogleAuth;
use crate::ProviderConfig;
use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::debug;
use url::Url;

/// Yandex only accepts redirect URIs registered with the app, so its port is
/// fixed; register `http://127.0.0.1:53682/` as the Redirect URI.
pub const YANDEX_REDIRECT_PORT: u16 = 53682;

/// How long to wait for the user to finish signing in.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_SCOPE: &str = "https://www.googleapis.com/auth/drive";
const YANDEX_AUTH_URL: &str = "https://oauth.yandex.ru/authorize";
const YANDEX_TOKEN_URL: &str = "https://oauth.yandex.ru/token";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Service {
    Google,
    Yandex,
}

impl Service {
    pub fn title(self) -> &'static str {
        match self {
            Self::Google => "Google Drive",
            Self::Yandex => "Яндекс.Диск",
        }
    }

    /// The OAuth app built into this binary, if any.
    pub fn builtin_app(self) -> Option<OAuthApp> {
        let (id, secret) = match self {
            Self::Google => (
                option_env!("BELKA_GOOGLE_CLIENT_ID"),
                option_env!("BELKA_GOOGLE_CLIENT_SECRET"),
            ),
            Self::Yandex => (
                option_env!("BELKA_YANDEX_CLIENT_ID"),
                option_env!("BELKA_YANDEX_CLIENT_SECRET"),
            ),
        };
        Some(OAuthApp {
            client_id: id.filter(|s| !s.is_empty())?.to_string(),
            client_secret: secret.unwrap_or_default().to_string(),
        })
    }
}

/// A registered OAuth client. Google "Desktop app" clients need the secret
/// (it is not confidential for this client type); Yandex works without it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthApp {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
}

pub struct SignedIn {
    pub config: ProviderConfig,
    /// The account's e-mail or login, when the service reports it.
    pub label: Option<String>,
}

/// Runs the whole sign-in: opens the consent page through `open_browser`,
/// waits for the redirect, exchanges the code and identifies the account.
/// Dropping the future stops the listener.
pub async fn sign_in(
    service: Service,
    app: &OAuthApp,
    open_browser: impl FnOnce(&str) -> Result<()>,
) -> Result<SignedIn> {
    let client_id = app.client_id.trim();
    let client_secret = app.client_secret.trim();
    if client_id.is_empty() {
        bail!("Client ID is not set");
    }
    if service == Service::Google && client_secret.is_empty() {
        bail!("Google requires the client secret of a Desktop app OAuth client");
    }

    let port = match service {
        Service::Google => 0,
        Service::Yandex => YANDEX_REDIRECT_PORT,
    };
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .with_context(|| {
            format!("Cannot listen on 127.0.0.1:{port}; is it used by another program?")
        })?;
    let redirect_uri = format!("http://127.0.0.1:{}/", listener.local_addr()?.port());

    let verifier = random_token(32)?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_token(16)?;
    let mut params = vec![
        ("response_type", "code"),
        ("client_id", client_id),
        ("redirect_uri", redirect_uri.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("state", state.as_str()),
    ];
    let auth_url = match service {
        Service::Google => {
            // offline + consent: always return a refresh token.
            params.extend([
                ("scope", GOOGLE_SCOPE),
                ("access_type", "offline"),
                ("prompt", "consent"),
            ]);
            GOOGLE_AUTH_URL
        }
        Service::Yandex => {
            params.push(("force_confirm", "yes"));
            YANDEX_AUTH_URL
        }
    };
    let url = Url::parse_with_params(auth_url, &params)?;
    open_browser(url.as_str()).context("Failed to open the browser")?;

    let code = tokio::time::timeout(SIGN_IN_TIMEOUT, wait_for_code(&listener, &state))
        .await
        .map_err(|_| anyhow!("Sign-in was not completed within 5 minutes"))??;
    drop(listener);

    let http = build_http_client()?;
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("client_id", client_id),
        ("redirect_uri", redirect_uri.as_str()),
        ("code_verifier", verifier.as_str()),
    ];
    if !client_secret.is_empty() {
        form.push(("client_secret", client_secret));
    }
    let token_url = match service {
        Service::Google => GOOGLE_TOKEN_URL,
        Service::Yandex => YANDEX_TOKEN_URL,
    };
    let resp = send("oauth: exchange code", || {
        Ok(http.post(token_url).form(&form).timeout(API_TIMEOUT))
    })
    .await?;
    let tokens: Tokens = ensure_success(resp, "Failed to obtain the access token")
        .await?
        .json()
        .await
        .context("Invalid token response")?;

    match service {
        Service::Google => {
            let refresh_token = tokens
                .refresh_token
                .context("Google did not return a refresh token")?;
            let label = google_email(&http, &tokens.access_token).await;
            Ok(SignedIn {
                config: ProviderConfig::Google(GoogleAuth::RefreshToken {
                    client_id: client_id.to_string(),
                    client_secret: client_secret.to_string(),
                    refresh_token,
                }),
                label,
            })
        }
        Service::Yandex => {
            // Yandex tokens live for a year; after that the user signs in again.
            let label = yandex_login(&http, &tokens.access_token).await;
            Ok(SignedIn {
                config: ProviderConfig::Yandex {
                    token: tokens.access_token,
                },
                label,
            })
        }
    }
}

#[derive(Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
}

fn random_token(bytes: usize) -> Result<String> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|err| anyhow!("No secure random source: {err}"))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

/// Answers requests on the loopback listener until the OAuth redirect arrives.
async fn wait_for_code(listener: &TcpListener, state: &str) -> Result<String> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        let Some(target) = read_request_target(&mut stream).await else {
            continue;
        };
        let url = Url::parse(&format!("http://127.0.0.1{target}"))?;
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();

        // Browsers also ask for /favicon.ico and the like.
        if !query.contains_key("code") && !query.contains_key("error") {
            respond(&mut stream, "404 Not Found", "").await;
            continue;
        }
        if query.get("state").map(String::as_str) != Some(state) {
            respond(
                &mut stream,
                "400 Bad Request",
                &page("Неверный ответ сервера авторизации."),
            )
            .await;
            bail!("OAuth state mismatch; the sign-in may have been tampered with");
        }
        if let Some(error) = query.get("error") {
            respond(
                &mut stream,
                "200 OK",
                &page("Вход отменён. Можно закрыть эту вкладку."),
            )
            .await;
            let detail = query.get("error_description").unwrap_or(error);
            bail!("Sign-in was declined: {detail}");
        }
        respond(
            &mut stream,
            "200 OK",
            &page("Вход выполнен. Можно закрыть эту вкладку и вернуться в Belka."),
        )
        .await;
        return Ok(query["code"].clone());
    }
}

/// Reads the request head and returns the target of `GET <target> HTTP/1.1`.
async fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 16 * 1024 {
        let n = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut buf))
            .await
            .ok()?
            .ok()?;
        if n == 0 {
            break;
        }
        head.extend_from_slice(&buf[..n]);
    }
    let line = String::from_utf8_lossy(&head).lines().next()?.to_string();
    let mut parts = line.split_whitespace();
    (parts.next()? == "GET").then(|| parts.next().map(str::to_string))?
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

fn page(message: &str) -> String {
    format!(
        "<!doctype html><meta charset=utf-8><title>Belka</title>\
         <body style=\"font:16px system-ui,sans-serif;display:grid;place-items:center;height:90vh\">\
         <p>{message}</p></body>"
    )
}

async fn google_email(http: &reqwest::Client, token: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct About {
        user: User,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct User {
        email_address: Option<String>,
    }
    let about: About = http
        .get("https://www.googleapis.com/drive/v3/about")
        .query(&[("fields", "user(emailAddress)")])
        .bearer_auth(token)
        .timeout(API_TIMEOUT)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()?;
    about.user.email_address
}

async fn yandex_login(http: &reqwest::Client, token: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Disk {
        user: Option<User>,
    }
    #[derive(Deserialize)]
    struct User {
        login: Option<String>,
    }
    let disk: Disk = http
        .get("https://cloud-api.yandex.net/v1/disk/")
        .query(&[("fields", "user")])
        .header("Authorization", format!("OAuth {token}"))
        .timeout(API_TIMEOUT)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()?;
    let login = disk.user?.login?;
    debug!("Signed in to Yandex Disk as {login}");
    Some(login)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives the loopback listener the way a browser would.
    async fn redirect(port: u16, target: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream
            .write_all(format!("GET {target} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        response
    }

    #[tokio::test]
    async fn receives_the_code_and_ignores_other_requests() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let browser = tokio::spawn(async move {
            let favicon = redirect(port, "/favicon.ico").await;
            let done = redirect(port, "/?state=s1&code=abc%2F123").await;
            (favicon, done)
        });

        let code = wait_for_code(&listener, "s1").await.unwrap();
        let (favicon, done) = browser.await.unwrap();

        assert_eq!(code, "abc/123");
        assert!(favicon.starts_with("HTTP/1.1 404"));
        assert!(done.starts_with("HTTP/1.1 200") && done.contains("Вход выполнен"));
    }

    #[tokio::test]
    async fn rejects_a_wrong_state_and_reports_denial() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { redirect(port, "/?state=evil&code=x").await });
        let err = wait_for_code(&listener, "s1").await.unwrap_err();
        assert!(err.to_string().contains("state mismatch"));

        tokio::spawn(async move {
            redirect(
                port,
                "/?state=s1&error=access_denied&error_description=nope",
            )
            .await
        });
        let err = wait_for_code(&listener, "s1").await.unwrap_err();
        assert!(err.to_string().contains("nope"), "{err}");
    }

    #[test]
    fn random_tokens_differ() {
        let a = random_token(32).unwrap();
        assert_eq!(a.len(), 43);
        assert_ne!(a, random_token(32).unwrap());
    }
}
