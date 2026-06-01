//! Splitting a contiguous byte range across N download workers.

/// One half-open byte range `[start, end)` to be fetched by a single worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Segment {
    pub index: usize,
    pub start: u64,
    pub end: u64,
}

impl Segment {
    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// HTTP `Range` header value (inclusive on both ends).
    pub fn range_header(&self) -> String {
        debug_assert!(self.end > self.start);
        format!("bytes={}-{}", self.start, self.end - 1)
    }
}

/// Split `total` bytes into at most `count` segments. Any remainder is
/// distributed one byte at a time across the first segments so all
/// segments are within 1 byte of each other.
///
/// `count` of zero is normalized to one. If `total` is zero, returns a
/// single empty segment.
pub fn split(total: u64, count: usize) -> Vec<Segment> {
    let count = count.max(1);
    if total == 0 {
        return vec![Segment {
            index: 0,
            start: 0,
            end: 0,
        }];
    }
    // Don't create more segments than there are bytes.
    let count = count.min(total as usize);
    let base = total / count as u64;
    let remainder = (total % count as u64) as usize;

    let mut segments = Vec::with_capacity(count);
    let mut start = 0u64;
    for i in 0..count {
        let extra = if i < remainder { 1 } else { 0 };
        let end = start + base + extra;
        segments.push(Segment {
            index: i,
            start,
            end,
        });
        start = end;
    }
    debug_assert_eq!(start, total);
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_even() {
        let segs = split(1000, 4);
        assert_eq!(segs.len(), 4);
        assert!(segs.iter().all(|s| s.len() == 250));
        assert_eq!(segs.first().unwrap().start, 0);
        assert_eq!(segs.last().unwrap().end, 1000);
    }

    #[test]
    fn split_with_remainder() {
        // 1003 / 4 = 250 r 3; first three segments get +1.
        let segs = split(1003, 4);
        assert_eq!(segs.len(), 4);
        assert_eq!(segs[0].len(), 251);
        assert_eq!(segs[1].len(), 251);
        assert_eq!(segs[2].len(), 251);
        assert_eq!(segs[3].len(), 250);
        assert_eq!(segs.last().unwrap().end, 1003);
    }

    #[test]
    fn split_contiguous_and_disjoint() {
        let segs = split(9999, 8);
        for pair in segs.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
        }
        assert_eq!(segs.first().unwrap().start, 0);
        assert_eq!(segs.last().unwrap().end, 9999);
        let sum: u64 = segs.iter().map(|s| s.len()).sum();
        assert_eq!(sum, 9999);
    }

    #[test]
    fn split_zero_total() {
        let segs = split(0, 4);
        assert_eq!(segs.len(), 1);
        assert!(segs[0].is_empty());
    }

    #[test]
    fn split_fewer_bytes_than_segments() {
        let segs = split(3, 8);
        assert_eq!(segs.len(), 3);
        assert!(segs.iter().all(|s| s.len() == 1));
    }

    #[test]
    fn split_zero_count_treated_as_one() {
        let segs = split(100, 0);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].len(), 100);
    }

    #[test]
    fn range_header_inclusive() {
        let s = Segment {
            index: 0,
            start: 0,
            end: 100,
        };
        assert_eq!(s.range_header(), "bytes=0-99");
    }
}
