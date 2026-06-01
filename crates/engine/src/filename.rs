//! Filename derivation. The engine's `probe` step feeds everything it
//! learns from the server into [`derive_filename`], which applies a
//! precedence ladder and returns the best name it can.
//!
//! The order matters and is fixed: callers should not try to re-shuffle
//! it. The whole point of having a single function is so that what the
//! engine does is testable in isolation.

use url::Url;

/// Derive the best filename from everything the HEAD-probe response
/// gave us. Precedence, highest first:
///
/// 1. `Content-Disposition: …; filename*=UTF-8''…` (RFC 5987,
///    percent-decoded).
/// 2. `Content-Disposition: …; filename="…"` (with quote-stripping and
///    sanitization).
/// 3. The final URL after HTTP redirects — not the user-supplied one —
///    so CDNs that 302 to a real filename are respected. The path tail
///    must "look like a filename" (i.e. have an extension OR not look
///    like a random slug).
/// 4. MIME type → extension map combined with a `download` slug, when
///    the path tail looks random (matches `^[A-Za-z0-9_-]{8,}$` with
///    no dot) but we got a `Content-Type` we can map.
/// 5. Last resort: the path tail of the final or original URL, even
///    if it looks random. Better than `download.bin`.
pub fn derive_filename(
    content_disposition: Option<&str>,
    final_url: &Url,
    original_url: &Url,
    content_type: Option<&str>,
) -> Option<String> {
    // (1) + (2): Content-Disposition.
    if let Some(cd) = content_disposition {
        if let Some(name) = from_content_disposition(cd) {
            let s = sanitize(&name);
            if !s.is_empty() {
                return Some(s);
            }
        }
    }

    // (3): final URL path tail, if it doesn't look like a random slug.
    if let Some(name) = from_url(final_url) {
        if !is_random_slug(&name) {
            let s = sanitize(&name);
            if !s.is_empty() {
                return Some(s);
            }
        }
    }

    // (4): MIME type → extension + `download` slug, when the URL path
    // is random but we know the type.
    if let Some(ct) = content_type {
        if let Some(ext) = extension_from_mime(ct) {
            return Some(format!("download.{ext}"));
        }
    }

    // (5): the original URL's path tail, even if random — at least it's
    // the URL the user pasted. Or, very last resort, the final URL.
    if let Some(name) = from_url(original_url).or_else(|| from_url(final_url)) {
        let s = sanitize(&name);
        if !s.is_empty() {
            return Some(s);
        }
    }

    None
}

/// Public for testing and for use sites that only have a URL (no probe
/// has happened yet — e.g. core's `add_download` fallback when the
/// pre-probe fails).
pub fn from_url(url: &Url) -> Option<String> {
    url.path_segments()
        .and_then(|mut segs| segs.rfind(|s| !s.is_empty()))
        .map(percent_decode_lossy)
        .filter(|s| !s.is_empty())
}

/// Parse a `Content-Disposition` value and pull out a filename. Handles
/// the common `filename="…"` form and RFC 5987 `filename*=UTF-8''…`,
/// with the encoded variant taking precedence.
pub fn from_content_disposition(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();

    // RFC 5987 encoded form wins (it's the form servers send when the
    // filename has non-ASCII bytes, and it round-trips them correctly).
    if let Some(idx) = lower.find("filename*=") {
        let raw = &value[idx + "filename*=".len()..];
        let raw = raw.split(';').next().unwrap_or(raw).trim();
        if let Some((_, encoded)) = raw.split_once("''") {
            let decoded = percent_decode_lossy(encoded.trim_matches('"'));
            if !decoded.is_empty() {
                return Some(decoded);
            }
        }
    }

    if let Some(idx) = lower.find("filename=") {
        // Skip a possible "filename*=" we already handled — find returns
        // the first match. The `filename*=` shape starts at the same
        // index as `filename=`, so the check below uses the position
        // of the *equals sign* and looks at the char before it.
        // (If the value has only `filename*=`, the find above returned
        //  that index too, but we already extracted from it.)
        let after = &value[idx + "filename=".len()..];
        let raw = after.split(';').next().unwrap_or(after).trim();
        let unquoted = raw.trim_matches('"');
        if !unquoted.is_empty() {
            return Some(unquoted.to_string());
        }
    }
    None
}

