//! Round-trip the shared framing helpers across both transports the
//! native host will see in production: an in-memory `duplex` and an
//! actual Windows named pipe pair. Catches any drift between the
//! prefix layout and the cap.

use tokio::io::duplex;
use unduhin_core::wire::framing::{read_frame, write_frame};

#[tokio::test]
async fn round_trip_small_payload_in_duplex() {
    let (mut a, mut b) = duplex(128);
    let payload = br#"{"type":"ping"}"#;
    write_frame(&mut a, payload).await.unwrap();
    drop(a);
    let back = read_frame(&mut b).await.unwrap().expect("frame");
    assert_eq!(back, payload);
}

#[tokio::test]
async fn round_trip_large_payload_in_duplex() {
    // 200 KB exercises the high bytes of the 4-byte LE prefix.
    let (mut a, mut b) = duplex(1024 * 1024);
    let payload: Vec<u8> = (0..200_000u32).flat_map(|n| n.to_le_bytes()).collect();
    write_frame(&mut a, &payload).await.unwrap();
    drop(a);
    let back = read_frame(&mut b).await.unwrap().expect("frame");
    assert_eq!(back.len(), payload.len());
    assert_eq!(back, payload);
}

#[tokio::test]
async fn multiple_frames_back_to_back() {
    // Reader must consume each prefix independently — no buffer
    // bleed-over between frames.
    let (mut a, mut b) = duplex(4096);
    let frames: Vec<Vec<u8>> = (0..5).map(|i| vec![i as u8; 100 + i * 50]).collect();
    for f in &frames {
        write_frame(&mut a, f).await.unwrap();
    }
    drop(a);
    for expected in &frames {
        let got = read_frame(&mut b).await.unwrap().expect("frame");
        assert_eq!(&got, expected);
    }
    assert!(read_frame(&mut b).await.unwrap().is_none());
}

#[cfg(windows)]
#[tokio::test]
async fn round_trip_over_real_named_pipe() {
    use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};

    // Use a per-test name so concurrent runs don't collide.
    let name = format!(r"\\.\pipe\unduhin-framing-test-{}", std::process::id());
    let mut server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&name)
        .expect("create server");

    let name_for_client = name.clone();
    let client_task = tokio::spawn(async move {
        // The client side connects after the server is listening.
        for _ in 0..20 {
            if let Ok(mut client) = ClientOptions::new().open(&name_for_client) {
                let payload = b"hello over pipe";
                write_frame(&mut client, payload).await.unwrap();
                let echoed = read_frame(&mut client).await.unwrap().expect("echo");
                assert_eq!(echoed, payload);
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("client failed to connect");
    });

    server.connect().await.expect("server accept");
    let frame = read_frame(&mut server).await.unwrap().expect("frame");
    write_frame(&mut server, &frame).await.unwrap();
    client_task.await.unwrap();
}
