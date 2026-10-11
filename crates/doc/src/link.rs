//! Hyperlink targets from documents are hostile input. Exports (HTML, Markdown, LaTeX, PDF) and
//! the editor decide which targets may become live links with the rules here, so a document can't
//! smuggle `javascript:` (or `vbscript:`, `file:`, custom schemes…) into a page or a viewer.

/// Schemes a document's link may use outside the document: the web and email.
pub const ALLOWED_SCHEMES: [&str; 3] = ["http", "https", "mailto"];

/// The link as a browser reads it: ASCII tab, line feed and carriage return removed anywhere,
/// and leading and trailing control characters and spaces trimmed (WHATWG URL parsing). Without
/// this, `java⇥script:` would look harmless here and still run in a browser.
pub fn normalized(url: &str) -> String {
    let no_breaks: String = url.chars().filter(|c| !matches!(c, '\t' | '\n' | '\r')).collect();
    no_breaks.trim_matches(|c: char| c <= ' ').to_string()
}

/// The scheme of `url` in lower case (`"https"` for `HTTPS://x`), or `None` for a relative
/// reference such as `#top`, `notes.html` or `../a.png`.
pub fn scheme(url: &str) -> Option<String> {
    let url = normalized(url);
    let end = url.find([':', '/', '?', '#'])?;
    if !url[end..].starts_with(':') {
        return None;
    }
    let s = &url[..end];
    let mut chars = s.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    // Not a scheme by URL syntax (`a b:c`): browsers treat it as a relative path; so do we.
    valid.then(|| s.to_ascii_lowercase())
}

/// Whether a document's link target may become a live link in an export: a bookmark (`#name`),
/// a relative reference, or an http, https or mailto URL. Everything else (`javascript:`,
/// `vbscript:`, `data:`, `file:`, `C:\…`, custom schemes) keeps its text but loses the link.
pub fn is_safe_target(url: &str) -> bool {
    let n = normalized(url);
    if n.is_empty() {
        return false;
    }
    match scheme(&n) {
        None => true,
        Some(s) => ALLOWED_SCHEMES.contains(&s.as_str()),
    }
}

/// Whether a link may be handed to the operating system or a viewer as it is: an http, https or
/// mailto URL written plainly, with no spaces or control characters around or inside the scheme
/// (relative references and bookmarks are not followed outside the document). Strict on purpose:
/// what is handed on must be exactly what was checked.
pub fn is_followable(url: &str) -> bool {
    !url.chars().any(|c| c.is_control()) && url == normalized(url) && scheme(url).is_some_and(|s| ALLOWED_SCHEMES.contains(&s.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_and_local_schemes_are_unsafe_however_written() {
        for bad in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            " javascript:alert(1)",
            "java\tscript:alert(1)",
            "java\nscript:alert(1)",
            "\u{1}javascript:alert(1)",
            "vbscript:msgbox(1)",
            "data:text/html,<script>alert(1)</script>",
            "file:///etc/passwd",
            "C:\\Windows\\System32\\calc.exe",
            "ms-msdt:/id",
            "",
            " \t ",
        ] {
            assert!(!is_safe_target(bad), "{bad:?}");
            assert!(!is_followable(bad), "{bad:?}");
        }
    }

    #[test]
    fn web_mail_bookmarks_and_relative_links_are_kept() {
        for good in
            ["https://example.com/a?b=c#d", "HTTP://EXAMPLE.COM", "mailto:a@b.c", "#chapter-2", "notes.html", "../img/a.png", "a b:c", "page?x=a:b"]
        {
            assert!(is_safe_target(good), "{good:?}");
        }
        assert!(is_followable("https://example.com"));
        assert!(is_followable("mailto:a@b.c"));
        assert!(!is_followable("#chapter-2"), "bookmarks stay inside the document");
        assert!(!is_followable("notes.html"));
        // Exports keep these (written normalized); the OS and viewers only get a plain URL.
        for padded in [" https://example.com", "\thttps://example.com", "ht\ttps://example.com"] {
            assert!(is_safe_target(padded), "{padded:?}");
            assert!(!is_followable(padded), "{padded:?}");
            assert!(is_followable(&normalized(padded)), "{padded:?}");
        }
    }

    #[test]
    fn schemes() {
        assert_eq!(scheme("HTTPS://x").as_deref(), Some("https"));
        assert_eq!(scheme("java\tscript:x").as_deref(), Some("javascript"));
        assert_eq!(scheme("x/y:z"), None);
        assert_eq!(scheme("#a:b"), None);
        assert_eq!(scheme("1abc:x"), None);
    }
}
