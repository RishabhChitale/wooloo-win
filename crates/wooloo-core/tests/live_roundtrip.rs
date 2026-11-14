//! Live round-trip against a THROWAWAY herdr server. Ignored unless WOOLOO_TEST_SESSION is set
//! (never point it at a real session): `WOOLOO_TEST_SESSION=wooloo-win-test cargo test -- --ignored`.
use wooloo_core::api::ApiClient;
use wooloo_core::machines::{Machine, MachineKind};
use wooloo_core::surface::{self, Decoder, Frame};
use wooloo_core::transport::{for_machine, Channel};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
#[ignore]
async fn roundtrip() {
    let Ok(session) = std::env::var("WOOLOO_TEST_SESSION") else { return };
    assert_ne!(session, "wooloo", "refusing to touch the real session");
    let m = Machine { id: "t".into(), label: "t".into(), kind: MachineKind::Local, ssh_target: None, session: Some(session), pipe_path: None };
    let t = for_machine(&m).unwrap();

    // A subscribed connection is event-only: snapshots need their own connection.
    let mut ev = ApiClient::new(t.connect(Channel::Api).await.expect("step connect events socket"));
    ev.subscribe().await.expect("step events.subscribe");
    let mut api = ApiClient::new(t.connect(Channel::Api).await.expect("step connect request socket"));
    let snap = api.snapshot().await.expect("step session.snapshot");
    assert!(!snap.workspaces.is_empty());

    let (mut rd, mut wr) = tokio::io::split(t.connect(Channel::Client).await.expect("step connect client socket"));
    wr.write_all(&surface::frame(&surface::hello(9, 18, 100, 30))).await.unwrap();
    let (mut dec, mut buf, mut chunk) = (Decoder::new(), Vec::new(), vec![0u8; 65536]);
    let mut full = false;
    for _ in 0..50 {
        let n = tokio::time::timeout(std::time::Duration::from_secs(10), rd.read(&mut chunk)).await.expect("step read frame: timeout").expect("step read frame: io");
        assert!(n > 0, "step read frame: server closed client socket");
        buf.extend_from_slice(&chunk[..n]);
        for f in surface::drain_frames(&mut buf).unwrap() {
            if matches!(dec.apply(&f).expect("step decode frame"), Frame::Full) { full = true; }
        }
        if full { break; }
    }
    assert!(full, "no full surface received");
}
