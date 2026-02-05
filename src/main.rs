use anyhow::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, UdpSocket};
use tokio::signal;
use tracing::info;

use ad_nope::config::Config;
use ad_nope::db::DbClient;
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

    // 3a. Init Shared DbClient if needed
    let use_sqlite_sink = config
        .logging
        .query_log_sinks
        .contains(&"sqlite".to_string());
    // In this unified model, if use_sqlite_sink is true, we need DbClient.
    // Also, if we want to allow the API to view historical data even if sink is disabled (not typical but possible if DB exists),
    // we could check file existence.
    // For now, let's stick to: if "sqlite" sink is enabled OR database file exists?
    // User request: "can we creat one db client and share it with both query sink and data source"
    // So distinct path for "sqlite" sink enabled.

    let mut db_client: Option<Arc<DbClient>> = None;

    if use_sqlite_sink {
        info!("SQLite sink enabled. Initializing shared DbClient.");
        let client = Arc::new(DbClient::new(config.logging.sqlite_path.clone()));
        // Initialize schema immediately (safe to do here)
        if let Err(e) = client.initialize() {
            tracing::error!("Failed to initialize SQLite database: {}", e);
            // Non-fatal? Maybe. logging will fail later.
        } else {
            db_client = Some(client);
        }
    }

    // 3b. Init DataSource & Optional Memory Sink
    let (memory_sink, data_source): (
        Option<Box<dyn ad_nope::logger::QueryLogSink>>,
        Arc<dyn ad_nope::api::ApiDataSource>,
    ) = if let Some(client) = db_client.clone() {
        // If DbClient is available (because sink is enabled), use it for API too.
        // This implies persistent stats.
        info!("Using SqliteDataSource for API.");
        (
            None, // Memory sink disabled to save RAM since we have SQLite
            Arc::new(ad_nope::api::sqlite::SqliteDataSource::new(client)),
        )
    } else {
        info!("SQLite sink disabled. Using MemoryDataSource for API.");
        let sink = ad_nope::logger::MemoryLogSink::new(100);
        let buffer = sink.clone_buffer();
        (
            Some(Box::new(sink)),
            Arc::new(ad_nope::api::memory::MemoryDataSource::new(
                stats.clone(),
                buffer,
            )),
        )
    };

    // 3c. Init QueryLogger
    let mut extra_sinks = Vec::new();
    if let Some(sink) = memory_sink {
        extra_sinks.push(sink);
    }

    let logger = QueryLogger::new(
        config.logging.clone(),
        blocklist_names,
        extra_sinks,
        db_client.clone(), // Pass the shared client (Option<Arc<DbClient>>)
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
    let api_state = app_state.clone();
    let api_config = config.clone();
    let api_refresh_tx = refresh_tx.clone();

    tokio::spawn(async move {
        ad_nope::api::start_api_server(data_source, api_state, api_config, api_refresh_tx, 8080)
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
