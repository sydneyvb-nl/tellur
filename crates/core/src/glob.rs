//! Shared glob matching for policy paths and redaction paths.
//!
//! Supports the common subset used in `.tellur` config and policies:
//! - `*`  — matches any run of characters within a path segment (not `/`)
//! - `**` — matches any run of characters including `/` (spanning segments)
//! - `?`  — matches a single character that is not `/`
//!
//! Matching is anchored to the whole path. A leading `**/` therefore lets a
//! pattern match at any directory depth (e.g. `**/.env*` matches `.env`,
//! `config/.env.production`).

/// Returns true if `path` matches the glob `pattern`.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    matches(pattern.as_bytes(), path.as_bytes())
}

/// Recursive glob matcher handling `*` (within a segment), `**` (across
/// segments, including a leading `**/` that may match zero directories), and
/// `?` (a single non-`/` character).
fn matches(pat: &[u8], text: &[u8]) -> bool {
    if pat.is_empty() {
        return text.is_empty();
    }

    match pat[0] {
        b'*' => {
            if pat.get(1) == Some(&b'*') {
                // `**` — matches any run of characters, including `/`. Skip an
                // optional `/` immediately following so `**/x` also matches `x`.
                let mut rest = &pat[2..];
                if rest.first() == Some(&b'/') {
                    rest = &rest[1..];
                }
                // Try consuming 0..=text.len() characters with `**`.
                for i in 0..=text.len() {
                    if matches(rest, &text[i..]) {
                        return true;
                    }
                }
                false
            } else {
                // Single `*` — matches a run of non-`/` characters.
                let rest = &pat[1..];
                let mut i = 0;
                loop {
                    if matches(rest, &text[i..]) {
                        return true;
                    }
                    if i == text.len() || text[i] == b'/' {
                        return false;
                    }
                    i += 1;
                }
            }
        }
        b'?' => !text.is_empty() && text[0] != b'/' && matches(&pat[1..], &text[1..]),
        c => !text.is_empty() && text[0] == c && matches(&pat[1..], &text[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::glob_match;

    #[test]
    fn test_exact() {
        assert!(glob_match("src/main.rs", "src/main.rs"));
        assert!(!glob_match("src/main.rs", "src/lib.rs"));
    }

    #[test]
    fn test_single_star_segment() {
        assert!(glob_match("src/*.rs", "src/main.rs"));
        assert!(!glob_match("src/*.rs", "src/sub/main.rs"));
    }

    #[test]
    fn test_double_star() {
        assert!(glob_match("src/auth/**", "src/auth/session.ts"));
        assert!(glob_match("src/auth/**", "src/auth/deep/nested.ts"));
        assert!(!glob_match("src/auth/**", "src/utils/x.ts"));
    }

    #[test]
    fn test_env_pattern() {
        // The key secret-path pattern that was previously broken.
        assert!(glob_match("**/.env*", ".env"));
        assert!(glob_match("**/.env*", ".env.production"));
        assert!(glob_match("**/.env*", "config/.env"));
        assert!(glob_match("**/.env*", "a/b/c/.env.local"));
        assert!(!glob_match("**/.env*", "src/environment.rs"));
    }

    #[test]
    fn test_pem_pattern() {
        assert!(glob_match("**/*.pem", "certs/server.pem"));
        assert!(glob_match("**/*.pem", "server.pem"));
        assert!(!glob_match("**/*.pem", "server.key"));
    }

    #[test]
    fn test_question_mark() {
        assert!(glob_match("file?.txt", "file1.txt"));
        assert!(!glob_match("file?.txt", "file.txt"));
    }
}
