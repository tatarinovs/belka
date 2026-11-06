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

/// Separates the explanation from the original message; the page splits
/// the text here to show the original under "Подробнее".
pub const DETAIL_MARK: &str = ". Подробности: ";

/// Russian text for the messages users meet. A known message of the app is
/// replaced; for network and HTTP failures an explanation goes first and the
/// original stays as the detail, after [`DETAIL_MARK`]. Names in quotes are
/// not looked at, so a file called `network.txt` does not make a failure
/// look like a network one.
pub fn translate(msg: &str) -> String {
    let lower = without_quoted(msg).to_lowercase();
    for (needle, text) in EXACT {
        if let Some(rest) = msg.strip_prefix(needle) {
            return format!("{text}{}", quotes(rest));
        }
    }
    if let Some(text) = known(msg) {
        return text;
    }
    let reason = if lower.starts_with("google drive has several items named") {
        Some("В Google Диске есть несколько объектов с одинаковым именем. Переименуйте дубликаты в Google Диске")
    } else if lower.starts_with("cannot upload an incomplete folder scan") {
        Some(
            "Не все файлы удалось прочитать. Загрузка не началась, чтобы не пропустить часть папки",
        )
    } else if has_any(
        &lower,
        &["dns error", "failed to lookup address", "no such host"],
    ) {
        Some("Сервер не найден: проверьте адрес и подключение к интернету")
    } else if has_any(
        &lower,
        &[
            "connection refused",
            "tcp connect error",
            "error sending request",
            "network is unreachable",
        ],
    ) {
        Some("Нет связи с сервером: проверьте подключение к интернету")
    } else if has_any(&lower, &["timed out", "no response within", "stalled"]) {
        Some("Сервер не отвечает")
    } else if has_any(&lower, &["certificate", "tls handshake", "invalid peer"]) {
        Some("Ошибка защищённого соединения: сертификат сервера не принят")
    } else if lower.contains("http 401") {
        Some("Облако не принимает логин, пароль или токен: откройте настройки аккаунта и войдите заново")
    } else if lower.contains("http 403") {
        Some("Доступ запрещён")
    } else if lower.contains("http 404") {
        Some("Не найдено: возможно, файл уже переместили или удалили. Обновите список")
    } else if has_any(
        &lower,
        &[
            "http 507",
            "insufficient storage",
            "quota",
            "storagequotaexceeded",
        ],
    ) {
        Some("В облаке не хватает места")
    } else if lower.contains("http 429") {
        Some("Слишком много запросов: облако просит подождать")
    } else if has_any(&lower, &["http 500", "http 502", "http 503", "http 504"]) {
        Some("Ошибка на стороне облака, попробуйте позже")
    } else if has_any(
        &lower,
        &[
            "access is denied",
            "permission denied",
            "os error 13",
            "os error 5)",
        ],
    ) {
        Some("Нет доступа к файлу или папке")
    } else if has_any(&lower, &["no space left", "os error 28", "os error 112"]) {
        Some("На диске не хватает места")
    } else {
        None
    };
    match reason {
        Some(reason) => format!("{reason}{DETAIL_MARK}{msg}"),
        None => msg.to_string(),
    }
}

