//! `belka accounts`: the accounts that `SOURCE:` can name.

use crate::remote::Accounts;
use anyhow::Result;

pub fn run(accounts: &mut Accounts) -> Result<()> {
    let from_file = accounts.is_file();
    let list = accounts.source()?.accounts()?;
    if list.is_empty() {
        eprintln!("There are no accounts");
    }
    for account in &list {
        let place = account
            .crypt
            .as_ref()
            .and_then(|c| {
                let base = list.iter().find(|a| a.id == c.base)?;
                Some(format!(" in {}:{}", base.name, c.root))
            })
            .unwrap_or_default();
        println!("{:<24} {}{place}", account.name, account.kind);
    }
    if !from_file {
        eprintln!("\nAlso: yandex, mailru, google, webdav, s3, synology set up by environment variables (see `belka providers`)");
    }
    Ok(())
}
