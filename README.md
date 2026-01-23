# ad-nope

[![Docker Build and Publish](https://github.com/ankurs47/ad-nope/actions/workflows/docker-publish.yml/badge.svg)](https://github.com/ankurs47/ad-nope/actions/workflows/docker-publish.yml)

> [!IMPORTANT]
> **✨ 100% AI-Generated Code ✨**
>
> This project was built entirely from scratch using **Google's Antigravity** in just 2 days. I didn't write a single line of code—I just provided the "executive vision" (read: told the AI what to do).
>
> Inspired by [blocky](https://github.com/0xERR0R/blocky) and built for fun. If it breaks, blame the robot! 🤖

ad-nope is a high-performance, privacy-focused DNS resolver and ad-blocker written in Rust. It leverages the power of [Hickory DNS](https://github.com/hickory-dns/hickory-dns) to support modern, secure DNS protocols for upstream resolution.

## 🚀 Features

* **Modern Protocols**: Supports DNS-over-HTTPS (DoH), DNS-over-TLS (DoT), DNS-over-QUIC (DoQ), and DNS-over-HTTP/3 (DoH3) for upstream connections.
* **Ad & Tracker Blocking**: Downloads and enforces standard blocklists (e.g., Hagezi's lists).
* **High Performance**: Built on asynchronous Rust (Tokio) with parallel upstream queries and efficient caching (Moka).
* **Connection Multiplexing**: Reuses connections to upstream resolvers (H3/QUIC/HTTPS) to minimize latency.
* **Observability**: Detailed statistics and structured logging (text or JSON) for blocked queries, cache hits, and upstream latency.
* **Multi-Arch Docker**: Runs on x86_64 servers and ARM64 devices (like Raspberry Pi).

## 🐳 Docker Quick Start

The easiest way to run ad-nope is using the official Docker image.

### 1. Pull the Image

```bash
docker pull ankurs47/ad-nope:latest
```

### 2. Run the Container

ad-nope listens on port `5300` inside the container by default (to avoid needing root privileges). You should map this to port `53` on your host.

```bash
docker run -d \
  --name ad-nope \
  --restart unless-stopped \
  -p 53:5300/udp \
  -p 53:5300/tcp \
  ankurs47/ad-nope:latest
```

### 3. Verify

You can test the server using `dig`:

```bash
dig @localhost -p 53 google.com
```

## ⚙️ Configuration

ad-nope is configured via a TOML file. You can mount your own configuration into the Docker container to customize behavior.

### Default Configuration

See [example_config.toml](example_config.toml) for an example configuration.

### Mounting Custom Config

To run with a custom configuration, mount your config file to `/app/config.toml` inside the container:

```bash
docker run -d \
  --name ad-nope \
  -p 53:5300/udp \
  -p 53:5300/tcp \
  -v $(pwd)/my-config.toml:/app/config.toml \
  -v $(pwd)/data:/app/data \
  ankurs47/ad-nope:latest
```

> **Note**: For SQLite logging, ensure your config points `sqlite_path` to the mounted volume, e.g., `sqlite_path = "/app/data/ad-nope.db"`.

### Docker Compose

Create a `docker-compose.yml` file:

```yaml
version: '3'
services:
  ad-nope:
    image: ankurs47/ad-nope:latest
    ports:
      - "53:5300/tcp"
      - "53:5300/udp"
    volumes:
      - ./my-config.toml:/app/config.toml
      - ./data:/app/data
    restart: unless-stopped
```

### Key Settings

**Upstream Servers**
Supports `udp`, `tcp`, `tls`, `https` (DoH), `quic` (DoQ), and `h3` (DoH3).

```toml
upstream_servers = [
  "h3://dns.google/dns-query",
  "quic://dns.controld.com/unfiltered",
  "https://dns10.quad9.net/dns-query"
]
```

**Blocklists**
Map friendly names to blocklist URLs (text format, one domain per line).

```toml
[blocklists]
hagezi-pro = "https://cdn.jsdelivr.net/gh/hagezi/dns-blocklists@latest/wildcard/pro-onlydomains.txt"
hagezi-gambling = "https://cdn.jsdelivr.net/gh/hagezi/dns-blocklists@latest/wildcard/gambling-onlydomains.txt"
```

**Caching & Performance**

```toml
parallel_queries = true        # Race all upstreams, return fastest
upstream_timeout_ms = 5000     # Timeout in milliseconds
upstream_connection_pool_size = 2 # Connections per upstream
```

## 🛠️ Building from Source

### Prerequisites

* Rust (latest stable)
* Build tools (CMake, C++ compiler) for cryptographic dependencies.

### Build & Run

```bash
# Clone the repository
git clone https://github.com/ankurs47/ad-nope.git
cd ad-nope

# Run in development mode
cargo run -- example_config.toml

# Build release binary
cargo build --release
```

### Cross-Compilation (ARM / Raspberry Pi)

The easiest way to cross-compile locally is using [cross](https://github.com/cross-rs/cross).

1. Install `cross`:
   ```bash
   cargo install cross
   ```
2. Build for ARM64 (e.g., Raspberry Pi 4/5, Docker ARM64):
   ```bash
   cross build --target aarch64-unknown-linux-gnu --release
   ```
3. The binary will be in `target/aarch64-unknown-linux-gnu/release/ad-nope`.

## 📊 Monitoring

ad-nope outputs stats to the console (or configured log target) periodically:

```text
STATS DUMP: Total: 1542, Blocked: 231 (15.0%), CacheHits: 412 (26.7%), Upstreams: [dns.google: 25.4ms] [cloudflare: 18.2ms] BlockStats: [hagezi-pro: 200 (86.6%)] [hagezi-gambling: 31 (13.4%)]
```

## 📜 License

This project is licensed under the MIT License.
