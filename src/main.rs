use anyhow::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, UdpSocket};
use tokio::signal;
use tracing::info;

// mod api; // Removed per user request
use ad_nope::config::Config;
use ad_nope::engine::{BlocklistManager, StandardManager};
use ad_nope::logger::QueryLogger;
use ad_nope::resolver::UpstreamResolver;
use ad_nope::server::DnsHandler;
use ad_nope::stats::StatsCollector;
use hickory_server::ServerFuture;

#[tokio::main]
async fn main() -> Result<()> {
    // 2. Load Config (Before logging init to get level)
    let config_path = std::env::args().nth(1).unwrap_or("config.toml".to_string());
    let config = if std::path::Path::new(&config_path).exists() {
        Config::load(&config_path).await?
    } else {
        Config::default()
    };

    // 1. Setup Logging (Tracing)
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        let mut filter = config.logging.level.clone();
        // Reduce noise from hickory_proto (H3/QUIC timeouts) unless explicitly set
        if !filter.contains("hickory_proto") {
            filter.push_str(",hickory_proto=error");
        }
        tracing_subscriber::EnvFilter::new(filter)
    });

    tracing_subscriber::fmt().with_env_filter(env_filter).init();
    info!("Starting AdNope...");

    if !std::path::Path::new(&config_path).exists() {
        info!("Config file not found, using defaults.");
    }

    // 3. Init Stats & Logger
    let upstream_names = if config.upstream_servers.is_empty() {
        vec!["cloudflare-tls".to_string()]
    } else {
        config.upstream_servers.clone()
    };

    // Use keys from sorted blocklists as friendly names
    let blocklist_names: Vec<String> = config
        .get_blocklists_sorted()
        .into_iter()
        .map(|(name, _url)| name)
        .collect();

    let stats = StatsCollector::new(
        config.stats.log_interval_seconds,
        upstream_names,
        blocklist_names.clone(),
    );
    let logger = QueryLogger::new(config.logging.clone(), blocklist_names);
    // 4. Init Blocklist Manager & Fetch Initial Lists
    let manager = Arc::new(StandardManager::new(config.clone()));
    let initial_matcher = manager.refresh().await;
    // We need a way to share the matcher with the handler and update it later.
    // The handler currently takes Arc<dyn BlocklistMatcher>.
    // To support updates, the Handler needs interior mutability or we need to restart the server?
    // Restarting is bad.
    // Ideally we use arc_swap or RwLock.
    // Let's wrap the matcher in an Arc<RwLock<...>> or similar in the Handler,
    // BUT Handler trait signatures are strict.
    //
    // SIMPLEST SOLUTION for now:
    // The Handler will hold `Arc<arc_swap::ArcSwap<dyn BlocklistMatcher>>` (requires dependency)
    // OR we just use `Arc<tokio::sync::RwLock<Arc<dyn BlocklistMatcher>>>`.
    // Let's modify `server.rs` to support this dynamic update if we have time.
    // For this MVP step, let's just use the static initial matcher.

    // 5. Init Upstream Resolver
    let upstream_resolver = Arc::new(UpstreamResolver::new(config.clone(), stats.clone()).await?);

    // 6. Build Handler
    let handler = DnsHandler::new(
        config.clone(),
        stats.clone(),
        logger.clone(),
        initial_matcher,
        upstream_resolver.clone(),
    );

    // 7. Spawn Periodic Updater
    let update_interval = Duration::from_secs(config.updates.interval_hours * 3600);
    let manager_for_loop = manager.clone();
    let handler_clone = handler.clone();

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(update_interval);
        // The first tick completes immediately. Since we just refreshed in main(),
        // we consume the first tick so the loop waits for the full duration before the next refresh.
        interval.tick().await;

        loop {
            interval.tick().await;
            let matcher = manager_for_loop.refresh().await;
            handler_clone.update_blocklist(matcher).await;
        }
    });

    // 8. Start Server
    let mut server = ServerFuture::new(handler);

    let addr = SocketAddr::new(config.host.parse().unwrap(), config.port);

    // UDP
    let udp_socket = UdpSocket::bind(addr).await?;
    server.register_socket(udp_socket);

    // TCP
    let tcp_listener = TcpListener::bind(addr).await?;
    server.register_listener(tcp_listener, Duration::from_secs(5));

    info!("DNS Server listening on {}", addr);

    // 8. Graceful Shutdown
    tokio::select! {
        _ = server.block_until_done() => {},
        _ = signal::ctrl_c() => {
            info!("Shutdown signal received.");
        }
    }

    Ok(())
}
