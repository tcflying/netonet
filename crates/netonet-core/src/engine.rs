use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use bytes::Bytes;
use dashmap::DashMap;
use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointAddr, EndpointId};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use tun_rs::AsyncDevice;

use crate::frame::{MAX_FRAME, read_frame, write_frame};

/// ALPN identifying the netonet overlay protocol.
pub const ALPN: &[u8] = b"netonet/vpn/0";

/// Bound on the per-peer outbound queue. Excess packets are dropped (VPN
/// traffic tolerates loss; unbounded buffering would only add latency).
const PEER_QUEUE: usize = 1024;

/// Shared engine state.
struct Engine {
    endpoint: Endpoint,
    device: Arc<AsyncDevice>,
    /// virtual IPv4 -> peer endpoint id
    routes: HashMap<Ipv4Addr, EndpointId>,
    /// peer endpoint id -> outbound packet queue (present while connected)
    peers: DashMap<EndpointId, mpsc::Sender<Bytes>>,
}

/// Runs the netonet engine until the process is stopped.
///
/// `routes` maps each peer's virtual IP to its public key. The engine:
/// - reads IP packets from the TUN device and forwards them to the peer that
///   owns the destination IP, over a per-peer QUIC bidirectional stream;
/// - accepts inbound connections and writes received packets back to the TUN.
///
/// To avoid two nodes opening duplicate connections to each other, the node
/// with the lexicographically smaller endpoint id dials; the other only accepts.
pub async fn run(
    endpoint: Endpoint,
    device: Arc<AsyncDevice>,
    routes: Vec<(Ipv4Addr, EndpointId)>,
    relay: Option<iroh::RelayUrl>,
) -> Result<()> {
    let me = endpoint.id();
    let route_map: HashMap<Ipv4Addr, EndpointId> = routes.iter().copied().collect();

    let engine = Arc::new(Engine {
        endpoint: endpoint.clone(),
        device,
        routes: route_map,
        peers: DashMap::new(),
    });

    // Accept inbound connections.
    tokio::spawn(accept_loop(engine.clone()));

    // Dial peers for which we are the designated dialer.
    for (_ip, peer_id) in &routes {
        if me.as_bytes() < peer_id.as_bytes() {
            tokio::spawn(dial_loop(engine.clone(), *peer_id, relay.clone()));
        } else {
            debug!(%peer_id, "waiting for inbound connection from peer (peer is dialer)");
        }
    }

    // Pump packets from the TUN device out to the right peer. This runs on the
    // current task for the lifetime of the process.
    tun_read_loop(engine).await
}

/// Reads packets from the TUN device and routes them to the owning peer.
async fn tun_read_loop(engine: Arc<Engine>) -> Result<()> {
    let mut buf = vec![0u8; MAX_FRAME];
    loop {
        let n = engine
            .device
            .recv(&mut buf)
            .await
            .context("reading from TUN device")?;
        if n == 0 {
            continue;
        }
        let Some(dst) = ipv4_destination(&buf[..n]) else {
            continue; // not IPv4 (e.g. IPv6/ARP); drop for now
        };
        let Some(peer_id) = engine.routes.get(&dst).copied() else {
            debug!(%dst, "no route for destination, dropping packet");
            continue;
        };
        if let Some(tx) = engine.peers.get(&peer_id) {
            // Drop if the peer's queue is full rather than blocking the reader.
            if tx.try_send(Bytes::copy_from_slice(&buf[..n])).is_err() {
                debug!(%peer_id, "peer queue full or closed, dropping packet");
            }
        } else {
            debug!(%peer_id, "peer not connected yet, dropping packet");
        }
    }
}

/// Continuously accepts inbound connections.
async fn accept_loop(engine: Arc<Engine>) {
    while let Some(incoming) = engine.endpoint.accept().await {
        let engine = engine.clone();
        tokio::spawn(async move {
            match incoming.await {
                Ok(conn) => {
                    let peer_id = conn.remote_id();
                    info!(%peer_id, "accepted inbound connection");
                    if let Err(err) = serve_connection(engine, conn, false).await {
                        warn!(%peer_id, error = %err, "inbound connection ended");
                    }
                }
                Err(err) => warn!(error = %err, "failed to accept connection"),
            }
        });
    }
}

/// Maintains an outbound connection to `peer_id`, reconnecting on failure.
async fn dial_loop(engine: Arc<Engine>, peer_id: EndpointId, relay: Option<iroh::RelayUrl>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        // Include the relay in the address so the peer is reachable through our
        // self-hosted relay even without DNS discovery.
        let mut addr = EndpointAddr::new(peer_id);
        if let Some(relay) = &relay {
            addr = addr.with_relay_url(relay.clone());
        }
        match engine.endpoint.connect(addr, ALPN).await {
            Ok(conn) => {
                info!(%peer_id, "dialed peer");
                backoff = Duration::from_secs(1);
                if let Err(err) = serve_connection(engine.clone(), conn, true).await {
                    warn!(%peer_id, error = %err, "outbound connection ended");
                }
            }
            Err(err) => {
                warn!(%peer_id, error = %err, ?backoff, "dial failed, retrying");
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

/// Drives a single connection: establishes the bi stream, registers the peer's
/// outbound queue, and pumps packets in both directions until it closes.
async fn serve_connection(engine: Arc<Engine>, conn: Connection, dialer: bool) -> Result<()> {
    let peer_id = conn.remote_id();

    // The dialer opens the stream; the acceptor waits for it. This keeps a
    // single, agreed-upon stream per connection.
    let (mut send, mut recv) = if dialer {
        conn.open_bi().await.context("opening bi stream")?
    } else {
        conn.accept_bi().await.context("accepting bi stream")?
    };

    let (tx, mut rx) = mpsc::channel::<Bytes>(PEER_QUEUE);
    engine.peers.insert(peer_id, tx);

    // Outbound: queue -> QUIC stream.
    let writer = tokio::spawn(async move {
        while let Some(pkt) = rx.recv().await {
            if let Err(err) = write_frame(&mut send, &pkt).await {
                debug!(error = %err, "write_frame failed");
                break;
            }
        }
        let _ = send.finish();
    });

    // Inbound: QUIC stream -> TUN device.
    let device = engine.device.clone();
    let reader = async move {
        let mut buf = vec![0u8; MAX_FRAME];
        loop {
            match read_frame(&mut recv, &mut buf).await {
                Ok(Some(n)) => {
                    if let Err(err) = device.send(&buf[..n]).await {
                        warn!(error = %err, "writing to TUN failed");
                        break;
                    }
                }
                Ok(None) => break, // clean EOF
                Err(err) => {
                    debug!(error = %err, "read_frame failed");
                    break;
                }
            }
        }
    };

    reader.await;
    engine.peers.remove(&peer_id);
    writer.abort();
    Ok(())
}

/// Extracts the IPv4 destination address from a raw IP packet, if it is IPv4.
fn ipv4_destination(packet: &[u8]) -> Option<Ipv4Addr> {
    // First nibble is the IP version; IPv4 destination lives at bytes 16..20.
    if packet.len() < 20 || (packet[0] >> 4) != 4 {
        return None;
    }
    Some(Ipv4Addr::new(
        packet[16], packet[17], packet[18], packet[19],
    ))
}
