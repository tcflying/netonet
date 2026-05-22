//! End-to-end test of the netonet transport stack: a self-hosted relay, two
//! Iroh endpoints discovering each other through it, and an IP packet round-trip
//! over the framed QUIC stream protocol used by the engine.

use std::time::Duration;

use iroh::{EndpointAddr, SecretKey};
use iroh_relay::server::{RelayConfig, Server, ServerConfig};
use netonet_core::engine::ALPN;
use netonet_core::frame::{read_frame, write_frame};
use netonet_core::build_endpoint;

#[tokio::test]
async fn relay_quic_frame_roundtrip() {
    // 1. Start a self-hosted relay on an ephemeral local port.
    let mut cfg = ServerConfig::default();
    cfg.relay = Some(RelayConfig::new(([127, 0, 0, 1], 0)));
    let relay = Server::spawn(cfg).await.expect("spawn relay");
    let relay_addr = relay.http_addr().expect("relay http addr");
    let relay_url: iroh::RelayUrl = format!("http://{relay_addr}").parse().unwrap();

    // 2. Two endpoints, both pinned to our relay.
    let acceptor = build_endpoint(SecretKey::generate(), Some(relay_url.clone()))
        .await
        .unwrap();
    let dialer = build_endpoint(SecretKey::generate(), Some(relay_url.clone()))
        .await
        .unwrap();
    acceptor.online().await;
    dialer.online().await;

    let acceptor_id = acceptor.id();

    // 3. Acceptor echoes one framed packet back.
    let server = tokio::spawn(async move {
        let incoming = acceptor.accept().await.expect("incoming");
        let conn = incoming.await.expect("connection");
        let (mut send, mut recv) = conn.accept_bi().await.expect("accept_bi");
        let mut buf = vec![0u8; 2048];
        let n = read_frame(&mut recv, &mut buf)
            .await
            .expect("read")
            .expect("frame");
        write_frame(&mut send, &buf[..n]).await.expect("echo");
        send.finish().ok();
        conn.closed().await;
    });

    // 4. Dialer connects through the relay and round-trips a fake IP packet.
    let addr = EndpointAddr::new(acceptor_id).with_relay_url(relay_url);
    let conn = dialer.connect(addr, ALPN).await.expect("connect");
    let (mut send, mut recv) = conn.open_bi().await.expect("open_bi");

    let packet = b"netonet test packet";
    write_frame(&mut send, packet).await.expect("write");

    let mut buf = vec![0u8; 2048];
    let n = read_frame(&mut recv, &mut buf)
        .await
        .expect("read")
        .expect("frame");
    assert_eq!(&buf[..n], packet);

    conn.close(0u32.into(), b"done");
    let _ = tokio::time::timeout(Duration::from_secs(5), server).await;
    let _ = relay.shutdown().await;
}
