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
        let name = relative
            .rsplit('/')
            .next()
            .unwrap_or(relative)
            .to_lowercase();
        let text: Vec<char> = name.chars().collect();
        wildcard(&self.text, &text)
    }
}

fn wildcard(pattern: &[char], text: &[char]) -> bool {
    // Every (pattern, text) position is decided once: without this, several
    // stars make the backtracking exponential.
    let mut memo = vec![None; (pattern.len() + 1) * (text.len() + 1)];
    matches_from(pattern, text, 0, 0, &mut memo)
}

/// Whether `pattern[p..]` matches `text[t..]`.
fn matches_from(
    pattern: &[char],
    text: &[char],
    p: usize,
    t: usize,
    memo: &mut [Option<bool>],
) -> bool {
    let key = p * (text.len() + 1) + t;
    if let Some(known) = memo[key] {
        return known;
    }
    let result = match pattern.get(p) {
        None => t == text.len(),
        Some('*') if pattern.get(p + 1) == Some(&'*') => {
            // `**/` also matches nothing, so `**/a` matches `a`.
            let rest = p + 2;
            let rest_no_slash = if pattern.get(rest) == Some(&'/') {
                rest + 1
            } else {
                rest
            };
            (t..=text.len()).any(|i| matches_from(pattern, text, rest, i, memo))
                || matches_from(pattern, text, rest_no_slash, t, memo)
        }
        Some('*') => {
            let mut found = false;
            for i in t..=text.len() {
                if matches_from(pattern, text, p + 1, i, memo) {
                    found = true;
                    break;
                }
                if text.get(i) == Some(&'/') {
                    break;
                }
            }
            found
        }
        Some('?') => {
            text.get(t).is_some_and(|c| *c != '/')
                && matches_from(pattern, text, p + 1, t + 1, memo)
        }
        Some(c) => text.get(t) == Some(c) && matches_from(pattern, text, p + 1, t + 1, memo),
    };
    memo[key] = Some(result);
    result
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
    fn many_stars_stay_fast() {
        let p = Pattern::new(&format!("{}b", "*a".repeat(30)));
        let started = std::time::Instant::now();
        assert!(!p.matches(&"a".repeat(200)));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn include_patterns_look_at_the_file_name() {
        let p = Pattern::new("*.jpg");
        assert!(!p.matches_name("a.jpg/b.png"));
        assert!(p.matches_name("dir/b.jpg"));
    }
}
