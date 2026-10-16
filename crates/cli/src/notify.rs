//! Reports the outcome of a run to Telegram, a webhook and a
//! healthchecks.io-style ping URL. A failed notification is logged and
//! never changes the exit code.

use crate::commands::Stats;
use anyhow::{bail, Result};
use belka_core::client::{build_http_client, ensure_success, send_with, RetryPolicy};
use belka_core::utils::format_bytes;
use clap::ValueEnum;
use reqwest::Client;
use serde::Serialize;
use std::time::Duration;
use tracing::{debug, info, warn};

const TIMEOUT: Duration = Duration::from_secs(30);
const TELEGRAM_API: &str = "https://api.telegram.org";
const MAX_ERROR_CHARS: usize = 1500;

/// Which outcomes are reported to Telegram and the webhook. The ping URL
/// always gets every outcome, it is a heartbeat.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum NotifyOn {
    /// Every run
    Always,
    /// Runs with failed files or a fatal error, and interrupted ones
    Failure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Success,
    /// Finished, but some files failed.
    Partial,
    Failure,
    Interrupted,
}

impl Status {
    fn is_success(self) -> bool {
        self == Self::Success
    }

    fn title(self, mode: Mode) -> String {
        let what = mode.noun();
        match self {
            Self::Success => format!("✅ {what} succeeded"),
            Self::Partial => format!("⚠️ {what} finished with errors"),
            Self::Failure => format!("❌ {what} failed"),
            Self::Interrupted => format!("⏹ {what} interrupted"),
        }
    }
}

/// What kind of run is reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Upload,
    Restore,
    Mirror,
    /// A backup task of the app.
    Task,
    Test,
}

impl Mode {
    fn noun(self) -> &'static str {
        match self {
            Self::Upload | Self::Task => "Backup",
            Self::Restore => "Restore",
            Self::Mirror => "Mirror",
            Self::Test => "Test",
        }
    }

    fn verb(self) -> &'static str {
        match self {
            Self::Restore => "downloaded",
            _ => "uploaded",
        }
    }
}

/// What a run ended with.
pub struct Report {
    pub status: Status,
    pub mode: Mode,
    /// What was copied where, e.g. `yandex:/Backups`.
    pub target: String,
    pub stats: Stats,
    pub elapsed: Duration,
    pub error: Option<String>,
}

struct Telegram {
    api: String,
    token: String,
    chat: String,
}

pub struct Notifier {
    on: NotifyOn,
    /// Distinguishes jobs in messages, NOTIFY_NAME.
    name: Option<String>,
    host: String,
    telegram: Option<Telegram>,
    webhook: Option<String>,
    ping: Option<String>,
    client: Client,
}

/// The JSON posted to NOTIFY_WEBHOOK_URL. `text` and `content` carry the
/// same message, so Slack, Mattermost, Rocket.Chat and Discord accept the
/// payload as is.
#[derive(Serialize)]
struct WebhookPayload<'a> {
    status: Status,
    name: Option<&'a str>,
    host: &'a str,
    mode: Mode,
    target: &'a str,
    processed: u64,
    transferred: u64,
    unchanged: u64,
    deleted: u64,
    failed: u64,
    bytes: u64,
    duration_secs: u64,
    error: Option<&'a str>,
    version: &'a str,
    text: &'a str,
    content: &'a str,
}

fn var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

