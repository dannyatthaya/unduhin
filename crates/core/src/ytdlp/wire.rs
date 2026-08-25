//! Deserializers for yt-dlp's `--dump-single-json` output and the lift
//! into our public [`ProbeResult`] / [`Format`] types.
//!
//! yt-dlp's JSON schema is large and not strictly typed. Every field here
//! is wrapped in `Option` with `#[serde(default)]` so a missing/renamed
//! field in a future yt-dlp release degrades to "we don't know" instead
//! of failing the probe entirely. Fixture tests in this module pin the
//! shape against captured outputs from known extractors.

use serde::Deserialize;

use crate::wire::MediaFormat;

use super::{Format, ProbeResult};

#[derive(Debug, Deserialize, Default)]
pub(super) struct RawInfo {
    #[serde(default)]
    pub webpage_url: Option<String>,
    #[serde(default)]
    pub original_url: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub extractor: Option<String>,
    #[serde(default)]
    pub extractor_key: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub uploader: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub is_live: Option<bool>,
    #[serde(default)]
    pub age_limit: Option<u32>,
    #[serde(default)]
    pub formats: Vec<RawFormat>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct RawFormat {
    #[serde(default)]
    pub format_id: Option<String>,
    #[serde(default)]
    pub ext: Option<String>,
    /// The actual fetchable media URL for this format (segment/manifest
    /// URL for HLS/DASH, direct file URL otherwise). Deliberately never
    /// surfaced on the public [`Format`] type — only consumed by
    /// [`RawInfo::into_media_formats`] for the extension-facing
    /// `Outbound::MediaFormats` reply, which is a distinct wire path from
    /// `ProbeResult`/`Format` (see [`MediaFormat`]'s doc comment).
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub resolution: Option<String>,
    #[serde(default)]
    pub fps: Option<f64>,
    #[serde(default)]
    pub vcodec: Option<String>,
    #[serde(default)]
    pub acodec: Option<String>,
    #[serde(default)]
    pub filesize: Option<u64>,
    #[serde(default)]
    pub filesize_approx: Option<u64>,
    #[serde(default)]
    pub tbr: Option<f64>,
    #[serde(default)]
    pub format_note: Option<String>,
}

impl RawInfo {
    /// Lift the raw yt-dlp JSON into a [`ProbeResult`]. Missing required
    /// fields (extractor, title) fall back to sensible placeholders so a
    /// stripped-down extractor still produces something the UI can show.
    pub(super) fn into_probe(self, requested_url: &str) -> ProbeResult {
        let url = self
            .webpage_url
            .or(self.original_url)
            .or(self.url)
            .unwrap_or_else(|| requested_url.to_string());

        let extractor = self
            .extractor
            .or(self.extractor_key)
            .unwrap_or_else(|| "generic".to_string())
            .to_ascii_lowercase();

        let title = self.title.unwrap_or_else(|| "Untitled".to_string());

        // Compute the recommended selectors against the raw formats so we
        // have direct access to `height` / `tbr` / `filesize` for scoring.
        // yt-dlp's array ordering is not guaranteed (best-first vs.
        // worst-first varies by extractor and yt-dlp version), so picking
        // by position alone gives nonsense like 144p for "best video".
        let recommended_video_audio = pick_best_video_audio(&self.formats);
        let recommended_audio_only = pick_best_audio_only(&self.formats);

        let formats: Vec<Format> = self.formats.into_iter().map(Format::from_raw).collect();

        ProbeResult {
            url,
            extractor,
            title,
            uploader: self.uploader.or(self.channel),
            duration_secs: self.duration.map(|d| d as u32),
            thumbnail_url: self.thumbnail,
            is_live: self.is_live.unwrap_or(false),
            age_limit: self.age_limit,
            formats,
            recommended_video_audio,
            recommended_audio_only,
        }
    }

