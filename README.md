# netonet

A self-hosted, cross-platform mesh VPN built on [Iroh](https://github.com/n0-computer/iroh).

netonet gives geographically separated machines the feeling of being on the
same home LAN. Each node exposes a virtual network interface (TUN); IP packets
are carried to the right peer over an Iroh QUIC connection, which transparently
NAT-hole-punches for a **direct** path and falls back to a relay only when
needed. You run your **own** relay, so no third-party infrastructure is involved.

> Built against `iroh = 1.0.0-rc.0`.

## How it works

```
   app traffic                                          app traffic
        │                                                    │
   ┌────▼────┐   read/write IP packets             ┌─────────▼────┐
   │  TUN    │◄───────────────┐         ┌──────────►│     TUN      │
   │ netonet0│                │         │           │   netonet0   │
   └─────────┘          ┌─────┴─────────┴─────┐     └──────────────┘
                        │   netonet engine    │
                        │  route by dst IP →  │
                        │  per-peer QUIC bi   │
                        │  stream (framed)    │
                        └─────────┬───────────┘
                                  │ Iroh: hole-punch direct,
                                  │ else via your relay
                        ┌─────────▼───────────┐
                        │   netonet-relay     │  (self-hosted)
                        └─────────────────────┘
```

- **Identity is a public key, not an IP.** You dial a peer by its Iroh
  `EndpointId` (its public key); Iroh figures out how to reach it.
- **Routing.** Each peer is statically assigned a virtual IP in the config. The
  engine parses the destination IP of every outbound packet and sends it over
  that peer's QUIC bidirectional stream (length-delimited framing).
- **End-to-end encrypted.** Iroh encrypts the QUIC connection; the relay only
  ever forwards opaque, encrypted bytes.

## Workspace layout

| Crate            | Kind        | Purpose                                                        |
| ---------------- | ----------- | ------------------------------------------------------------- |
| `netonet-core`   | lib         | Platform-agnostic engine: config, identity, framing, routing. |
| `netonet-node`   | bin         | Desktop node (Linux/macOS/Windows): builds a TUN + runs core. |
| `netonet-relay`  | bin         | Your self-hosted Iroh relay.                                  |
| `netonet-mobile` | cdylib      | Android JNI bindings driving the engine over a VpnService fd. |

## Quick start

### 1. Run a relay (on a publicly reachable host / VPS)

```sh
cargo run -p netonet-relay --release -- --bind 0.0.0.0:3340
```

The relay forwards only already-encrypted traffic. For a public deployment put
it behind a TLS-terminating reverse proxy, or use the upstream `iroh-relay`
binary if you want built-in ACME/TLS.

### 2. Configure two nodes

Copy `examples/node-a.toml` and `examples/node-b.toml`, set `relay_url` to your
relay, and fill in each peer's `endpoint_id`. To learn a node's id, run it once
without `secret_key` — it prints its public key and a `secret_key` value you can
paste back to keep a stable identity.

### 3. Run the nodes (needs root / `CAP_NET_ADMIN`)

```sh
# Host A
sudo ./target/release/netonet --config node-a.toml
# Host B
sudo ./target/release/netonet --config node-b.toml
```

Now `ping 10.7.0.2` from host A reaches host B over the overlay.

## Configuration reference

```toml
# Optional: 32-byte node secret key as 64 hex chars. Omit to generate one.
secret_key = "..."
# Your self-hosted relay. Omit to use n0's public relays + DNS discovery.
relay_url = "http://relay.example.com:3340"

[interface]
name = "netonet0"   # optional interface name
ip = "10.7.0.1"     # this node's overlay IP
prefix = 24         # overlay subnet prefix
mtu = 1380          # keep below the path MTU

[[peers]]
endpoint_id = "<peer public key>"
ip = "10.7.0.2"     # peer's overlay IP
```

When `relay_url` is set, netonet uses **only** that relay and performs **no**
external (n0) DNS/pkarr discovery — peers are reached purely by `endpoint_id`
plus the shared relay.

## Android

Android does not allow a process to open `/dev/net/tun` directly; the OS hands
your app a pre-configured TUN file descriptor via `VpnService`. `netonet-mobile`
takes that fd and runs the same engine. See [`android/README.md`](android/README.md)
for the build steps and a `VpnService` skeleton.

## Testing

```sh
cargo test -p netonet-core   # spins up a relay + two endpoints, round-trips a packet
```

## License

MIT OR Apache-2.0.
