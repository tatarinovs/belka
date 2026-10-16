//! Errors of the commands, as the page shows them: in Russian where the
//! message is known, otherwise as the libraries wrote it. The core and the
//! CLI speak English; only the GUI translates.

use serde::{Serialize, Serializer};

/// A command error: the whole context chain, translated.
#[derive(Debug)]
pub struct CmdError(String);

pub type CmdResult<T> = Result<T, CmdError>;

impl From<anyhow::Error> for CmdError {
    /// Also logged: the page shows the message, but the log keeps it.
    fn from(err: anyhow::Error) -> Self {
        let msg = format!("{err:#}");
        tracing::warn!("{msg}");
        Self(translate(&msg))
    }
}

impl Serialize for CmdError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

/// Russian text for the messages users meet. A known message of the app is
/// replaced; for network and HTTP failures an explanation goes first and the
/// original stays as the detail.
pub fn translate(msg: &str) -> String {
    let lower = msg.to_lowercase();
    for (needle, text) in EXACT {
        if let Some(rest) = msg.strip_prefix(needle) {
            return format!("{text}{}", quotes(rest));
        }
    }
    if let Some(text) = known(msg) {
        return text;
    }
    let reason = if has_any(&lower, &["dns error", "failed to lookup address", "no such host"]) {
        Some("Сервер не найден: проверьте адрес и подключение к интернету")
    } else if has_any(
        &lower,
        &["connection refused", "tcp connect error", "error sending request", "network is unreachable"],
    ) {
        Some("Нет связи с сервером: проверьте подключение к интернету")
    } else if has_any(&lower, &["timed out", "no response within", "stalled"]) {
        Some("Сервер не отвечает")
    } else if has_any(&lower, &["certificate", "tls handshake", "invalid peer"]) {
        Some("Ошибка защищённого соединения: сертификат сервера не принят")
    } else if lower.contains("http 401") {
        Some("Неверный логин, пароль или токен: войдите заново")
    } else if lower.contains("http 403") {
        Some("Доступ запрещён")
    } else if lower.contains("http 404") {
        Some("Не найдено")
    } else if has_any(&lower, &["http 507", "insufficient storage", "quota", "storagequotaexceeded"]) {
        Some("В облаке не хватает места")
    } else if lower.contains("http 429") {
        Some("Слишком много запросов: облако просит подождать")
    } else if has_any(&lower, &["http 500", "http 502", "http 503", "http 504"]) {
        Some("Ошибка на стороне облака, попробуйте позже")
    } else if has_any(&lower, &["access is denied", "permission denied", "os error 13", "os error 5)"]) {
        Some("Нет доступа к файлу или папке")
    } else if has_any(&lower, &["no space left", "os error 28", "os error 112"]) {
        Some("На диске не хватает места")
    } else {
        None
    };
    match reason {
        Some(reason) => format!("{reason}. Подробности: {msg}"),
        None => msg.to_string(),
    }
}

/// Messages of the app that start with a fixed text; the rest (usually a
/// quoted name) is kept.
const EXACT: &[(&str, &str)] = &[
    ("Account name is empty", "Не указано название аккаунта"),
    ("Account not found", "Аккаунт не найден"),
    ("Credentials are missing; sign in again", "Нет данных для входа: подключите аккаунт заново"),
    ("The account type cannot be changed", "Тип аккаунта изменить нельзя"),
    ("Sign-in cancelled", "Вход отменён"),
    ("Choose the folder to back up", "Выберите папку, которую нужно копировать"),
    ("Choose the cloud to back up to", "Выберите облако, куда копировать"),
    ("Backup task not found", "Задача автобэкапа не найдена"),
    ("The cloud account of the task was removed", "Облачный аккаунт задачи удалён"),
    ("Nothing to upload", "Нечего загружать"),
    ("The file is damaged", "Файл повреждён"),
    ("Wrong password, or the encrypted file is damaged", "Неверный пароль, или зашифрованный файл повреждён"),
    ("An encrypted folder cannot be placed inside another one", "Зашифрованную папку нельзя положить в другую зашифрованную"),
    ("The root folder cannot be renamed", "Корневую папку нельзя переименовать"),
    ("Refusing to delete the root folder", "Корневую папку удалить нельзя"),
    ("Cannot access ", "Нет доступа к "),
    ("Cannot open folder ", "Не удаётся открыть папку "),
    ("Invalid name ", "Недопустимое имя "),
    ("This build has no built-in OAuth app; enter your own", "В этой сборке нет встроенного приложения OAuth: укажите своё"),
];

/// Messages with a name in the middle.
fn known(msg: &str) -> Option<String> {
    if let Some(name) = between(msg, "", " already exists") {
        return Some(format!("{} уже существует", quotes(name)));
    }
    if let Some(name) = between(msg, "The backup ", " is running already") {
        return Some(format!("Бэкап {name} уже выполняется"));
    }
    if let Some(names) = between(msg, "Backups ", " go to this account; remove them first") {
        return Some(format!(
            "В этот аккаунт копируют задачи автобэкапа {names}: сначала удалите их"
        ));
    }
    if let Some(names) = between(msg, "Encrypted folders ", " are stored in this account; remove them first") {
        return Some(format!(
            "В этом аккаунте хранятся зашифрованные папки {names}: сначала удалите их"
        ));
    }
    if let Some(path) = between(msg, "", " is not a folder") {
        return Some(format!("{} — не папка", quotes(path)));
    }
    None
}

fn between<'a>(msg: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    let rest = msg.strip_prefix(prefix)?;
    let end = rest.find(suffix)?;
    (end > 0 && !rest[..end].contains(": ")).then(|| &rest[..end])
}

/// `'name'` becomes `«name»`, as Russian text quotes.
fn quotes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut open = true;
    for c in text.chars() {
        if c == '\'' {
            out.push(if open { '«' } else { '»' });
            open = !open;
        } else {
            out.push(c);
        }
    }
    out
}

fn has_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| text.contains(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_messages_are_translated() {
        assert_eq!(translate("Account not found"), "Аккаунт не найден");
        assert_eq!(translate("Invalid name 'a/b'"), "Недопустимое имя «a/b»");
        assert_eq!(translate("'Отчёт' already exists"), "«Отчёт» уже существует");
        assert_eq!(
            translate("The backup «Камера» is running already"),
            "Бэкап «Камера» уже выполняется"
        );
        assert_eq!(
            translate("Backups «Камера» go to this account; remove them first"),
            "В этот аккаунт копируют задачи автобэкапа «Камера»: сначала удалите их"
        );
    }

    #[test]
    fn transport_failures_get_an_explanation() {
        let msg = "Cannot connect to WebDAV: webdav: PROPFIND: request failed: error sending request: tcp connect error";
        assert!(translate(msg).starts_with("Нет связи с сервером"));
        assert!(translate(msg).ends_with(msg));
        assert!(translate("Upload failed: HTTP 401 Unauthorized").starts_with("Неверный логин"));
        assert_eq!(translate("something new"), "something new");
    }
}
