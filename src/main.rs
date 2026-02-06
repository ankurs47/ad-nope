use anyhow::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, UdpSocket};
use tokio::signal;
use tracing::info;

use ad_nope::config::Config;
use ad_nope::engine::{AppState, BlocklistManager, StandardManager};
use ad_nope::init::{
    create_memory_source, create_persistent_source, init_sqlite_db, setup_logging,
};
use ad_nope::logger::QueryLogger;
use ad_nope::server::DnsHandler;
use ad_nope::stats::StatsCollector;
use hickory_server::ServerFuture;

#[tokio::main]
async fn main() -> Result<()> {
    let (config, config_path) = load_config().await?;
    setup_logging(&config);
    info!("Starting ad-nope...");

    if !std::path::Path::new(&config_path).exists() {
        info!("Config file not found, using defaults.");
    }

    let (stats, blocklist_names) = init_app_stats(&config);
    let db_client = init_sqlite_db(&config);
    let (data_source, logger) = match db_client {
        Some(client) => {
            let log_writer = match client.create_log_writer() {
                Ok(w) => Some(w),
                Err(e) => {
                    tracing::error!("Failed to create log writer from DbClient: {}", e);
                    None
                }
            };

            let logger = init_query_logger(&config, &blocklist_names, log_writer, None);
            let source = create_persistent_source(client);
            (source, logger)
        }
        None => {
            let (sink, source) = create_memory_source(stats.clone());
            let logger = init_query_logger(&config, &blocklist_names, None, sink);
            (source, logger)
        }
    };

    let manager = Arc::new(StandardManager::new(config.clone()));
    let initial_matcher = manager.refresh().await;
    let app_state = AppState::new();

    let upstream_resolver =
        ad_nope::resolver::create_resolver(config.clone(), stats.clone()).await?;

    let handler = DnsHandler::new(
        config.clone(),
        stats.clone(),
        logger,
        initial_matcher,
        upstream_resolver,
        app_state.clone(),
    );

    let (refresh_tx, refresh_rx) = tokio::sync::mpsc::channel::<()>(1);
    spawn_background_tasks(
        &config,
        manager.clone(),
        app_state,
        data_source,
        handler.clone(),
        refresh_tx,
        refresh_rx,
    );

    run_server(config, handler).await
}

async fn load_config() -> Result<(Config, String)> {
    let config_path = std::env::args().nth(1).unwrap_or("config.toml".to_string());
    let config = if std::path::Path::new(&config_path).exists() {
        Config::load(&config_path).await?
    } else {
        Config::default()
    };
    Ok((config, config_path))
}

fn init_app_stats(config: &Config) -> (Arc<StatsCollector>, Vec<String>) {
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
    (stats, blocklist_names)
}

fn init_query_logger(
    config: &Config,
    blocklist_names: &[String],
    log_writer: Option<ad_nope::db::LogWriter>,
    memory_sink: Option<Box<dyn ad_nope::logger::QueryLogSink>>,
) -> Arc<QueryLogger> {
    let mut extra_sinks: Vec<Box<dyn ad_nope::logger::QueryLogSink>> = Vec::new();
    if let Some(sink) = memory_sink {
        extra_sinks.push(sink);
    }

    QueryLogger::new(
        config.logging.clone(),
        blocklist_names.to_vec(),
        extra_sinks,
        log_writer,
    )
}

fn spawn_background_tasks(
    config: &Config,
    manager: Arc<StandardManager>,
    app_state: AppState,
    data_source: Arc<dyn ad_nope::api::ApiDataSource>,
    handler: DnsHandler,
    refresh_tx: tokio::sync::mpsc::Sender<()>,
    refresh_rx: tokio::sync::mpsc::Receiver<()>,
) {
    // 1. Periodic Updater
    let update_interval = Duration::from_secs(config.updates.interval_hours * 3600);
    let manager_clone = manager;
    let mut refresh_rx = refresh_rx;
    let handler_clone = handler;

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(update_interval);
        interval.tick().await;

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    info!("Scheduled blocklist update...");
                }
                _ = refresh_rx.recv() => {
                    info!("Forced blocklist update triggered via API...");
                    interval.reset();
                }
            }
            let matcher = manager_clone.refresh().await;
            handler_clone.update_blocklist(matcher).await;
        }
    });

    // 2. API Server
    let api_state = app_state;
    let api_config = config.clone();
    let api_refresh_tx = refresh_tx;

    tokio::spawn(async move {
        ad_nope::api::start_api_server(data_source, api_state, api_config, api_refresh_tx, 8080)
            .await;
    });
}

async fn run_server(config: Config, handler: DnsHandler) -> Result<()> {
    let mut server = ServerFuture::new(handler);
    let addr = SocketAddr::new(config.host.parse().unwrap(), config.port);

    let udp_socket = UdpSocket::bind(addr).await?;
    server.register_socket(udp_socket);

    let tcp_listener = TcpListener::bind(addr).await?;
    server.register_listener(tcp_listener, Duration::from_secs(5));

    info!("DNS Server listening on {}", addr);

    tokio::select! {
        _ = server.block_until_done() => {},
        _ = signal::ctrl_c() => {
            info!("Shutdown signal received.");
        }
    }

    Ok(())
}
