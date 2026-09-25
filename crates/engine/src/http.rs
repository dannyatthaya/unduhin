//! Thin wrappers around reqwest for probing and ranged GETs.

use std::time::Duration;

use reqwest::header::{
    HeaderMap, HeaderName, HeaderValue, ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH,
    CONTENT_RANGE, CONTENT_TYPE, ETAG, LAST_MODIFIED, LOCATION, RANGE, USER_AGENT,
};
use reqwest::{Method, Response, StatusCode};
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
    /// Raw `Content-Type` header (incl. any `; charset=` suffix), if the
    /// server sent one. The completion gate in `core` uses this to reject
    /// one-click file hosts that serve an HTML landing page instead of the
    /// requested bytes.
    pub content_type: Option<String>,
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

/// Most redirects followed for one request, matching reqwest's default.
const MAX_REDIRECTS: usize = 10;

/// Captured headers that still go out after a redirect leaves the origin
/// the request started on: the ones a browser itself sends to any site, and
/// none that authenticate. Everything else a browser capture can carry —
/// `Cookie`, `Authorization`, a site's own `X-Api-Key` — is only ever sent
/// to the origin it was captured for.
fn is_cross_origin_safe(name: &HeaderName) -> bool {
    is_ordinary_browser_header(name.as_str())
}

/// Whether `name` (any case) is one of the ordinary, non-credential headers
/// a browser sends to any site — the set [`Client`] keeps across origins.
/// Also what the app keeps when it cannot encrypt a capture at rest.
pub fn is_ordinary_browser_header(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    let n = n.as_str();
    matches!(
        n,
        "accept"
            | "accept-language"
            | "cache-control"
            | "dnt"
            | "pragma"
            | "priority"
            | "referer"
            | "upgrade-insecure-requests"
            // Set on the client builder rather than carried in the header
            // map, so listing it changes nothing for `Client`; it is here
            // for the other callers.
            | "user-agent"
    ) || n.starts_with("sec-fetch-")
        || n.starts_with("sec-ch-")
}

/// HTTP client used by the engine: reqwest plus redirect handling that
/// keeps captured credentials on the origin they belong to.
///
/// reqwest follows redirects itself, but only strips its fixed list of
/// sensitive headers (`Cookie`, `Authorization`, …) when a redirect changes
/// host. A browser capture carries arbitrary headers, and a site-specific
/// token header would have followed a redirect to any host the server
/// named. So automatic redirects are off, [`RequestBuilder::send`] follows
/// them by hand, and once a redirect leaves the starting origin the rest of
/// the chain uses a client that only carries [`is_cross_origin_safe`]
/// headers.
#[derive(Debug, Clone)]
pub struct Client {
    /// Every captured header.
    full: reqwest::Client,
    /// Only the cross-origin-safe ones. Same as `full` when nothing was
    /// filtered out.
    cross_origin: reqwest::Client,
}

impl Client {
    pub fn get(&self, url: Url) -> RequestBuilder<'_> {
        self.request(Method::GET, url)
    }

    pub fn head(&self, url: Url) -> RequestBuilder<'_> {
        self.request(Method::HEAD, url)
    }

    fn request(&self, method: Method, url: Url) -> RequestBuilder<'_> {
        RequestBuilder {
            client: self,
            method,
            url,
            headers: HeaderMap::new(),
            invalid_header: None,
        }
    }
}

/// One request on a [`Client`]. Headers set here (`Range`, `If-Range`) are
/// per-request and are re-sent on every redirect hop.
#[must_use]
pub struct RequestBuilder<'a> {
    client: &'a Client,
    method: Method,
    url: Url,
    headers: HeaderMap,
    invalid_header: Option<HeaderName>,
}

impl RequestBuilder<'_> {
    pub fn header<V>(mut self, name: HeaderName, value: V) -> Self
    where
        HeaderValue: TryFrom<V>,
    {
        match HeaderValue::try_from(value) {
            Ok(v) => {
                self.headers.insert(name, v);
            }
            Err(_) => self.invalid_header = Some(name),
        }
        self
    }

    /// Send the request, following up to [`MAX_REDIRECTS`] redirects.
    pub async fn send(self) -> Result<Response> {
        let RequestBuilder {
            client,
            mut method,
            mut url,
            headers,
            invalid_header,
        } = self;
        if let Some(name) = invalid_header {
            return Err(EngineError::other(format!(
                "invalid value for header {name}"
            )));
        }
        let origin = url.origin();
        let mut left_origin = false;
        for _ in 0..=MAX_REDIRECTS {
            let http = if left_origin {
                &client.cross_origin
            } else {
                &client.full
            };
            let resp = http
                .request(method.clone(), url.clone())
                .headers(headers.clone())
                .send()
                .await?;
            let Some(next) = redirect_target(&resp, &url) else {
                return Ok(resp);
            };
            if resp.status() == StatusCode::SEE_OTHER && method != Method::HEAD {
                method = Method::GET;
            }
            // Sticky: a chain that comes back to the origin after leaving it
            // has already been through a host that could have chosen where
            // to send us next.
            left_origin |= next.origin() != origin;
            url = next;
        }
        Err(EngineError::other(format!(
            "too many redirects (more than {MAX_REDIRECTS})"
        )))
    }
}

