use anyhow::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, UdpSocket};
use tokio::signal;
use tracing::info;

use ad_nope::config::Config;
use ad_nope::engine::{AppState, BlocklistManager, StandardManager};
use ad_nope::init::{init_data_source, setup_logging};
use ad_nope::logger::QueryLogger;
use ad_nope::server::DnsHandler;
use ad_nope::stats::StatsCollector;
use hickory_server::ServerFuture;

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Load Config
    let config_path = std::env::args().nth(1).unwrap_or("config.toml".to_string());
    let config = if std::path::Path::new(&config_path).exists() {
        Config::load(&config_path).await?
    } else {
        Config::default()
    };

    // 2. Setup Logging
    setup_logging(&config);
    info!("Starting ad-nope...");

    if !std::path::Path::new(&config_path).exists() {
        info!("Config file not found, using defaults.");
    }

    // 3. Init Stats
    let upstream_names = config.upstream_servers.clone();
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

    // 4. Init Data Source & DB
    let (memory_sink, data_source, db_client) = init_data_source(&config, stats.clone());

    // 5. Init QueryLogger
    let mut extra_sinks = Vec::new();
    if let Some(sink) = memory_sink {
        extra_sinks.push(sink);
    }

    let logger = QueryLogger::new(
        config.logging.clone(),
        blocklist_names,
        extra_sinks,
        db_client,
    );

    // 6. Init Blocklist Manager & Fetch Initial Lists
    let manager = Arc::new(StandardManager::new(config.clone()));
    let initial_matcher = manager.refresh().await;

    // 7. Init AppState (Pause Control)
    let app_state = AppState::new();

    // 8. Init Upstream Resolver
    let upstream_resolver =
        ad_nope::resolver::create_resolver(config.clone(), stats.clone()).await?;

    // 9. Build Handler
    let handler = DnsHandler::new(
        config.clone(),
        stats.clone(),
        logger.clone(),
        initial_matcher,
        upstream_resolver.clone(),
        app_state.clone(),
    );

    // 10. Spawn Periodic Updater & Initial Refresh Signal
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

    // 11. Start API Server (Embedded UI)
    let api_state = app_state.clone();
    let api_config = config.clone();
    let api_refresh_tx = refresh_tx.clone();

    tokio::spawn(async move {
        ad_nope::api::start_api_server(data_source, api_state, api_config, api_refresh_tx, 8080)
            .await;
    });

    // 12. Start Server
    let mut server = ServerFuture::new(handler);
    let addr = SocketAddr::new(config.host.parse().unwrap(), config.port);

    // UDP
    let udp_socket = UdpSocket::bind(addr).await?;
    server.register_socket(udp_socket);

    // TCP
    let tcp_listener = TcpListener::bind(addr).await?;
    server.register_listener(tcp_listener, Duration::from_secs(5));

    info!("DNS Server listening on {}", addr);

    // 13. Graceful Shutdown
    tokio::select! {
        _ = server.block_until_done() => {},
        _ = signal::ctrl_c() => {
            info!("Shutdown signal received.");
        }
    }

    Ok(())
}
