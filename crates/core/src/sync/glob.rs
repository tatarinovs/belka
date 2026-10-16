//! File name patterns for `--include` / `--exclude` and backup filters.
//!
//! `*` matches anything except `/`, `**` matches anything including `/`,
//! `?` matches one character. Matching ignores case: on phones and Windows
//! `IMG_1.JPG` and `img_1.jpg` are the same kind of file.

#[derive(Clone, Debug)]
pub struct Pattern {
    /// Lowercased, without a leading `/`.
    text: Vec<char>,
    /// A pattern with `/` is matched against the whole relative path,
    /// one without it against every name in the path.
    anchored: bool,
}

impl Pattern {
    pub fn new(pattern: &str) -> Self {
        let trimmed = pattern.trim().trim_end_matches('/');
        let anchored = trimmed.contains('/');
        Self {
            text: trimmed
                .trim_start_matches('/')
                .to_lowercase()
                .chars()
                .collect(),
            anchored,
        }
    }

    /// Whether the pattern matches `relative`, a `/`-separated path.
    pub fn matches(&self, relative: &str) -> bool {
        let relative = relative.to_lowercase();
        if self.anchored {
            let text: Vec<char> = relative.chars().collect();
            return wildcard(&self.text, &text);
        }
        relative.split('/').any(|name| {
            let text: Vec<char> = name.chars().collect();
            wildcard(&self.text, &text)
        })
    }

    /// Whether the pattern matches the last name of `relative` only; used
    /// for include patterns, which select files, not the folders above them.
    pub fn matches_name(&self, relative: &str) -> bool {
        if self.anchored {
            return self.matches(relative);
        }
        let name = relative.rsplit('/').next().unwrap_or(relative).to_lowercase();
        let text: Vec<char> = name.chars().collect();
        wildcard(&self.text, &text)
    }
}

fn wildcard(pattern: &[char], text: &[char]) -> bool {
    match pattern.first() {
        None => text.is_empty(),
        Some('*') if pattern.get(1) == Some(&'*') => {
            // `**/` also matches nothing, so `**/a` matches `a`.
            let rest = &pattern[2..];
            let rest_no_slash = rest.strip_prefix(&['/']).unwrap_or(rest);
            (0..=text.len()).any(|i| wildcard(rest, &text[i..]))
                || wildcard(rest_no_slash, text)
        }
        Some('*') => {
            let rest = &pattern[1..];
            for i in 0..=text.len() {
                if wildcard(rest, &text[i..]) {
                    return true;
                }
                if text.get(i) == Some(&'/') {
                    break;
                }
            }
            false
        }
        Some('?') => text.first().is_some_and(|c| *c != '/') && wildcard(&pattern[1..], &text[1..]),
        Some(c) => text.first() == Some(c) && wildcard(&pattern[1..], &text[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_anywhere_in_the_path() {
        let p = Pattern::new("*.TMP");
        assert!(p.matches("a/b/x.tmp"));
        assert!(!p.matches("a/b/x.tmpl"));
        let p = Pattern::new("node_modules");
        assert!(p.matches("web/node_modules"));
        assert!(p.matches("web/node_modules/x.js"));
        assert!(!p.matches("web/node_modules2"));
    }

    #[test]
    fn paths_are_anchored_and_double_star_crosses_folders() {
        let p = Pattern::new("/Camera/*.jpg");
        assert!(p.matches("camera/a.jpg"));
        assert!(!p.matches("camera/sub/a.jpg"));
        assert!(!p.matches("x/camera/a.jpg"));
        let p = Pattern::new("Camera/**/*.jpg");
        assert!(p.matches("camera/a.jpg"));
        assert!(p.matches("camera/2026/10/a.jpg"));
        let p = Pattern::new("**/cache");
        assert!(p.matches("cache"));
        assert!(p.matches("a/b/cache"));
    }

    #[test]
    fn question_mark_is_one_character() {
        let p = Pattern::new("img_?.jpg");
        assert!(p.matches("IMG_1.JPG"));
        assert!(!p.matches("img_12.jpg"));
    }

    #[test]
    fn include_patterns_look_at_the_file_name() {
        let p = Pattern::new("*.jpg");
        assert!(!p.matches_name("a.jpg/b.png"));
        assert!(p.matches_name("dir/b.jpg"));
    }
}
