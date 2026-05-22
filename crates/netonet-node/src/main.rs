use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use netonet_core::{Config, build_endpoint, run};
use tracing::info;
use tun_rs::DeviceBuilder;

/// netonet desktop node: bridges a TUN device to remote peers over Iroh.
#[derive(Parser, Debug)]
#[command(name = "netonet", version, about)]
struct Cli {
    /// Path to the TOML configuration file.
    #[arg(short, long, default_value = "netonet.toml")]
    config: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "netonet_core=info,netonet=info".into()),
        )
        .init();

    let cli = Cli::parse();
    let config = Config::load(&cli.config)?;

    let secret_key = config.secret_key()?;
    info!("node id: {}", secret_key.public());
    if config.secret_key.is_none() {
        info!(
            "no secret_key in config; generated an ephemeral one. To keep a stable \
             identity, set secret_key = \"{}\"",
            hex::encode(secret_key.to_bytes())
        );
    }

    let relay = config.relay()?;
    let routes = config.routes()?;

    // Build the TUN device.
    let mut builder = DeviceBuilder::new()
        .ipv4(config.interface.ip, config.interface.prefix, None)
        .mtu(config.interface.mtu);
    if let Some(name) = &config.interface.name {
        builder = builder.name(name.clone());
    }
    let device = builder
        .build_async()
        .context("creating TUN device (need CAP_NET_ADMIN / root?)")?;
    info!(ip = %config.interface.ip, "TUN device up");

    let endpoint = build_endpoint(secret_key, relay.clone()).await?;
    endpoint.online().await;
    info!("endpoint online, joining overlay");

    run(endpoint, Arc::new(device), routes, relay).await
}