    /// Lift into the browser-extension-facing [`MediaFormat`] list used by
    /// `Outbound::MediaFormats`. A separate mapping from [`into_probe`]
    /// (not an extra field bolted onto [`ProbeResult`]/[`Format`]) — see
    /// [`MediaFormat`]'s doc comment for why the two shapes are kept
    /// deliberately apart.
    ///
    /// [`into_probe`]: RawInfo::into_probe
    pub(super) fn into_media_formats(self) -> Vec<MediaFormat> {
        // `duration` is top-level, not per-format, so it has to be read
        // here and pushed down onto every row.
        let duration_secs = self.duration.filter(|d| *d > 0.0).map(|d| d as u32);
        media_formats_from_raw(&self.formats, duration_secs)
    }
}

impl Format {
    pub(super) fn from_raw(raw: RawFormat) -> Self {
        let resolution = derive_resolution(&raw);
        Self {
            format_id: raw.format_id.unwrap_or_default(),
            ext: raw.ext.unwrap_or_default(),
            resolution,
            fps: raw.fps.map(|f| f.round() as u32),
            vcodec: raw.vcodec,
            acodec: raw.acodec,
            filesize_bytes: raw.filesize.or(raw.filesize_approx),
            tbr_kbps: raw.tbr,
            note: raw.format_note,
        }
    }
}

/// Shared by [`Format::from_raw`] and [`media_formats_from_raw`] so the
/// resolution-string derivation (explicit `resolution` field, else
/// `width`x`height`, else `{height}p`, else "audio only" for an
/// audio-only format with neither) lives in exactly one place.
fn derive_resolution(raw: &RawFormat) -> Option<String> {
    raw.resolution
        .clone()
        .or_else(|| match (raw.width, raw.height) {
            (Some(w), Some(h)) => Some(format!("{w}x{h}")),
            (None, Some(h)) => Some(format!("{h}p")),
            _ => match raw.vcodec.as_deref() {
                Some("none") | None if raw.acodec.is_some() => Some("audio only".to_string()),
                _ => None,
            },
        })
}

/// Filter/dedupe/sort raw yt-dlp formats into the [`MediaFormat`] list the
/// extension's discovery popup renders — mirroring the extension's own
/// manifest-sniffed ordering (height descending, then bandwidth
/// descending) so a URL discovered via this in-app probe and one
/// discovered by the extension's own HLS/DASH sniffer present identically.
///
/// - Formats with no `url` are dropped — yt-dlp's `--dump-single-json`
///   can emit entries (storyboard/thumbnail tracks, some subtitle-only
///   rows) that aren't independently fetchable as media.
/// - Audio-only formats (`vcodec == "none"`) are dropped — this reply
///   feeds the extension's "download this video" picker, not a
///   standalone audio-extraction surface.
/// - Duplicate URLs are deduped, keeping the first-seen entry (yt-dlp's
///   own array ordering).
fn media_formats_from_raw(formats: &[RawFormat], duration_secs: Option<u32>) -> Vec<MediaFormat> {
    let mut seen_urls = std::collections::HashSet::new();
    let mut out: Vec<MediaFormat> = formats
        .iter()
        .filter(|f| f.vcodec.as_deref() != Some("none"))
        .filter_map(|f| {
            let url = f.url.clone()?;
            if !seen_urls.insert(url.clone()) {
                return None;
            }
            // yt-dlp's `tbr` is total bitrate in kbps; `bandwidth`
            // mirrors the bits/sec unit of an HLS manifest's
            // `EXT-X-STREAM-INF:BANDWIDTH` attribute, which is what
            // the extension's own sniffed formats carry — keeping the
            // unit consistent is what makes the descending sort below
            // actually comparable across the two discovery paths.
            let bandwidth = f.tbr.map(|kbps| (kbps.max(0.0) * 1000.0).round() as u64);
            // yt-dlp leaves both size fields empty on most HLS formats.
            // Fall back to the same bitrate x duration estimate the
            // extension's own manifest parser uses, so the two discovery
            // paths agree instead of one showing a size and the other a
            // blank. Both are estimates either way — `filesize_approx` is
            // itself derived from the bitrate.
            let filesize_bytes = f
                .filesize
                .or(f.filesize_approx)
                .or_else(|| estimate_bytes(bandwidth, duration_secs));
            Some(MediaFormat {
                url,
                height: f.height,
                resolution: derive_resolution(f),
                bandwidth,
                filesize_bytes,
                duration_secs,
                fps: f.fps.filter(|v| *v > 0.0).map(|v| v.round() as u32),
                vcodec: f.vcodec.clone().filter(|c| c != "none"),
                acodec: f.acodec.clone().filter(|c| c != "none"),
            })
        })
        .collect();

    out.sort_by(|a, b| {
        b.height
            .unwrap_or(0)
            .cmp(&a.height.unwrap_or(0))
            .then(b.bandwidth.unwrap_or(0).cmp(&a.bandwidth.unwrap_or(0)))
    });
    out
}

/// Transfer size implied by a constant `bandwidth` (bits/sec) held for
/// `duration_secs`. Used only when yt-dlp reported no size of its own.
/// The result is an estimate: a real encode's bitrate varies around its
/// advertised value, so the caller must present it as approximate.
///
/// Saturating arithmetic, not a plain multiply: `bandwidth` comes from
/// yt-dlp's parse of a manifest the site controls, so a malformed or
/// hostile value must clamp rather than overflow.
fn estimate_bytes(bandwidth: Option<u64>, duration_secs: Option<u32>) -> Option<u64> {
    let bits_per_sec = bandwidth.filter(|b| *b > 0)?;
    let secs = duration_secs.filter(|d| *d > 0)?;
    Some(bits_per_sec.saturating_mul(u64::from(secs)) / 8)
}

fn is_video_only(f: &RawFormat) -> bool {
    let v = f.vcodec.as_deref().unwrap_or("none");
    let a = f.acodec.as_deref().unwrap_or("none");
    v != "none" && a == "none"
}

fn is_audio_only(f: &RawFormat) -> bool {
    let v = f.vcodec.as_deref().unwrap_or("none");
    let a = f.acodec.as_deref().unwrap_or("none");
    v == "none" && a != "none"
}

fn is_combined(f: &RawFormat) -> bool {
    let v = f.vcodec.as_deref().unwrap_or("none");
    let a = f.acodec.as_deref().unwrap_or("none");
    v != "none" && a != "none"
}

fn video_score(f: &RawFormat) -> (u32, u64, u64) {
    // Sort key: (height, tbr_kbps_as_u64, filesize). Missing fields fall
    // back to zero so a format with metadata always beats one without.
    let h = f.height.unwrap_or(0);
    let tbr = f.tbr.unwrap_or(0.0).max(0.0) as u64;
    let size = f.filesize.or(f.filesize_approx).unwrap_or(0);
    (h, tbr, size)
}

fn audio_score(f: &RawFormat) -> (u64, u64) {
    let tbr = f.tbr.unwrap_or(0.0).max(0.0) as u64;
    let size = f.filesize.or(f.filesize_approx).unwrap_or(0);
    (tbr, size)
}

/// Pick a format selector for "best video + audio".
///
/// Scoring is explicit rather than positional because yt-dlp's `formats`
/// array isn't reliably sorted — some extractors emit best-first, others
/// worst-first, and the order can change between yt-dlp releases. Picking
/// by `iter().rev().next()` therefore gave 144p as "best" on YouTube.
///
/// Preference order:
/// 1. Separate `bestvideo + bestaudio` if the best video-only beats the
///    best combined in resolution (or if no combined exists).
/// 2. Best combined single-format otherwise.
fn pick_best_video_audio(formats: &[RawFormat]) -> Option<String> {
    let best_video = formats
        .iter()
        .filter(|f| is_video_only(f))
        .max_by_key(|f| video_score(f));
    let best_audio = formats
        .iter()
        .filter(|f| is_audio_only(f))
        .max_by_key(|f| audio_score(f));
    let best_combined = formats
        .iter()
        .filter(|f| is_combined(f))
        .max_by_key(|f| video_score(f));

    match (best_video, best_audio, best_combined) {
        (Some(v), Some(a), Some(c)) => {
            // Prefer separate streams only when they actually beat the
            // combined option's resolution; otherwise mux is wasted work.
            if v.height.unwrap_or(0) > c.height.unwrap_or(0) {
                Some(format!(
                    "{}+{}",
                    v.format_id.clone().unwrap_or_default(),
                    a.format_id.clone().unwrap_or_default()
                ))
            } else {
                c.format_id.clone()
            }
        }
        (Some(v), Some(a), None) => Some(format!(
            "{}+{}",
            v.format_id.clone().unwrap_or_default(),
            a.format_id.clone().unwrap_or_default()
        )),
        (_, _, Some(c)) => c.format_id.clone(),
        (Some(v), None, None) => v.format_id.clone(),
        _ => None,
    }
}

fn pick_best_audio_only(formats: &[RawFormat]) -> Option<String> {
    formats
        .iter()
        .filter(|f| is_audio_only(f))
        .max_by_key(|f| audio_score(f))
        .and_then(|f| f.format_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_like_fixture_lifts_cleanly() {
        // A minimal YouTube-shaped payload: one video-only, one audio-only,
        // one combined fallback.
        let json = r#"{
            "webpage_url": "https://www.youtube.com/watch?v=abc",
            "extractor": "youtube",
            "title": "Test Video",
            "uploader": "Channel Name",
            "duration": 215.0,
            "thumbnail": "https://example.com/t.jpg",
            "age_limit": 0,
            "is_live": false,
            "formats": [
                {"format_id": "18", "ext": "mp4", "width": 640, "height": 360,
                 "vcodec": "avc1.42001E", "acodec": "mp4a.40.2", "tbr": 568.0,
                 "filesize": 5000000, "format_note": "360p"},
                {"format_id": "137", "ext": "mp4", "width": 1920, "height": 1080,
                 "vcodec": "avc1.640028", "acodec": "none", "tbr": 4500.0,
                 "filesize": 50000000, "format_note": "1080p"},
                {"format_id": "140", "ext": "m4a",
                 "vcodec": "none", "acodec": "mp4a.40.2", "tbr": 128.0,
                 "filesize": 3500000, "format_note": "audio"}
            ]
        }"#;

        let raw: RawInfo = serde_json::from_str(json).unwrap();
        let probe = raw.into_probe("https://www.youtube.com/watch?v=abc");

        assert_eq!(probe.extractor, "youtube");
        assert_eq!(probe.title, "Test Video");
        assert_eq!(probe.uploader.as_deref(), Some("Channel Name"));
        assert_eq!(probe.duration_secs, Some(215));
        assert_eq!(probe.formats.len(), 3);

        // Format 137 (1080p video-only) beats format 18 (360p combined),
        // so the recommendation is the separate-stream mux.
        assert_eq!(probe.recommended_video_audio.as_deref(), Some("137+140"));
        assert_eq!(probe.recommended_audio_only.as_deref(), Some("140"));
    }

    #[test]
    fn missing_optional_fields_are_tolerated() {
        // Twitter / TikTok often ship a single combined format and no
        // uploader / duration on the top-level dict.
        let json = r#"{
            "extractor": "twitter",
            "title": "tweet video",
            "formats": [
                {"format_id": "hls-720", "ext": "mp4",
                 "vcodec": "avc1.640028", "acodec": "mp4a.40.2",
                 "resolution": "1280x720"}
            ]
        }"#;

        let raw: RawInfo = serde_json::from_str(json).unwrap();
        let probe = raw.into_probe("https://twitter.com/x/status/1");
        assert_eq!(probe.url, "https://twitter.com/x/status/1");
        assert!(probe.uploader.is_none());
        assert!(probe.duration_secs.is_none());
        assert_eq!(probe.recommended_video_audio.as_deref(), Some("hls-720"));
        assert!(probe.recommended_audio_only.is_none());
    }

