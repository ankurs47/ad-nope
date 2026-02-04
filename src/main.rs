use anyhow::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, UdpSocket};
use tokio::signal;
use tracing::info;

use ad_nope::config::Config;
use ad_nope::engine::{AppState, BlocklistManager, StandardManager};
use ad_nope::logger::QueryLogger;
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

        // Suppress hickory_server logs unless explicitly enabled/overridden
        if !filter.contains("hickory_server") {
            filter.push_str(",hickory_server=off");
        }
        // Also suppress hickory_proto if not set
        if !filter.contains("hickory_proto") {
            filter.push_str(",hickory_proto=off");
        }

        tracing_subscriber::EnvFilter::new(filter)
    });

    tracing_subscriber::fmt().with_env_filter(env_filter).init();
    info!("Starting ad-nope...");

    if !std::path::Path::new(&config_path).exists() {
        info!("Config file not found, using defaults.");
    }

    // 3. Init Stats & Logger
    let upstream_names = config.upstream_servers.clone();

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
    // 3a. Init Memory Logger
    let memory_sink = ad_nope::logger::MemoryLogSink::new(100); // Keep last 100 queries
    let memory_buffer = memory_sink.clone_buffer();

    // 3b. Init QueryLogger with MemorySink
    let logger = QueryLogger::new(
        config.logging.clone(),
        blocklist_names,
        vec![Box::new(memory_sink)],
    );

    // 4. Init Blocklist Manager & Fetch Initial Lists
    let manager = Arc::new(StandardManager::new(config.clone()));
    let initial_matcher = manager.refresh().await;

    // 6.5 Init AppState (Pause Control)
    let app_state = AppState::new();

    // 5. Init Upstream Resolver
    let upstream_resolver =
        ad_nope::resolver::create_resolver(config.clone(), stats.clone()).await?;

    // 6. Build Handler
    let handler = DnsHandler::new(
        config.clone(),
        stats.clone(),
        logger.clone(),
        initial_matcher,
        upstream_resolver.clone(),
        app_state.clone(), // Pass AppState
    );

    // 7. Spawn Periodic Updater & Initial Refresh Signal
    let update_interval = Duration::from_secs(config.updates.interval_hours * 3600);
    let manager_for_loop = manager.clone();
    let handler_clone = handler.clone();

    // Channel for forcing refresh
    let (refresh_tx, mut refresh_rx) = tokio::sync::mpsc::channel::<()>(1);

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(update_interval);
        // The first tick completes immediately
        interval.tick().await;

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    info!("Scheduled blocklist update...");
                }
                _ = refresh_rx.recv() => {
                    info!("Forced blocklist update triggered via API...");
                    interval.reset(); // Reset timer to avoid double update
                }
            }
            let matcher = manager_for_loop.refresh().await;
            handler_clone.update_blocklist(matcher).await;
        }
    });

    // 8. Start API Server (Embedded UI)
    let api_stats = stats.clone();
    let api_state = app_state.clone();
    let api_config = config.clone();
    let api_refresh_tx = refresh_tx.clone();
    let api_memory_buffer = memory_buffer;

    tokio::spawn(async move {
        ad_nope::api::start_api_server(
            api_stats,
            api_state,
            api_config,
            api_refresh_tx,
            api_memory_buffer,
            8080,
        )
        .await;
    });

    // 9. Start Server
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
