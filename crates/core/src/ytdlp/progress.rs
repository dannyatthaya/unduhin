//! Parser for yt-dlp's `--progress-template` output.
//!
//! We invoke yt-dlp with
//! `--newline --progress-template "%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.speed)s|%(progress.eta)s"`
//! so each tick is a single newline-terminated line of four pipe-delimited
//! fields. Any field may be the literal `"NA"` (yt-dlp's sentinel) or
//! `"None"` for unknown values. The parser converts those to `None`.
//!
//! yt-dlp also emits non-progress lines on stdout — `[download] …`
//! summaries, the post-processor banner, etc. The parser returns `None`
//! for any line that doesn't match the expected shape so the caller can
//! skip it.

use std::time::Duration;

/// One parsed progress tick from a yt-dlp child process.
#[derive(Debug, Clone, PartialEq)]
pub struct Tick {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub speed_bps: Option<f64>,
    pub eta: Option<Duration>,
}

/// Parse one line of `--progress-template` output. Returns `None` for
/// non-progress lines (banners, warnings, post-processor output).
pub fn parse_line(line: &str) -> Option<Tick> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let parts: Vec<&str> = line.split('|').collect();
    if parts.len() != 4 {
        return None;
    }
    let downloaded = parse_u64(parts[0])?;
    let total = parse_u64(parts[1]);
    let speed_bps = parse_f64(parts[2]);
    let eta = parse_u64(parts[3]).map(Duration::from_secs);
    Some(Tick {
        downloaded,
        total,
        speed_bps,
        eta,
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
        let line = "1048576|10485760|524288.0|18";
        let t = parse_line(line).unwrap();
        assert_eq!(t.downloaded, 1_048_576);
        assert_eq!(t.total, Some(10_485_760));
        assert!((t.speed_bps.unwrap() - 524_288.0).abs() < f64::EPSILON);
        assert_eq!(t.eta, Some(Duration::from_secs(18)));
    }

    #[test]
    fn na_fields_become_none() {
        let line = "1024|NA|NA|NA";
        let t = parse_line(line).unwrap();
        assert_eq!(t.downloaded, 1024);
        assert!(t.total.is_none());
        assert!(t.speed_bps.is_none());
        assert!(t.eta.is_none());
    }

    #[test]
    fn none_string_also_missing() {
        // Some extractors emit "None" rather than "NA".
        let line = "0|None|None|None";
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
    fn float_downloaded_tolerated() {
        let t = parse_line("12345.0|NA|NA|NA").unwrap();
        assert_eq!(t.downloaded, 12_345);
    }
}
