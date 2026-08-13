//! Parser for yt-dlp's `--progress-template` output.
//!
//! We invoke yt-dlp with `--newline` and a six-field, pipe-delimited
//! download template (see `ytdlp::mod`'s `download()` for the literal):
//! downloaded bytes, total bytes, speed, ETA, fragment index, fragment
//! count. Any field may be the literal `"NA"` (yt-dlp's sentinel) or
//! `"None"` for unknown values. The parser converts those to `None`.
//!
//! yt-dlp also emits non-progress lines on stdout — `[download] …`
//! summaries, the post-processor banner, etc. The parser returns `None`
//! for any line that doesn't match the expected shape so the caller can
//! skip it.

use std::time::Duration;

/// Field count of the download progress template. A line with any other
/// number of `|`-separated fields is not one of ours.
const FIELD_COUNT: usize = 6;

/// One parsed progress tick from a yt-dlp child process.
#[derive(Debug, Clone, PartialEq)]
pub struct Tick {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub speed_bps: Option<f64>,
    pub eta: Option<Duration>,
    /// 1-based index of the fragment currently being written, for
    /// fragmented (HLS / DASH) transfers. `None` for plain HTTP.
    pub fragment_index: Option<u64>,
    /// Total fragment count when yt-dlp knows it up front. `None` for
    /// plain HTTP and for live manifests of unknown length.
    pub fragment_count: Option<u64>,
}

impl Tick {
    /// Best available total-size figure for this tick.
    ///
    /// Fragmented media frequently reports neither `total_bytes` nor
    /// `total_bytes_estimate` — an HLS media playlist has no
    /// Content-Length for the whole stream, and yt-dlp's own estimate is
    /// absent on the first tick and on live-derived manifests. Without a
    /// total the UI has nothing to draw and the bar sits empty for the
    /// entire download, which is exactly what extension-captured HLS
    /// streams looked like.
    ///
    /// Fragment counts are the fallback: fragments of a given rendition
    /// are near-uniform in duration, so scaling the bytes we already have
    /// by `count / index` estimates the whole. Deliberately an estimate —
    /// it drifts by a few percent and firms up as `index` grows, which
    /// still beats no bar at all.
    pub fn effective_total(&self) -> Option<u64> {
        if let Some(total) = self.total {
            return Some(total);
        }
        let (index, count) = (self.fragment_index?, self.fragment_count?);
        // `index` is 1-based; guard the pre-first-fragment tick and any
        // count yt-dlp revises downward mid-run.
        if index == 0 || count < index || self.downloaded == 0 {
            return None;
        }
        Some(((self.downloaded as u128 * count as u128) / index as u128) as u64)
    }
}

/// Parse one line of `--progress-template` output. Returns `None` for
/// non-progress lines (banners, warnings, post-processor output).
pub fn parse_line(line: &str) -> Option<Tick> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let parts: Vec<&str> = line.split('|').collect();
    if parts.len() != FIELD_COUNT {
        return None;
    }
    let downloaded = parse_u64(parts[0])?;
    let total = parse_u64(parts[1]);
    let speed_bps = parse_f64(parts[2]);
    let eta = parse_u64(parts[3]).map(Duration::from_secs);
    let fragment_index = parse_u64(parts[4]);
    let fragment_count = parse_u64(parts[5]);
    Some(Tick {
        downloaded,
        total,
        speed_bps,
        eta,
        fragment_index,
        fragment_count,
    })
}

fn is_missing(s: &str) -> bool {
    matches!(s, "" | "NA" | "None" | "null")
}

fn parse_u64(s: &str) -> Option<u64> {
    let s = s.trim();
    if is_missing(s) {
        return None;
    }
    // yt-dlp formats numeric fields without commas or units when using the
    // raw `%(progress.X)s` template, but a defensive float parse handles
    // the rare extractor that emits "12345.0".
    if let Ok(n) = s.parse::<u64>() {
        return Some(n);
    }
    s.parse::<f64>().ok().map(|f| f as u64)
}

fn parse_f64(s: &str) -> Option<f64> {
    let s = s.trim();
    if is_missing(s) {
        return None;
    }
    s.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_tick_parses() {
        let line = "1048576|10485760|524288.0|18|NA|NA";
        let t = parse_line(line).unwrap();
        assert_eq!(t.downloaded, 1_048_576);
        assert_eq!(t.total, Some(10_485_760));
        assert!((t.speed_bps.unwrap() - 524_288.0).abs() < f64::EPSILON);
        assert_eq!(t.eta, Some(Duration::from_secs(18)));
        assert_eq!(t.effective_total(), Some(10_485_760));
    }

    #[test]
    fn fragmented_tick_parses_fragment_fields() {
        let line = "1048576|NA|524288.0|NA|10|100";
        let t = parse_line(line).unwrap();
        assert_eq!(t.downloaded, 1_048_576);
        assert!(t.total.is_none());
        assert_eq!(t.fragment_index, Some(10));
        assert_eq!(t.fragment_count, Some(100));
    }

    #[test]
    fn na_fields_become_none() {
        let line = "1024|NA|NA|NA|NA|NA";
        let t = parse_line(line).unwrap();
        assert_eq!(t.downloaded, 1024);
        assert!(t.total.is_none());
        assert!(t.speed_bps.is_none());
        assert!(t.eta.is_none());
        assert!(t.fragment_index.is_none());
        assert!(t.fragment_count.is_none());
    }

    #[test]
    fn none_string_also_missing() {
        // Some extractors emit "None" rather than "NA".
        let line = "0|None|None|None|None|None";
        let t = parse_line(line).unwrap();
        assert_eq!(t.downloaded, 0);
        assert!(t.total.is_none());
    }

    #[test]
    fn banner_lines_return_none() {
        assert!(parse_line("[download] Destination: video.mp4").is_none());
        assert!(parse_line("[ffmpeg] Merging formats into \"out.mkv\"").is_none());
        assert!(parse_line("").is_none());
    }

    #[test]
    fn wrong_field_count_returns_none() {
        // The four-field shape we used before the fragment fields were
        // added must not parse as a truncated six-field tick.
        assert!(parse_line("1024|2048|512.0|4").is_none());
    }

    #[test]
    fn float_downloaded_tolerated() {
        let t = parse_line("12345.0|NA|NA|NA|NA|NA").unwrap();
        assert_eq!(t.downloaded, 12_345);
    }

    #[test]
    fn effective_total_estimates_from_fragment_counts() {
        // 10 of 100 fragments done for 1 MiB, so the whole is ~10 MiB.
        let t = parse_line("1048576|NA|NA|NA|10|100").unwrap();
        assert_eq!(t.effective_total(), Some(10_485_760));
    }

    #[test]
    fn effective_total_prefers_a_real_total_over_the_estimate() {
        let t = parse_line("1048576|9999999|NA|NA|10|100").unwrap();
        assert_eq!(t.effective_total(), Some(9_999_999));
    }

    #[test]
    fn effective_total_is_none_without_usable_inputs() {
        // No total, no fragment counts.
        assert_eq!(
            parse_line("1024|NA|NA|NA|NA|NA").unwrap().effective_total(),
            None
        );
        // Fragment index ahead of a stale count would extrapolate backwards.
        assert_eq!(
            parse_line("1024|NA|NA|NA|101|100")
                .unwrap()
                .effective_total(),
            None
        );
        // Nothing downloaded yet — scaling zero says nothing.
        assert_eq!(
            parse_line("0|NA|NA|NA|1|100").unwrap().effective_total(),
            None
        );
    }
}
