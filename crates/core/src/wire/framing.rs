//! Length-prefixed framing shared by the native messaging host
//! (stdin/stdout) and the in-app named-pipe server.
//!
//! Layout: a 4-byte little-endian unsigned length, followed by exactly
//! that many UTF-8 bytes of JSON. Both sides reject any frame larger
//! than [`MAX_FRAME_BYTES`].
//!
//! Chrome's Native Messaging spec uses the same shape on stdio, so the
//! native host loop is a straight pass-through into the pipe. Reusing
//! the helpers on both transports keeps the cross-process contract in
//! one place.

use std::io;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Defensive cap on a single frame. 16 MB is well over the largest
/// `Outbound::Status` payload we expect (a handful of `StatusEntry`
/// rows ~= a few kB) and matches Chrome's per-message native messaging
/// hard cap (1 MB extension → host, 64 KB host → extension — we apply
/// a generous shared ceiling so neither direction stalls on a
/// surprising-but-legal payload).
pub const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;

/// Read a single length-prefixed frame from `reader`.
///
/// Returns `Ok(None)` on clean EOF (peer closed before any bytes of a
/// new frame arrived) so callers can distinguish "peer hung up cleanly"
/// from a partial frame (an `Err(UnexpectedEof)` mid-frame).
///
/// Frames that declare a length above [`MAX_FRAME_BYTES`] are rejected
/// without reading the payload — a hostile peer can't force us to
/// allocate gigabytes.
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    // Read the 4-byte length prefix one chunk at a time. A zero-byte
    // read *before* any prefix byte arrived is the only "clean EOF"
    // we accept — a partial prefix is a framing error.
    let mut len_buf = [0u8; 4];
    let mut filled = 0usize;
    while filled < len_buf.len() {
        let n = reader.read(&mut len_buf[filled..]).await?;
        if n == 0 {
            if filled == 0 {
                return Ok(None);
            }
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "partial length prefix",
            ));
        }
        filled += n;
    }
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame too large: {len} > {MAX_FRAME_BYTES}"),
        ));
    }
    let mut buf = vec![0u8; len as usize];
    reader.read_exact(&mut buf).await?;
    Ok(Some(buf))
}

/// Write a single length-prefixed frame to `writer`, flushing after the
/// payload. Errors if `payload.len()` exceeds [`MAX_FRAME_BYTES`].
pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, payload: &[u8]) -> io::Result<()> {
    let len: u32 = u32::try_from(payload.len())
        .ok()
        .filter(|l| *l <= MAX_FRAME_BYTES)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("payload too large: {} > {MAX_FRAME_BYTES}", payload.len()),
            )
        })?;
    writer.write_all(&len.to_le_bytes()).await?;
    writer.write_all(payload).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn round_trip_small() {
        let (mut a, mut b) = duplex(64);
        let payload = b"{\"type\":\"ping\"}";
        write_frame(&mut a, payload).await.unwrap();
        drop(a);
        let back = read_frame(&mut b).await.unwrap().expect("frame");
        assert_eq!(back, payload);
        assert!(read_frame(&mut b).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn round_trip_large() {
        // 200 KB — well past the 64 K boundary, so we exercise the
        // 4-byte LE prefix on its high bytes.
        let (mut a, mut b) = duplex(1024 * 1024);
        let payload: Vec<u8> = (0..200_000).map(|i| (i % 251) as u8).collect();
        write_frame(&mut a, &payload).await.unwrap();
        drop(a);
        let back = read_frame(&mut b).await.unwrap().expect("frame");
        assert_eq!(back, payload);
    }

    #[tokio::test]
    async fn rejects_oversize_frame() {
        // Forge a length prefix above the cap; reader must reject
        // without touching the payload bytes.
        let (mut a, mut b) = duplex(16);
        let bogus_len = (MAX_FRAME_BYTES + 1).to_le_bytes();
        tokio::io::AsyncWriteExt::write_all(&mut a, &bogus_len)
            .await
            .unwrap();
        drop(a);
        let err = read_frame(&mut b).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn clean_eof_returns_none() {
        let (a, mut b) = duplex(4);
        drop(a);
        let r = read_frame(&mut b).await.unwrap();
        assert!(r.is_none());
    }

    #[tokio::test]
    async fn partial_prefix_is_error() {
        let (mut a, mut b) = duplex(4);
        // Write only 2 of 4 bytes of the length prefix, then close.
        tokio::io::AsyncWriteExt::write_all(&mut a, &[0x10, 0x00])
            .await
            .unwrap();
        drop(a);
        let err = read_frame(&mut b).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }
}
