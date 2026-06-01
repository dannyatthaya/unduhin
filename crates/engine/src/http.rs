//! Thin wrappers around reqwest for probing and ranged GETs.

use std::time::Duration;

use reqwest::header::{
    HeaderMap, HeaderName, HeaderValue, ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH,
    CONTENT_TYPE, ETAG, LAST_MODIFIED, RANGE,
};
use reqwest::{Client, Response, StatusCode};
use url::Url;

use crate::error::{EngineError, Result};
use crate::filename::derive_filename;

/// What the engine learned about a remote resource before opening any ranged
/// connections.
#[derive(Debug, Clone)]
pub struct RemoteInfo {
    /// Final URL after redirects.
    pub url: Url,
    /// Total body length in bytes, if the server reported it.
    pub content_length: Option<u64>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    /// True iff the server advertised `Accept-Ranges: bytes`.
    pub accept_ranges: bool,
    /// Best-effort filename derived from Content-Disposition or the URL path.
    pub filename_hint: Option<String>,
}

/// Header names the engine refuses to send on the caller's behalf. These
/// fall into two buckets:
///
/// - **Per-request overrides** (`range`, `host`, `content-length`,
///   `content-encoding`, `connection`, `accept-encoding`, `te`,
///   `transfer-encoding`, `upgrade`) — must be set by the transport layer
///   on each request; letting a default header replay them across all
///   requests would corrupt ranged GETs.
/// - **Hop-by-hop / proxy auth** (`proxy-authorization`,
///   `proxy-authenticate`) — captured by the extension from the browser's
///   internal request chain but never meaningful when reqwest establishes
///   its own connection.
///
/// Case-insensitive; entries are stored lower-cased.
pub const HEADER_DROP_LIST: &[&str] = &[
    "range",
    "host",
    "content-length",
    "content-encoding",
    "connection",
    "accept-encoding",
    "te",
    "transfer-encoding",
    "upgrade",
    "proxy-authorization",
    "proxy-authenticate",
];

/// Build a reqwest client with sensible defaults. Pass `None` for
/// `user_agent` to keep the engine's compiled-in default.
///
/// `extra_headers` is replayed on every request via `default_headers`.
/// Names on the [`HEADER_DROP_LIST`] are silently dropped — passing them
/// is not an error because they typically arrive as part of a captured
/// browser request and the caller would otherwise have to filter them
/// itself.
pub fn build_client(
    connect_timeout: Duration,
    read_timeout: Duration,
    user_agent: Option<&str>,
    extra_headers: &[(String, String)],
) -> Result<Client> {
    let ua = user_agent
        .map(|s| s.to_string())
        .unwrap_or_else(|| concat!("unduhin/", env!("CARGO_PKG_VERSION")).to_string());
    let headers = sanitize_headers(extra_headers);
    Client::builder()
        .connect_timeout(connect_timeout)
        .read_timeout(read_timeout)
        .user_agent(ua)
        .default_headers(headers)
        .build()
        .map_err(EngineError::from)
}

/// Filter `pairs` against [`HEADER_DROP_LIST`] and convert into a
/// [`HeaderMap`]. Bad bytes in either the name or the value are logged
/// and skipped rather than panicking — the captured headers come from
/// untrusted browser input, so a single malformed entry should never
/// poison the whole download.
fn sanitize_headers(pairs: &[(String, String)]) -> HeaderMap {
    let mut out = HeaderMap::with_capacity(pairs.len());
    for (name, value) in pairs {
        let lower = name.to_ascii_lowercase();
        if HEADER_DROP_LIST.iter().any(|d| *d == lower) {
            tracing::trace!(header = %name, "engine::http: dropping per-request header");
            continue;
        }
        let header_name = match HeaderName::try_from(name.as_str()) {
            Ok(n) => n,
            Err(_) => {
                tracing::warn!(header = %name, "engine::http: invalid header name; skipping");
                continue;
            }
        };
        let header_value = match HeaderValue::try_from(value.as_str()) {
            Ok(v) => v,
            Err(_) => {
                tracing::warn!(header = %name, "engine::http: invalid header value; skipping");
                continue;
            }
        };
        out.append(header_name, header_value);
    }
    out
}

