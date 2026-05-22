use std::net::SocketAddr;

use anyhow::{Context, Result};
use clap::Parser;
use iroh_relay::server::{RelayConfig, Server, ServerConfig};
use tracing::info;

/// Self-hosted Iroh relay for netonet.
///
/// Serves the relay protocol over plain HTTP. The relay only forwards Iroh's
/// already end-to-end encrypted traffic, so the relay hop never sees packet
/// payloads. For a public deployment, either run this behind a TLS-terminating
/// reverse proxy or use the upstream `iroh-relay` binary's built-in TLS/ACME.
#[derive(Parser, Debug)]
#[command(name = "netonet-relay", version, about)]
struct Cli {
    /// Address to bind the relay HTTP server on.
    #[arg(short, long, default_value = "0.0.0.0:3340")]
    bind: SocketAddr,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "netonet_relay=info,iroh_relay=info".into()),
        )
        .init();

    let cli = Cli::parse();

    // `ServerConfig` is `#[non_exhaustive]`, so build via Default + field assignment.
    let mut config = ServerConfig::default();
    config.relay = Some(RelayConfig::new(cli.bind));

    let mut server = Server::spawn(config)
        .await
        .context("spawning relay server")?;

    if let Some(addr) = server.http_addr() {
        info!(%addr, "netonet relay listening (configure peers with relay_url = \"http://<host>:{}\")", addr.port());
    }

    tokio::select! {
        res = server.join() => {
            res.context("relay task panicked")?.context("relay server error")?;
        }
        _ = tokio::signal::ctrl_c() => {
            info!("shutting down");
            server.shutdown().await.context("relay shutdown")?;
        }
    }

    Ok(())
}