/// Where a redirect response points, or `None` when `resp` is not one to
/// follow (not a 301/302/303/307/308, no usable `Location`, or a target
/// that is not http(s)) — in which case it is returned to the caller as is.
fn redirect_target(resp: &Response, current: &Url) -> Option<Url> {
    if !matches!(
        resp.status(),
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    ) {
        return None;
    }
    let location = resp.headers().get(LOCATION)?.to_str().ok()?;
    let next = current.join(location).ok()?;
    matches!(next.scheme(), "http" | "https").then_some(next)
}

/// Build the engine's HTTP client. Pass `None` for `user_agent` to keep
/// the engine's compiled-in default.
///
/// `extra_headers` is replayed on every request via `default_headers`
/// (only the cross-origin-safe subset once a redirect leaves the starting
/// origin — see [`Client`]). Names on the [`HEADER_DROP_LIST`] are silently
/// dropped — passing them is not an error because they typically arrive as
/// part of a captured browser request and the caller would otherwise have
/// to filter them itself.
pub fn build_client(
    connect_timeout: Duration,
    read_timeout: Duration,
    user_agent: Option<&str>,
    extra_headers: &[(String, String)],
) -> Result<Client> {
    let mut headers = sanitize_headers(extra_headers);
    // User-Agent gets a single, deterministic slot on the builder rather
    // than riding along in `default_headers` (where it would compete with
    // the builder's own UA). Precedence: an explicit `user_agent` override
    // (the app's global setting) wins; otherwise the browser's captured
    // `User-Agent` (forwarded in `extra_headers`) is used so the request
    // matches what the page made — essential for hosts that bind a
    // session / anti-bot cookie (e.g. `cf_clearance`) to the exact UA.
    // Falling back to the compiled-in default last. Pulling it out of
    // `headers` guarantees we never send two `User-Agent` values.
    let captured_ua = headers
        .remove(USER_AGENT)
        .and_then(|v| v.to_str().ok().map(|s| s.to_string()));
    let ua = user_agent
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .or(captured_ua)
        .unwrap_or_else(|| concat!("unduhin/", env!("CARGO_PKG_VERSION")).to_string());
    let cross_origin_headers: HeaderMap = headers
        .iter()
        .filter(|(name, _)| is_cross_origin_safe(name))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let all_safe = cross_origin_headers.len() == headers.len();
    let make = |headers: HeaderMap| {
        reqwest::Client::builder()
            .connect_timeout(connect_timeout)
            .read_timeout(read_timeout)
            .user_agent(ua.clone())
            .default_headers(headers)
            // Save exactly the bytes the server sends. With decoding on,
            // reqwest asked for `gzip` and decoded it, so a `.tar.gz` served
            // with `Content-Encoding: gzip` (or a text file a CDN
            // compresses) was not saved as served — and its range offsets
            // and lengths no longer matched the bytes on disk. A download
            // manager wants the representation, not a decoded view of it.
            .no_gzip()
            // Followed by hand in `RequestBuilder::send`.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(EngineError::from)
    };
    let full = make(headers)?;
    let cross_origin = if all_safe {
        full.clone()
    } else {
        make(cross_origin_headers)?
    };
    Ok(Client { full, cross_origin })
}

/// Check that a `206` really starts where `requested_start` asked it to.
///
/// The engine writes a range body at the offset it requested, so a server
/// that answers `206` with some other range (clamped, rounded to a chunk, or
/// simply from the start) would corrupt the file without any other error.
/// A `206` with no `Content-Range` is accepted, as before: it gives nothing
/// to check against, and rejecting it would break servers that worked.
pub(crate) fn check_range_start(resp: &Response, requested_start: u64) -> Result<()> {
    match content_range_start(resp) {
        Some(served) if served != requested_start => Err(EngineError::RangeMismatch {
            requested: requested_start,
            served,
        }),
        _ => Ok(()),
    }
}

/// First byte offset of a `Content-Range: bytes <first>-<last>/<len>`
/// header, if there is a parseable one.
fn content_range_start(resp: &Response) -> Option<u64> {
    let raw = resp.headers().get(CONTENT_RANGE)?.to_str().ok()?;
    let range = raw.trim().strip_prefix("bytes")?.trim_start();
    let (first, _) = range.split_once('-')?;
    first.trim().parse().ok()
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
        content_type,
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