impl Notifier {
    /// Reads NOTIFY_* variables; `None` if no channel is configured.
    pub fn from_env(on: NotifyOn) -> Result<Option<Self>> {
        let telegram = match (var("NOTIFY_TELEGRAM_TOKEN"), var("NOTIFY_TELEGRAM_CHAT")) {
            (Some(token), Some(chat)) => Some(Telegram {
                api: var("NOTIFY_TELEGRAM_API")
                    .unwrap_or_else(|| TELEGRAM_API.into())
                    .trim_end_matches('/')
                    .to_string(),
                token,
                chat,
            }),
            (Some(_), None) => bail!("NOTIFY_TELEGRAM_TOKEN is set without NOTIFY_TELEGRAM_CHAT"),
            (None, Some(_)) => bail!("NOTIFY_TELEGRAM_CHAT is set without NOTIFY_TELEGRAM_TOKEN"),
            (None, None) => None,
        };
        let webhook = var("NOTIFY_WEBHOOK_URL");
        let ping = var("NOTIFY_PING_URL").map(|u| u.trim_end_matches('/').to_string());
        for url in webhook.iter().chain(&ping) {
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                bail!("Notification URL must start with https:// or http://: {url}");
            }
        }
        if telegram.is_none() && webhook.is_none() && ping.is_none() {
            return Ok(None);
        }
        Ok(Some(Self {
            on,
            name: var("NOTIFY_NAME"),
            host: host_name(),
            telegram,
            webhook,
            ping,
            client: build_http_client()?,
        }))
    }

    /// Tells the ping service that a run has started, so it can measure
    /// the duration and notice runs that never finish.
    pub async fn started(&self) {
        if let Some(ping) = &self.ping {
            let url = format!("{ping}/start");
            if let Err(err) = self.post("ping", &url, String::new()).await {
                warn!("Notification (ping) failed: {err:#}");
            }
        }
    }

    pub async fn finished(&self, report: &Report) {
        let text = self.message(report);
        let notify = report.status != Status::Success || self.on == NotifyOn::Always;
        let mut sent = Vec::new();

        if let Some(ping) = &self.ping {
            let url = if report.status.is_success() {
                ping.clone()
            } else {
                format!("{ping}/fail")
            };
            let result = self.post("ping", &url, text.clone()).await;
            self.log("ping", result, &mut sent);
        }
        if !notify {
            debug!("Notifications: success is not reported (NOTIFY_ON=failure)");
        } else {
            if let Some(tg) = &self.telegram {
                let result = self.telegram(tg, &text).await;
                self.log("Telegram", result, &mut sent);
            }
            if let Some(url) = &self.webhook {
                let result = self.webhook(url, report, &text).await;
                self.log("webhook", result, &mut sent);
            }
        }
        if !sent.is_empty() {
            debug!("Notification sent: {}", sent.join(", "));
        }
    }

    /// Sends a test message to every configured channel.
    pub async fn test(&self) -> Result<()> {
        let report = Report {
            status: Status::Success,
            mode: Mode::Test,
            target: "-".into(),
            stats: Stats::default(),
            elapsed: Duration::ZERO,
            error: None,
        };
        let text = format!(
            "🔔 Belka test notification\nHost: {}{}",
            self.host,
            self.name
                .as_deref()
                .map(|n| format!("\nJob: {n}"))
                .unwrap_or_default()
        );
        let mut failed = 0;
        let mut check = |what: &str, result: Result<()>| match result {
            Ok(()) => info!("  [+] {what}: sent"),
            Err(err) => {
                warn!("  [!] {what}: {err:#}");
                failed += 1;
            }
        };
        if let Some(tg) = &self.telegram {
            check("Telegram", self.telegram(tg, &text).await);
        }
        if let Some(url) = &self.webhook {
            check("Webhook", self.webhook(url, &report, &text).await);
        }
        if let Some(ping) = &self.ping {
            check("Ping", self.post("ping", ping, text.clone()).await);
        }
        if failed > 0 {
            bail!("{failed} notification channel(s) failed");
        }
        Ok(())
    }

    fn log(&self, what: &'static str, result: Result<()>, sent: &mut Vec<&'static str>) {
        match result {
            Ok(()) => sent.push(what),
            Err(err) => warn!("Notification ({what}) failed: {err:#}"),
        }
    }

    fn message(&self, r: &Report) -> String {
        let title = r.status.title(r.mode);
        let mut lines = vec![match &self.name {
            Some(name) => format!("{title} — {name}"),
            None => title,
        }];
        lines.push(format!("Host: {}", self.host));
        lines.push(format!("Target: {}", r.target));
        let s = &r.stats;
        if s.transferred + s.unchanged + s.failed > 0 {
            lines.push(format!(
                "Files: {} {} ({}), {} unchanged, {} failed",
                s.transferred,
                r.mode.verb(),
                format_bytes(s.bytes),
                s.unchanged,
                s.failed
            ));
        }
        if s.deleted > 0 {
            lines.push(format!("Deleted: {}", s.deleted));
        }
        lines.push(format!("Time: {}", format_duration(r.elapsed)));
        if let Some(error) = &r.error {
            // Telegram rejects messages over 4096 characters.
            let mut error: String = error.chars().take(MAX_ERROR_CHARS).collect();
            if error.len() < r.error.as_ref().map_or(0, String::len) {
                error.push('…');
            }
            lines.push(format!("Error: {error}"));
        }
        lines.join("\n")
    }

    async fn telegram(&self, tg: &Telegram, text: &str) -> Result<()> {
        let url = format!("{}/bot{}/sendMessage", tg.api, tg.token);
        let body = serde_json::json!({
            "chat_id": tg.chat,
            "text": text,
            "disable_web_page_preview": true,
        });
        let resp = send_with("Telegram", policy(), || {
            Ok(self.client.post(&url).timeout(TIMEOUT).json(&body))
        })
        .await
        // The URL holds the bot token: keep it out of the logs.
        .map_err(|err| anyhow::anyhow!("{}", redact(&format!("{err:#}"), &tg.token)))?;
        ensure_success(resp, "Telegram sendMessage")
            .await
            .map_err(|err| anyhow::anyhow!("{}", redact(&format!("{err:#}"), &tg.token)))?;
        Ok(())
    }

    async fn webhook(&self, url: &str, r: &Report, text: &str) -> Result<()> {
        let payload = WebhookPayload {
            status: r.status,
            name: self.name.as_deref(),
            host: &self.host,
            mode: r.mode,
            target: &r.target,
            processed: r.stats.processed,
            transferred: r.stats.transferred,
            unchanged: r.stats.unchanged,
            deleted: r.stats.deleted,
            failed: r.stats.failed,
            bytes: r.stats.bytes,
            duration_secs: r.elapsed.as_secs(),
            error: r.error.as_deref(),
            version: env!("CARGO_PKG_VERSION"),
            text,
            content: text,
        };
        let resp = send_with("webhook", policy(), || {
            Ok(self.client.post(url).timeout(TIMEOUT).json(&payload))
        })
        .await?;
        ensure_success(resp, "webhook").await?;
        Ok(())
    }

    /// POSTs `body` as plain text; healthchecks.io shows it in the event log.
    async fn post(&self, what: &str, url: &str, body: String) -> Result<()> {
        let resp = send_with(what, policy(), || {
            Ok(self
                .client
                .post(url)
                .timeout(TIMEOUT)
                .header(reqwest::header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .body(body.clone()))
        })
        .await?;
        ensure_success(resp, what).await?;
        Ok(())
    }
}

