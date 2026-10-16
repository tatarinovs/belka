//! `belka login`: signs in through the browser and prints the .env lines,
//! like `rclone authorize`. Run it on a desktop, then copy the lines to the
//! server's .env.

use crate::config::{self, ProviderKind};
use anyhow::{bail, Result};
use belka_core::oauth::{self, Service};
use belka_core::storage::google::GoogleAuth;
use belka_core::ProviderConfig;
use std::process::Command;

pub async fn run(kind: ProviderKind) -> Result<()> {
    let service = match kind {
        ProviderKind::Google => Service::Google,
        ProviderKind::Yandex => Service::Yandex,
        _ => bail!("login supports google and yandex; other providers use passwords or keys"),
    };
    let Some(app) = config::oauth_app(service)? else {
        let prefix = if service == Service::Google {
            "GOOGLE"
        } else {
            "YANDEX"
        };
        bail!("This build has no built-in OAuth app: set {prefix}_CLIENT_ID and {prefix}_CLIENT_SECRET");
    };
    let builtin = service.builtin_app().as_ref() == Some(&app);

    // Messages go to stderr, so `belka login google >> .env` writes only the variables.
    let signed_in = oauth::sign_in(service, &app, |url| {
        eprintln!(
            "Opening the browser to sign in to {}. If it does not open, visit:\n\n{url}\n",
            service.title()
        );
        open_browser(url);
        Ok(())
    })
    .await?;
    if let Some(label) = &signed_in.label {
        eprintln!("Signed in as {label}. Add these lines to .env:\n");
    } else {
        eprintln!("Signed in. Add these lines to .env:\n");
    }

    match signed_in.config {
        ProviderConfig::Google(GoogleAuth::RefreshToken {
            client_id,
            client_secret,
            refresh_token,
        }) => {
            // The token works only with the app it was issued to; the built-in
            // one needs no variables, but then the CLI must be this build.
            if !builtin {
                println!("GOOGLE_CLIENT_ID={client_id}");
                println!("GOOGLE_CLIENT_SECRET={client_secret}");
            }
            println!("GOOGLE_REFRESH_TOKEN={refresh_token}");
        }
        ProviderConfig::Yandex { token } => println!("YANDEX_TOKEN={token}"),
        _ => unreachable!("sign_in returns Google or Yandex settings"),
    }
    Ok(())
}

/// Best effort: the URL is printed anyway, e.g. for a server without a desktop.
fn open_browser(url: &str) {
    // rundll32 passes the URL as is; `cmd /c start` would split it at '&'.
    let mut command = if cfg!(windows) {
        let mut c = Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    if let Err(err) = command.spawn() {
        tracing::debug!("Cannot open the browser: {err}");
    }
}