/// `msg` without the names in quotes: `'…'` as the core quotes paths, and
/// `«…»`. An apostrophe inside a word ("doesn't", "O'Brien") is not a quote.
fn without_quoted(msg: &str) -> String {
    let chars: Vec<char> = msg.chars().collect();
    let mut out = String::with_capacity(msg.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let opens = c == '«' || (c == '\'' && (i == 0 || !chars[i - 1].is_alphanumeric()));
        if opens {
            let close = if c == '«' { '»' } else { '\'' };
            let end = (i + 1..chars.len()).find(|&j| {
                chars[j] == close
                    && (close == '»' || chars.get(j + 1).is_none_or(|next| !next.is_alphanumeric()))
            });
            if let Some(end) = end {
                out.push_str("''");
                i = end + 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Messages of the app that start with a fixed text; the rest (usually a
/// quoted name) is kept.
const EXACT: &[(&str, &str)] = &[
    ("Account name is empty", "Не указано название аккаунта"),
    ("Account not found", "Аккаунт не найден"),
    (
        "Credentials are missing; sign in again",
        "Нет данных для входа: подключите аккаунт заново",
    ),
    (
        "The account type cannot be changed",
        "Тип аккаунта изменить нельзя",
    ),
    ("Sign-in cancelled", "Вход отменён"),
    (
        "Choose the folder to back up",
        "Выберите папку, которую нужно копировать",
    ),
    (
        "Choose the cloud to back up to",
        "Выберите облако, куда копировать",
    ),
    ("Backup task not found", "Задача автобэкапа не найдена"),
    (
        "The cloud account of the task was removed",
        "Облачный аккаунт задачи удалён",
    ),
    ("Nothing to upload", "Нечего загружать"),
    (
        "A folder cannot be copied into itself",
        "Папку нельзя скопировать в саму себя",
    ),
    (
        "Files of this type run as programs when opened",
        "Файлы этого типа при открытии запускаются как программы; чтобы изучить такой файл, скачайте его",
    ),
    (
        "The password must be at least 8 characters long",
        "Пароль должен быть не короче 8 символов",
    ),
    (
        "The file was made by a newer version of the program",
        "Файл создан более новой версией программы: обновите Belka",
    ),
    ("The file is damaged", "Файл повреждён"),
    (
        "Wrong password, or the encrypted file is damaged",
        "Неверный пароль, или зашифрованный файл повреждён",
    ),
    (
        "An encrypted folder cannot be placed inside another one",
        "Зашифрованную папку нельзя положить в другую зашифрованную",
    ),
    (
        "The root folder cannot be renamed",
        "Корневую папку нельзя переименовать",
    ),
    (
        "Refusing to delete the root folder",
        "Корневую папку удалить нельзя",
    ),
    (
        "The cloud cannot stream this file",
        "Это облако не отдаёт файлы потоком: скачайте файл",
    ),
    (
        "No player is set for files of this type",
        "В системе не назначена программа для файлов этого типа",
    ),
    ("No app can play the video", "Нет приложения, которое может воспроизвести это видео"),
    ("Cannot access ", "Нет доступа к "),
    ("Cannot open folder ", "Не удаётся открыть папку "),
    ("Invalid name ", "Недопустимое имя "),
    (
        "This build has no built-in OAuth app; enter your own",
        "В этой сборке нет встроенного приложения OAuth: укажите своё",
    ),
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
    if let Some(names) = between(
        msg,
        "Encrypted folders ",
        " are stored in this account; remove them first",
    ) {
        return Some(format!(
            "В этом аккаунте хранятся зашифрованные папки {names}: сначала удалите их"
        ));
    }
    if let Some(path) = between(msg, "", " contains files that cannot be decrypted") {
        return Some(format!(
            "В {} есть файлы, которые не расшифровываются этим паролем: проверьте пароль и соль \
             или выберите пустую папку",
            quotes(path)
        ));
    }
    if let Some(name) = between(msg, "", " is in this folder already") {
        return Some(format!("{} уже лежит в этой папке", quotes(name)));
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
        assert_eq!(
            translate("'Отчёт' already exists"),
            "«Отчёт» уже существует"
        );
        assert_eq!(
            translate("The backup «Камера» is running already"),
            "Бэкап «Камера» уже выполняется"
        );
        assert_eq!(
            translate("Backups «Камера» go to this account; remove them first"),
            "В этот аккаунт копируют задачи автобэкапа «Камера»: сначала удалите их"
        );
        assert!(translate(
            "'/Encrypted' contains files that cannot be decrypted with this password. \
             Check the password and the salt, or choose an empty folder"
        )
        .starts_with("В «/Encrypted» есть файлы"));
    }

    #[test]
    fn transport_failures_get_an_explanation() {
        let msg = "Cannot connect to WebDAV: webdav: PROPFIND: request failed: error sending request: tcp connect error";
        assert!(translate(msg).starts_with("Нет связи с сервером"));
        assert!(translate(msg).ends_with(msg));
        assert!(
            translate("Upload failed: HTTP 401 Unauthorized").starts_with("Облако не принимает")
        );
        assert_eq!(translate("something new"), "something new");
    }

    #[test]
    fn quoted_names_are_not_reasons() {
        let msg = "'D:\\Network\\token timed out.txt' changed while it was being uploaded";
        assert_eq!(translate(msg), msg);
        assert_eq!(
            translate("Failed to upload 'O'Brien quota.txt': HTTP 507 Insufficient Storage"),
            "В облаке не хватает места. Подробности: \
             Failed to upload 'O'Brien quota.txt': HTTP 507 Insufficient Storage"
        );
        assert_eq!(
            without_quoted("doesn't exist: «a 'b'» and 'c'"),
            "doesn't exist: '' and ''"
        );
    }
}