    #[test]
    fn separate_streams_compose_a_plus_selector() {
        let json = r#"{
            "extractor": "vimeo",
            "title": "v",
            "formats": [
                {"format_id": "v720", "ext": "mp4", "width": 1280, "height": 720,
                 "vcodec": "h264", "acodec": "none"},
                {"format_id": "a128", "ext": "m4a",
                 "vcodec": "none", "acodec": "aac"}
            ]
        }"#;
        let raw: RawInfo = serde_json::from_str(json).unwrap();
        let probe = raw.into_probe("https://vimeo.com/123");
        assert_eq!(probe.recommended_video_audio.as_deref(), Some("v720+a128"));
        assert_eq!(probe.recommended_audio_only.as_deref(), Some("a128"));
    }

    #[test]
    fn unknown_fields_in_payload_do_not_break() {
        // yt-dlp regularly adds new top-level fields; serde must ignore.
        let json = r#"{
            "extractor": "tiktok",
            "title": "t",
            "_some_future_field": {"nested": [1, 2, 3]},
            "formats": []
        }"#;
        let raw: RawInfo = serde_json::from_str(json).unwrap();
        let probe = raw.into_probe("https://tiktok.com/@x/video/1");
        assert_eq!(probe.extractor, "tiktok");
        assert!(probe.formats.is_empty());
        assert!(probe.recommended_video_audio.is_none());
    }

    // --- into_media_formats / media_formats_from_raw -------------------

    #[test]
    fn media_formats_drops_entries_with_no_url() {
        let json = r#"{
            "extractor": "generic",
            "title": "t",
            "formats": [
                {"format_id": "no-url", "vcodec": "avc1", "acodec": "aac", "height": 720},
                {"format_id": "has-url", "vcodec": "avc1", "acodec": "aac", "height": 480,
                 "url": "https://cdn.example.com/480.m3u8"}
            ]
        }"#;
        let raw: RawInfo = serde_json::from_str(json).unwrap();
        let formats = raw.into_media_formats();
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].url, "https://cdn.example.com/480.m3u8");
    }

    #[test]
    fn media_formats_drops_audio_only() {
        let json = r#"{
            "extractor": "generic",
            "title": "t",
            "formats": [
                {"format_id": "video", "vcodec": "avc1", "acodec": "aac", "height": 720,
                 "url": "https://cdn.example.com/720.m3u8"},
                {"format_id": "audio", "vcodec": "none", "acodec": "aac",
                 "url": "https://cdn.example.com/audio.m3u8"}
            ]
        }"#;
        let raw: RawInfo = serde_json::from_str(json).unwrap();
        let formats = raw.into_media_formats();
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].url, "https://cdn.example.com/720.m3u8");
    }

    #[test]
    fn media_formats_dedupes_by_url_keeping_first_seen() {
        let json = r#"{
            "extractor": "generic",
            "title": "t",
            "formats": [
                {"format_id": "first", "vcodec": "avc1", "acodec": "aac", "height": 1080,
                 "url": "https://cdn.example.com/same.m3u8"},
                {"format_id": "second", "vcodec": "avc1", "acodec": "aac", "height": 2160,
                 "url": "https://cdn.example.com/same.m3u8"}
            ]
        }"#;
        let raw: RawInfo = serde_json::from_str(json).unwrap();
        let formats = raw.into_media_formats();
        assert_eq!(formats.len(), 1);
        // First-seen (1080p) wins, not the later 2160p duplicate.
        assert_eq!(formats[0].height, Some(1080));
    }

    #[test]
    fn media_formats_sorts_by_height_then_bandwidth_descending() {
        let json = r#"{
            "extractor": "generic",
            "title": "t",
            "formats": [
                {"format_id": "low", "vcodec": "avc1", "acodec": "aac", "height": 360, "tbr": 500.0,
                 "url": "https://cdn.example.com/360.m3u8"},
                {"format_id": "high-bitrate", "vcodec": "avc1", "acodec": "aac", "height": 1080,
                 "tbr": 6000.0, "url": "https://cdn.example.com/1080-high.m3u8"},
                {"format_id": "low-bitrate", "vcodec": "avc1", "acodec": "aac", "height": 1080,
                 "tbr": 3000.0, "url": "https://cdn.example.com/1080-low.m3u8"}
            ]
        }"#;
        let raw: RawInfo = serde_json::from_str(json).unwrap();
        let formats = raw.into_media_formats();
        let urls: Vec<&str> = formats.iter().map(|f| f.url.as_str()).collect();
        assert_eq!(
            urls,
            vec![
                "https://cdn.example.com/1080-high.m3u8",
                "https://cdn.example.com/1080-low.m3u8",
                "https://cdn.example.com/360.m3u8",
            ]
        );
        // tbr (kbps) is converted to bandwidth (bits/sec): 6000 kbps -> 6_000_000.
        assert_eq!(formats[0].bandwidth, Some(6_000_000));
    }

    #[test]
    fn media_formats_carry_size_duration_fps_and_codecs() {
        let json = r#"{
            "extractor": "generic",
            "title": "t",
            "duration": 3600.0,
            "formats": [
                {"format_id": "v", "vcodec": "avc1.640028", "acodec": "mp4a.40.2",
                 "height": 1080, "fps": 29.97, "tbr": 6000.0, "filesize": 4500000000,
                 "url": "https://cdn.example.com/1080.m3u8"}
            ]
        }"#;
        let raw: RawInfo = serde_json::from_str(json).unwrap();
        let formats = raw.into_media_formats();
        assert_eq!(formats[0].filesize_bytes, Some(4_500_000_000));
        assert_eq!(formats[0].duration_secs, Some(3600));
        assert_eq!(formats[0].fps, Some(30));
        assert_eq!(formats[0].vcodec.as_deref(), Some("avc1.640028"));
        assert_eq!(formats[0].acodec.as_deref(), Some("mp4a.40.2"));
    }

    #[test]
    fn media_formats_estimate_size_when_ytdlp_reports_none() {
        // The HLS norm: yt-dlp knows the bitrate and the duration but not
        // the size. 6000 kbps for 3600 s is 6_000_000 * 3600 / 8 bytes.
        let json = r#"{
            "extractor": "generic",
            "title": "t",
            "duration": 3600.0,
            "formats": [
                {"format_id": "v", "vcodec": "avc1", "acodec": "aac", "height": 1080,
                 "tbr": 6000.0, "url": "https://cdn.example.com/1080.m3u8"}
            ]
        }"#;
        let raw: RawInfo = serde_json::from_str(json).unwrap();
        let formats = raw.into_media_formats();
        assert_eq!(formats[0].filesize_bytes, Some(2_700_000_000));
    }

    #[test]
    fn media_formats_leave_size_unknown_without_a_duration() {
        // A live stream, or an extractor that reports no duration: an
        // estimate is impossible, so the field stays empty rather than
        // showing a wrong number.
        let json = r#"{
            "extractor": "generic",
            "title": "t",
            "formats": [
                {"format_id": "v", "vcodec": "avc1", "acodec": "aac", "height": 1080,
                 "tbr": 6000.0, "url": "https://cdn.example.com/1080.m3u8"}
            ]
        }"#;
        let raw: RawInfo = serde_json::from_str(json).unwrap();
        let formats = raw.into_media_formats();
        assert_eq!(formats[0].filesize_bytes, None);
        assert_eq!(formats[0].duration_secs, None);
    }

    #[test]
    fn media_formats_drop_the_none_codec_sentinel() {
        // yt-dlp writes the string "none", not null, for a missing track.
        // Passing that through would print "none" as a codec name.
        let json = r#"{
            "extractor": "generic",
            "title": "t",
            "formats": [
                {"format_id": "v", "vcodec": "avc1", "acodec": "none", "height": 720,
                 "url": "https://cdn.example.com/720.m3u8"}
            ]
        }"#;
        let raw: RawInfo = serde_json::from_str(json).unwrap();
        let formats = raw.into_media_formats();
        assert_eq!(formats[0].vcodec.as_deref(), Some("avc1"));
        assert_eq!(formats[0].acodec, None);
    }

    #[test]
    fn media_formats_no_label_field_and_matches_resolution_derivation() {
        // Resolution derivation reuses the same helper `Format::from_raw`
        // uses, so a format with no explicit `resolution` string still
        // gets a sensible one here.
        let json = r#"{
            "extractor": "generic",
            "title": "t",
            "formats": [
                {"format_id": "v", "vcodec": "avc1", "acodec": "aac", "width": 1280, "height": 720,
                 "url": "https://cdn.example.com/720.m3u8"}
            ]
        }"#;
        let raw: RawInfo = serde_json::from_str(json).unwrap();
        let formats = raw.into_media_formats();
        assert_eq!(formats[0].resolution.as_deref(), Some("1280x720"));
    }
}
