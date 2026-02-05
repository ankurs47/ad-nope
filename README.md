# AdNope

> [!WARNING]
> **Disclaimer: This code is AI-generated and not human-verified. Use at your own risk.**

**AdNope** is a high-performance, asynchronous DNS ad-blocker written in Rust. It acts as a DNS sinkhole, blocking requests to known advertisement and tracking domains while forwarding legitimate queries to upstream DNS resolvers.

It was built from scratch in 2 days by an AI Agent (Antigravity by Google) under human guidance, inspired by "block adblocker" and done for fun.

## Features

- **High Performance**: Built on `tokio` for non-blocking I/O and `hickory-server` (formerly trust-dns) for robust DNS handling.
- **Concurrent Blocklist Updates**: Fetches and parses multiple blocklists in parallel.
- **In-Memory Caching**: Implements a high-speed cache with TTL enforcement and stale-while-revalidate background refreshing.
- **Granular Statistics**: Tracks detailed usage metrics including top clients, top domains, and blocklist effectiveness using sharded `DashMap`s for concurrency.
- **API & UI**: Embedded API server and web interface (served from `http://localhost:8080` by default) for monitoring and control.
- **Privacy-Aware Logging**: Configurable logging backend (Console, SQLite, Memory) with options to anonymize or skip logging.
- **Hot Reloading**: Supports live pausing/resuming of blocking logic via API.

## Getting Started

### Prerequisites

- Rust (latest stable)
- Cargo

### Installation

1.  Clone the repository:
    ```bash
    git clone https://github.com/ankurs47/ad-nope.git
    cd ad-nope
    ```

2.  Run the server:
    ```bash
    cargo run --release -- config.toml
    ```

### Running Tests

To verify the codebase:

```bash
cargo test
```

## Configuration

The application is configured via a `config.toml` file. If not provided, it falls back to sensible defaults.

**Key Settings:**

- `upstream_servers`: List of efficient DNS resolvers (e.g., DNS-over-HTTPS/QUIC).
- `blocklists`: Map of blocklist names to URLs.
- `cache`: Tuning for capacity and TTLs.
- `logging`: Logging levels and sinks (Console, SQLite).

**Example `config.toml`:**

```toml
host = "0.0.0.0"
port = 5300

upstream_servers = ["h3://dns.google/dns-query"]

[cache]
enable = true
capacity = 10000
grace_period_sec = 10

[updates]
interval_hours = 24
concurrent_downloads = 4
```

## Architecture

**AdNope** follows a modular architecture designed for extensibility and readability:

- **`src/main.rs`**: Application entry point. Handles configuration loading and component initialization.
- **`src/server`**: `DnsHandler` manages the request lifecycle: matching blocklists, checking cache, and resolving via upstream.
- **`src/engine`**:
  - `BlocklistManager`: Downloads and parses blocklists.
  - `DomainMatcher`: Optimized in-memory matching logic (suffix matching).
- **`src/resolver`**: Flexible upstream resolution strategies (Parallel, Round-Robin).
- **`src/stats.rs`**: High-concurrency metric collection using atomic counters and concurrent hash maps.
- **`src/logger`**: Structured query logging system supporting multiple sinks (Console, SQLite).
- **`src/api`**: Axum-based HTTP API for the implementation of the frontend UI.
- **`src/config.rs`**: Strongly-typed configuration with defaults.

## Development

This project emphasizes descriptiveness and documentation.

- New components should implement clear traits (`QueryLogSink`, `ApiDataSource`).
- Public items should have `rustdoc` comments (`///`).
- Complex logic is explained with inline comments.

### Contributing

1.  Fork the repo.
2.  Create your feature branch.
3.  Add tests for your changes.
4.  Submit a Pull Request.

---
*Built with ❤️ (and silicon) by Antigravity.*