fn policy() -> RetryPolicy {
    RetryPolicy {
        max_attempts: 3,
        retry_server_errors: true,
    }
}

fn redact(text: &str, secret: &str) -> String {
    text.replace(secret, "***")
}

fn host_name() -> String {
    ["COMPUTERNAME", "HOSTNAME"]
        .iter()
        .find_map(|v| var(v))
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "unknown".into())
}

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m {}s", secs / 60, secs % 60),
        _ => format!("{}h {}m", secs / 3600, secs % 3600 / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notifier(name: Option<&str>) -> Notifier {
        Notifier {
            on: NotifyOn::Failure,
            name: name.map(Into::into),
            host: "srv".into(),
            telegram: None,
            webhook: None,
            ping: None,
            client: build_http_client().unwrap(),
        }
    }

    #[test]
    fn message_lists_what_happened() {
        let report = Report {
            status: Status::Partial,
            mode: Mode::Upload,
            target: "yandex:/Backups".into(),
            stats: Stats {
                processed: 5,
                transferred: 3,
                unchanged: 1,
                deleted: 2,
                failed: 1,
                bytes: 2048,
            },
            elapsed: Duration::from_secs(125),
            error: None,
        };
        let text = notifier(Some("nightly")).message(&report);
        assert_eq!(
            text,
            "⚠️ Backup finished with errors — nightly\nHost: srv\nTarget: yandex:/Backups\n\
             Files: 3 uploaded (2.0 KB), 1 unchanged, 1 failed\nDeleted: 2\nTime: 2m 5s"
        );
    }

    #[test]
    fn redacts_tokens_and_formats_durations() {
        assert_eq!(
            redact("GET /bot123:abc/send", "123:abc"),
            "GET /bot***/send"
        );
        assert_eq!(format_duration(Duration::from_secs(7)), "7s");
        assert_eq!(format_duration(Duration::from_secs(3725)), "1h 2m");
    }
}