/// Issue a HEAD request and parse the headers we care about. Falls back to a
/// ranged GET (Range: bytes=0-0) if the server rejects HEAD with 405.
pub async fn probe(client: &Client, url: &Url) -> Result<RemoteInfo> {
    let resp = client.head(url.clone()).send().await?;
    let resp = if resp.status() == StatusCode::METHOD_NOT_ALLOWED {
        tracing::debug!("HEAD not allowed; falling back to ranged GET for probe");
        client
            .get(url.clone())
            .header(RANGE, "bytes=0-0")
            .send()
            .await?
    } else {
        resp
    };

    let status = resp.status();
    if !status.is_success() && status != StatusCode::PARTIAL_CONTENT {
        return Err(map_status_error(status.as_u16()));
    }

    Ok(parse_remote_info(url, &resp))
}

pub(crate) fn parse_remote_info(original_url: &Url, resp: &Response) -> RemoteInfo {
    let headers = resp.headers();

    let content_length = headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());

    let etag = header_string(headers, &ETAG);
    let last_modified = header_string(headers, &LAST_MODIFIED);

    let accept_ranges = headers
        .get(ACCEPT_RANGES)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.eq_ignore_ascii_case("bytes"))
        .unwrap_or(false);

    let content_disposition = header_string(headers, &CONTENT_DISPOSITION);
    let content_type = header_string(headers, &CONTENT_TYPE);
    let final_url = resp.url().clone();
    let filename_hint = derive_filename(
        content_disposition.as_deref(),
        &final_url,
        original_url,
        content_type.as_deref(),
    );

    RemoteInfo {
        url: final_url,
        content_length,
        etag,
        last_modified,
        accept_ranges,
        filename_hint,
    }
}

fn header_string(
    headers: &reqwest::header::HeaderMap,
    name: &reqwest::header::HeaderName,
) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

pub(crate) fn map_status_error(status: u16) -> EngineError {
    use crate::retry::{classify_status, RetryClass};
    match classify_status(status) {
        RetryClass::Terminal => EngineError::TerminalStatus { status },
        RetryClass::Transient => EngineError::TransientStatus { status },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_client_with_default_user_agent_succeeds() {
        let c = build_client(Duration::from_secs(5), Duration::from_secs(30), None, &[]);
        assert!(c.is_ok());
    }

    #[test]
    fn build_client_with_custom_user_agent_succeeds() {
        let c = build_client(
            Duration::from_secs(5),
            Duration::from_secs(30),
            Some("curl/8.6"),
            &[],
        );
        assert!(c.is_ok());
    }

    #[test]
    fn sanitize_drops_disallowed_names_case_insensitively() {
        let pairs = vec![
            ("RANGE".into(), "bytes=0-100".into()),
            ("Host".into(), "example.com".into()),
            ("content-LENGTH".into(), "10".into()),
            ("Cookie".into(), "a=b".into()),
            ("Referer".into(), "https://example.com/".into()),
            ("Accept-Encoding".into(), "gzip".into()),
            ("Proxy-Authorization".into(), "Basic abc".into()),
        ];
        let map = sanitize_headers(&pairs);
        // Range / Host / Content-Length / Accept-Encoding /
        // Proxy-Authorization all gone — only Cookie + Referer survive.
        assert!(!map.contains_key("range"));
        assert!(!map.contains_key("host"));
        assert!(!map.contains_key("content-length"));
        assert!(!map.contains_key("accept-encoding"));
        assert!(!map.contains_key("proxy-authorization"));
        assert_eq!(map.get("cookie").unwrap(), "a=b");
        assert_eq!(map.get("referer").unwrap(), "https://example.com/");
    }

    #[test]
    fn sanitize_skips_invalid_bytes_without_panic() {
        let pairs = vec![
            // Whitespace in name is illegal — HeaderName::try_from rejects.
            ("Bad Name".into(), "value".into()),
            // CR/LF in value is illegal — HeaderValue::try_from rejects.
            ("X-Ok".into(), "line1\r\nline2".into()),
            // Sane pair — must survive.
            ("X-Foo".into(), "bar".into()),
        ];
        let map = sanitize_headers(&pairs);
        assert!(!map.contains_key("bad name"));
        assert!(!map.contains_key("x-ok"));
        assert_eq!(map.get("x-foo").unwrap(), "bar");
    }

    #[test]
    fn sanitize_empty_input_yields_empty_map() {
        assert!(sanitize_headers(&[]).is_empty());
    }
}
