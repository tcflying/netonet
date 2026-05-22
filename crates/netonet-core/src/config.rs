use std::net::Ipv4Addr;
use std::path::Path;

use anyhow::{Context, Result};
use iroh::{EndpointId, RelayUrl, SecretKey};
use serde::{Deserialize, Serialize};

/// On-disk configuration for a netonet node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// The node's secret key as a 64-char hex string.
    ///
    /// If absent, a fresh key is generated on startup. Persist the generated
    /// value (printed at startup) so the node keeps a stable identity.
    #[serde(default)]
    pub secret_key: Option<String>,

    /// URL of the self-hosted relay server, e.g. `http://relay.example.com:3340`.
    ///
    /// When set, the node uses ONLY this relay (no public n0 relays). Peers must
    /// share the same relay to discover each other through it.
    #[serde(default)]
    pub relay_url: Option<String>,

    /// Virtual interface configuration.
    pub interface: InterfaceConfig,

    /// Static peer table mapping a peer's public key to its virtual IP.
    #[serde(default)]
    pub peers: Vec<PeerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterfaceConfig {
    /// Optional explicit interface name (e.g. `netonet0`). Ignored on platforms
    /// that assign names automatically.
    #[serde(default)]
    pub name: Option<String>,
    /// This node's virtual IPv4 address inside the overlay network.
    pub ip: Ipv4Addr,
    /// Network prefix length (e.g. 24 for a /24).
    #[serde(default = "default_prefix")]
    pub prefix: u8,
    /// MTU for the TUN device. Keep below the QUIC path MTU to avoid fragmentation.
    #[serde(default = "default_mtu")]
    pub mtu: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerConfig {
    /// The peer's public key (z-base-32 encoded), used to dial it via Iroh.
    pub endpoint_id: String,
    /// The virtual IPv4 address assigned to this peer in the overlay.
    pub ip: Ipv4Addr,
}

fn default_prefix() -> u8 {
    24
}

fn default_mtu() -> u16 {
    1380
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config file {}", path.display()))?;
        let config: Config = toml::from_str(&text).context("parsing config TOML")?;
        Ok(config)
    }

    /// Resolves the configured secret key, or generates one if absent.
    pub fn secret_key(&self) -> Result<SecretKey> {
        match &self.secret_key {
            Some(hex_str) => {
                let bytes = hex::decode(hex_str.trim()).context("decoding secret_key hex")?;
                let arr: [u8; 32] = bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("secret_key must be exactly 32 bytes (64 hex chars)"))?;
                Ok(SecretKey::from_bytes(&arr))
            }
            None => Ok(SecretKey::generate()),
        }
    }

    /// Parses the configured relay URL, if any.
    pub fn relay(&self) -> Result<Option<RelayUrl>> {
        match &self.relay_url {
            Some(url) => Ok(Some(url.parse().context("parsing relay_url")?)),
            None => Ok(None),
        }
    }

    /// Builds the routing table: virtual IP -> peer endpoint id.
    pub fn routes(&self) -> Result<Vec<(Ipv4Addr, EndpointId)>> {
        let mut routes = Vec::with_capacity(self.peers.len());
        for peer in &self.peers {
            let id: EndpointId = peer
                .endpoint_id
                .trim()
                .parse()
                .with_context(|| format!("parsing peer endpoint_id {}", peer.endpoint_id))?;
            routes.push((peer.ip, id));
        }
        Ok(routes)
    }
}