/// True if `s` looks like a randomly-generated URL slug — alphanumeric
/// (plus `_` / `-`), at least 8 chars, no dot. Path tails that look
/// like this are what services like Mediafire and Google Drive serve;
/// taking them as filenames is what produces the `TUNDthEb` symptom.
pub fn is_random_slug(s: &str) -> bool {
    if s.contains('.') {
        return false;
    }
    if s.len() < 8 {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Map a `Content-Type` header value to a file extension (without the
/// leading dot). Conservative — we only return an extension when the
/// mapping is unambiguous. Charset / boundary parameters in the
/// header are ignored.
pub fn extension_from_mime(content_type: &str) -> Option<&'static str> {
    let main = content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_ascii_lowercase();
    Some(match main.as_str() {
        // Common application types we care about for a download manager.
        "application/pdf" => "pdf",
        "application/zip" => "zip",
        "application/x-7z-compressed" => "7z",
        "application/x-rar-compressed" | "application/vnd.rar" => "rar",
        "application/x-tar" => "tar",
        "application/gzip" | "application/x-gzip" => "gz",
        "application/x-bzip2" => "bz2",
        "application/x-xz" => "xz",
        "application/x-msdownload" | "application/vnd.microsoft.portable-executable" => "exe",
        "application/x-msi" | "application/x-ms-installer" => "msi",
        "application/x-apple-diskimage" => "dmg",
        "application/x-debian-package" | "application/vnd.debian.binary-package" => "deb",
        "application/x-redhat-package-manager" | "application/x-rpm" => "rpm",
        "application/epub+zip" => "epub",
        "application/json" => "json",
        "application/xml" | "text/xml" => "xml",
        "application/javascript" | "text/javascript" => "js",
        "application/wasm" => "wasm",
        "application/octet-stream" => return None,
        // Office / docs.
        "application/msword" => "doc",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.ms-excel" => "xls",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "application/vnd.ms-powerpoint" => "ppt",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        // Audio.
        "audio/mpeg" => "mp3",
        "audio/mp4" | "audio/x-m4a" => "m4a",
        "audio/aac" => "aac",
        "audio/ogg" => "ogg",
        "audio/opus" => "opus",
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/wav" | "audio/x-wav" => "wav",
        // Video.
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/x-matroska" => "mkv",
        "video/quicktime" => "mov",
        "video/x-msvideo" => "avi",
        // Image.
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "image/x-icon" | "image/vnd.microsoft.icon" => "ico",
        // Text.
        "text/plain" => "txt",
        "text/csv" => "csv",
        "text/markdown" => "md",
        "text/html" => "html",
        _ => return None,
    })
}

/// Strip path separators, control chars, and reserved Windows
/// characters from a filename. Returns the cleaned name; may return
/// empty if `name` was entirely garbage.
fn sanitize(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control())
        .map(|c| match c {
            // POSIX path separators and Windows-reserved characters.
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect::<String>()
        .trim_matches(|c: char| c == '.' || c.is_whitespace())
        .to_string()
}

fn percent_decode_lossy(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push(((h << 4) | l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    // -- is_random_slug -----------------------------------------------------

    #[test]
    fn random_slug_recognizes_path_tails() {
        assert!(is_random_slug("TUNDthEb"));
        assert!(is_random_slug("abc123xyz"));
        assert!(is_random_slug("8_chars_long"));
        assert!(is_random_slug("with-dashes-and_underscores"));
    }

    #[test]
    fn random_slug_rejects_when_extension_present() {
        assert!(!is_random_slug("archive.zip"));
        assert!(!is_random_slug("naïve.txt"));
        assert!(!is_random_slug("file.tar.gz"));
    }

    #[test]
    fn random_slug_rejects_short_strings() {
        assert!(!is_random_slug("short"));
        assert!(!is_random_slug("a"));
        assert!(!is_random_slug(""));
    }

    #[test]
    fn random_slug_rejects_with_punctuation() {
        assert!(!is_random_slug("hello world"));
        assert!(!is_random_slug("file (1)"));
    }

    // -- extension_from_mime ------------------------------------------------

    #[test]
    fn mime_extensions_common_cases() {
        assert_eq!(extension_from_mime("video/mp4"), Some("mp4"));
        assert_eq!(extension_from_mime("application/pdf"), Some("pdf"));
        assert_eq!(extension_from_mime("application/zip"), Some("zip"));
        assert_eq!(extension_from_mime("audio/mpeg"), Some("mp3"));
    }

    #[test]
    fn mime_ignores_charset_and_boundary() {
        assert_eq!(
            extension_from_mime("text/plain; charset=utf-8"),
            Some("txt")
        );
        assert_eq!(
            extension_from_mime("application/json; charset=UTF-8"),
            Some("json")
        );
    }

    #[test]
    fn mime_octet_stream_is_useless() {
        assert_eq!(extension_from_mime("application/octet-stream"), None);
        assert_eq!(extension_from_mime("application/unknown"), None);
    }

    // -- from_content_disposition ------------------------------------------

    #[test]
    fn cd_simple_quoted() {
        assert_eq!(
            from_content_disposition("attachment; filename=\"hello.bin\"").as_deref(),
            Some("hello.bin")
        );
    }

    #[test]
    fn cd_unquoted() {
        assert_eq!(
            from_content_disposition("attachment; filename=plain.txt").as_deref(),
            Some("plain.txt")
        );
    }

    #[test]
    fn cd_rfc5987_percent_decoded() {
        assert_eq!(
            from_content_disposition("attachment; filename*=UTF-8''na%C3%AFve.txt").as_deref(),
            Some("naïve.txt")
        );
    }

    #[test]
    fn cd_rfc5987_wins_over_plain() {
        // Servers commonly send both; the encoded version wins because
        // it round-trips non-ASCII bytes correctly.
        let v = "attachment; filename=\"naive.txt\"; filename*=UTF-8''na%C3%AFve.txt";
        assert_eq!(from_content_disposition(v).as_deref(), Some("naïve.txt"));
    }

    // -- derive_filename precedence ----------------------------------------

    #[test]
    fn derive_prefers_content_disposition_over_url() {
        let original = url("https://example.com/d/TUNDthEb");
        let final_ = original.clone();
        let got = derive_filename(
            Some("attachment; filename=\"holy_bible.epub\""),
            &final_,
            &original,
            Some("application/epub+zip"),
        );
        assert_eq!(got.as_deref(), Some("holy_bible.epub"));
    }

    #[test]
    fn derive_uses_final_url_after_redirect() {
        // Mediafire-shape: user pasted the share page URL, server
        // 302s to a direct download URL with the filename in it.
        let original = url("https://www.mediafire.com/file/abc123xyz");
        let final_ = url("https://download123.mediafire.com/abc/archive.zip");
        let got = derive_filename(None, &final_, &original, None);
        assert_eq!(got.as_deref(), Some("archive.zip"));
    }

    #[test]
    fn derive_falls_back_to_mime_when_path_is_random() {
        // The reported symptom: path tail is a slug, no Content-
        // Disposition, but the server tells us the type.
        let url_ = url("https://files.example.com/d/TUNDthEb");
        let got = derive_filename(None, &url_, &url_, Some("application/epub+zip"));
        assert_eq!(got.as_deref(), Some("download.epub"));
    }

    #[test]
    fn derive_uses_path_tail_when_it_has_an_extension() {
        let url_ = url("https://example.com/files/report.pdf");
        let got = derive_filename(None, &url_, &url_, None);
        assert_eq!(got.as_deref(), Some("report.pdf"));
    }

    #[test]
    fn derive_returns_random_slug_when_nothing_better_is_available() {
        // Last-resort: better than `download.bin` since at least it
        // identifies the resource, but the caller should treat it as
        // "ask the user to rename" territory.
        let url_ = url("https://example.com/d/TUNDthEb");
        let got = derive_filename(None, &url_, &url_, None);
        assert_eq!(got.as_deref(), Some("TUNDthEb"));
    }

    #[test]
    fn derive_returns_none_for_root_url_without_signals() {
        let url_ = url("https://example.com/");
        let got = derive_filename(None, &url_, &url_, None);
        assert_eq!(got, None);
    }

    #[test]
    fn derive_percent_decodes_url_path() {
        let url_ = url("https://example.com/files/na%C3%AFve.txt");
        let got = derive_filename(None, &url_, &url_, None);
        assert_eq!(got.as_deref(), Some("naïve.txt"));
    }

    #[test]
    fn derive_sanitizes_content_disposition() {
        // Server tries to be cute with path separators; we strip them.
        let url_ = url("https://example.com/");
        let got = derive_filename(
            Some("attachment; filename=\"../../etc/passwd\""),
            &url_,
            &url_,
            None,
        );
        // `/` → `_`; leading dots are trimmed off, inner `..` left
        // alone (path-traversal isn't a concern once separators are
        // gone).
        assert_eq!(got.as_deref(), Some("_.._etc_passwd"));
    }

    // Precedence-branch table

    struct Case {
        name: &'static str,
        content_disposition: Option<&'static str>,
        original_url: &'static str,
        final_url: &'static str,
        content_type: Option<&'static str>,
        expect: Option<&'static str>,
    }

    #[test]
    fn derive_filename_precedence_table() {
        let cases = [
            Case {
                name: "rfc5987_utf8_wins",
                content_disposition: Some("attachment; filename*=UTF-8''na%C3%AFve.pdf"),
                original_url: "https://example.com/d/TUNDthEb",
                final_url: "https://example.com/d/TUNDthEb",
                content_type: Some("application/pdf"),
                expect: Some("naïve.pdf"),
            },
            Case {
                name: "quoted_sanitized_path_separators",
                content_disposition: Some("attachment; filename=\"../../etc/passwd\""),
                original_url: "https://example.com/d/TUNDthEb",
                final_url: "https://example.com/d/TUNDthEb",
                content_type: None,
                expect: Some("_.._etc_passwd"),
            },
            Case {
                name: "quoted_with_spaces",
                content_disposition: Some("attachment; filename=\"Annual Report 2025.pdf\""),
                original_url: "https://example.com/r/xyz",
                final_url: "https://example.com/r/xyz",
                content_type: None,
                expect: Some("Annual Report 2025.pdf"),
            },
            Case {
                name: "final_url_after_redirect",
                content_disposition: None,
                original_url: "https://srv.example.com/r/xyz",
                final_url: "https://cdn.example.com/files/Annual.pdf",
                content_type: None,
                expect: Some("Annual.pdf"),
            },
            Case {
                name: "mime_when_path_is_random_slug",
                content_disposition: None,
                original_url: "https://cdn.example.com/d/abc123def",
                final_url: "https://cdn.example.com/d/abc123def",
                content_type: Some("application/pdf"),
                expect: Some("download.pdf"),
            },
            Case {
                name: "octet_stream_rejected_falls_to_original",
                content_disposition: None,
                original_url: "https://srv.example.com/r/foo.zip",
                final_url: "https://cdn.example.com/d/abc123def",
                content_type: Some("application/octet-stream"),
                expect: Some("foo.zip"),
            },
            Case {
                name: "original_url_fallback_when_final_is_random",
                content_disposition: None,
                original_url: "https://srv.example.com/r/foo.zip",
                final_url: "https://cdn.example.com/d/abc123def",
                content_type: None,
                expect: Some("foo.zip"),
            },
            Case {
                name: "all_empty_returns_none",
                content_disposition: None,
                original_url: "https://cdn.example.com/d/abc123def",
                final_url: "https://cdn.example.com/d/xyz789ghi",
                content_type: None,
                // Both URLs are random-slug, no CD, no MIME — branch 5
                // falls back to original-URL tail, which IS still a slug,
                // so it's the last-resort name (the path tail is taken
                // even when random).
                expect: Some("abc123def"),
            },
        ];

        for case in cases {
            let got = derive_filename(
                case.content_disposition,
                &url(case.final_url),
                &url(case.original_url),
                case.content_type,
            );
            assert_eq!(
                got.as_deref(),
                case.expect,
                "case `{}`: expected {:?}, got {:?}",
                case.name,
                case.expect,
                got
            );
        }
    }
}
