use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Everything except RFC 3986 "unreserved" characters gets percent-encoded.
/// This is what AWS SigV4 requires and what every WebDAV server accepts.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Percent-encodes a single path segment or query component.
pub fn encode_component(s: &str) -> String {
    utf8_percent_encode(s, COMPONENT).to_string()
}

/// Percent-encodes every segment of a `/`-separated path, keeping the separators.
pub fn encode_path(path: &str) -> String {
    path.split('/')
        .map(encode_component)
        .collect::<Vec<_>>()
        .join("/")
}

/// Resolves a POSIX-style remote path, normalizing `.` and `..` segments,
/// treating `\` as a separator, stripping redundant slashes, and always
/// anchoring to `/` so a stray `..` cannot escape root.
pub fn normalize_remote_dir(input: &str) -> String {
    let mut stack: Vec<&str> = Vec::new();
    for part in input.split(['/', '\\']) {
        match part {
            "." => continue,
            // Only an exact `..` goes up: ` ..` is a name like any other.
            ".." => {
                stack.pop();
            }
            _ if part.trim().is_empty() => continue,
            _ => stack.push(part),
        }
    }
    format!("/{}", stack.join("/"))
}

/// Joins a remote directory and a relative path into a normalized absolute path.
pub fn join_remote(dir: &str, name: &str) -> String {
    normalize_remote_dir(&format!("{dir}/{name}"))
}

/// Splits a remote path into its normalized parent directory and final name.
pub fn split_remote(path: &str) -> (String, String) {
    let path = normalize_remote_dir(path);
    match path.rsplit_once('/') {
        Some(("", name)) => ("/".to_string(), name.to_string()),
        Some((parent, name)) => (parent.to_string(), name.to_string()),
        None => ("/".to_string(), String::new()),
    }
}

/// Makes an arbitrary string safe to use as a single file name on any
/// platform and in any cloud: no separators, reserved or control characters.
pub fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches(['.', ' ']);
    if trimmed.is_empty() {
        "_".to_string()
    } else if is_reserved_on_windows(trimmed) {
        format!("_{trimmed}")
    } else {
        trimmed.to_string()
    }
}

/// `CON`, `NUL`, `COM1` and the like are devices on Windows, with any extension.
fn is_reserved_on_windows(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default().trim_end();
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && matches!(upper.as_bytes()[3], b'1'..=b'9'))
}

/// `path`, or `name (2).ext` and so on if an earlier call took it. Clouds
/// such as Google Drive allow equal names in one folder, and sanitizing can
/// make different names equal. Case is ignored, as Windows and macOS do.
pub fn unique_path(path: PathBuf, taken: &mut HashSet<String>) -> PathBuf {
    let key = |p: &Path| p.to_string_lossy().to_lowercase();
    if taken.insert(key(&path)) {
        return path;
    }
    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (2..)
        .map(|n| path.with_file_name(format!("{stem} ({n}){ext}")))
        .find(|candidate| taken.insert(key(candidate)))
        .expect("an unused name exists")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_keeps_dotdot_inside_root() {
        assert_eq!(normalize_remote_dir("/Upload"), "/Upload");
        assert_eq!(normalize_remote_dir("Upload/../../etc"), "/etc");
        assert_eq!(normalize_remote_dir("../../etc/passwd"), "/etc/passwd");
        assert_eq!(normalize_remote_dir("/a/./b//c/"), "/a/b/c");
        assert_eq!(normalize_remote_dir(""), "/");
        assert_eq!(normalize_remote_dir("/"), "/");
        assert_eq!(normalize_remote_dir("/Backups/../Upload/"), "/Upload");
        assert_eq!(normalize_remote_dir("\\Backups\\Daily"), "/Backups/Daily");
        assert_eq!(normalize_remote_dir("/a/ .. /b"), "/a/ .. /b");
        assert_eq!(normalize_remote_dir("/a/ /b"), "/a/b");
    }

    #[test]
    fn join_and_split() {
        assert_eq!(join_remote("/", "a.txt"), "/a.txt");
        assert_eq!(join_remote("/Upload/", "/sub/a.txt"), "/Upload/sub/a.txt");
        assert_eq!(
            split_remote("/Upload/a b.txt"),
            ("/Upload".into(), "a b.txt".into())
        );
        assert_eq!(split_remote("/a.txt"), ("/".into(), "a.txt".into()));
    }

    #[test]
    fn encoding_is_strict() {
        assert_eq!(
            encode_component("[v1.0] my app~_-.apk"),
            "%5Bv1.0%5D%20my%20app~_-.apk"
        );
        assert_eq!(encode_component("a/b+c"), "a%2Fb%2Bc");
        assert_eq!(
            encode_path("Папка/x y"),
            "%D0%9F%D0%B0%D0%BF%D0%BA%D0%B0/x%20y"
        );
    }

    #[test]
    fn sanitize() {
        assert_eq!(sanitize_file_name("release/1.0"), "release_1.0");
        assert_eq!(sanitize_file_name("a:b*c?.zip"), "a_b_c_.zip");
        assert_eq!(sanitize_file_name(".."), "_");
        assert_eq!(sanitize_file_name("  name. "), "name");
        assert_eq!(sanitize_file_name("Файл.txt"), "Файл.txt");
        assert_eq!(sanitize_file_name("con"), "_con");
        assert_eq!(sanitize_file_name("NUL.txt"), "_NUL.txt");
        assert_eq!(sanitize_file_name("COM1.log"), "_COM1.log");
        assert_eq!(sanitize_file_name("console.txt"), "console.txt");
        assert_eq!(sanitize_file_name("COM0"), "COM0");
    }

    #[test]
    fn unique_path_numbers_duplicates_case_insensitively() {
        let mut taken = HashSet::new();
        let dir = Path::new("dl");
        assert_eq!(
            unique_path(dir.join("a.txt"), &mut taken),
            dir.join("a.txt")
        );
        assert_eq!(
            unique_path(dir.join("A.txt"), &mut taken),
            dir.join("A (2).txt")
        );
        assert_eq!(
            unique_path(dir.join("a.txt"), &mut taken),
            dir.join("a (3).txt")
        );
        assert_eq!(unique_path(dir.join("b"), &mut taken), dir.join("b"));
    }
}
