//! Core engine for netonet: an Iroh-based mesh VPN.
//!
//! The engine bridges a local TUN device to remote peers over Iroh QUIC
//! connections. IP packets read from the TUN are routed to the peer that owns
//! the destination virtual IP and sent over a per-peer bidirectional QUIC
//! stream; packets received from peers are written back to the TUN.
//!
//! Platform-specific crates are responsible for creating the [`tun_rs::AsyncDevice`]
//! (from a name + IP on desktop, or from a file descriptor on Android) and the
//! Iroh [`iroh::Endpoint`]; this crate is platform-agnostic.

pub mod config;
pub mod engine;
pub mod frame;

pub use config::{Config, InterfaceConfig, PeerConfig};
pub use engine::{ALPN, run};

use anyhow::Result;
use iroh::endpoint::presets;
use iroh::{Endpoint, RelayConfig, RelayMap, RelayMode, SecretKey};
use std::sync::Arc;

/// Builds an Iroh endpoint configured for netonet.
///
/// When `relay` is provided, the endpoint uses ONLY that relay (a self-hosted
/// server). Otherwise it falls back to the default n0 relays and DNS discovery.
pub async fn build_endpoint(
    secret_key: SecretKey,
    relay: Option<iroh::RelayUrl>,
) -> Result<Endpoint> {
    let endpoint = match relay {
        Some(url) => {
            // Self-hosted: pin to our relay only and skip n0's DNS/pkarr discovery
            // entirely. Peers are reached by explicit address (id + relay url), so
            // no external discovery service is involved.
            let config = RelayConfig::new(url.clone(), None);
            let map = RelayMap::empty();
            map.insert(url, Arc::new(config));
            Endpoint::builder(presets::Minimal)
                .secret_key(secret_key)
                .alpns(vec![ALPN.to_vec()])
                .relay_mode(RelayMode::Custom(map))
                .bind()
                .await?
        }
        None => {
            // No self-hosted relay: fall back to n0's defaults (public relays + DNS).
            Endpoint::builder(presets::N0)
                .secret_key(secret_key)
                .alpns(vec![ALPN.to_vec()])
                .bind()
                .await?
        }
    };
    Ok(endpoint)
}
