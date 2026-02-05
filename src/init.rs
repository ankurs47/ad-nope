//! Initialization helpers for the application startup.

use crate::api::{ApiDataSource, InMemoryStatsSource, PersistentStatsSource};
use crate::config::Config;
use crate::db::DbClient;
use crate::logger::{MemoryLogSink, QueryLogSink};
use crate::stats::StatsCollector;
use std::sync::Arc;
use tracing::info;

/// Sets up the tracing subscriber with the configured filters.
pub fn setup_logging(config: &Config) {
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
}

/// Initializes the database client and the API data source.
///
/// Returns a tuple containing:
/// 1. An optional `MemoryLogSink` (if no SQLite sink is used).
/// 2. The `ApiDataSource` (either SQLite-backed or Memory-backed).
/// 3. The optional shared `DbClient`.
#[allow(clippy::type_complexity)]
pub fn init_data_source(
    config: &Config,
    stats: Arc<StatsCollector>,
) -> (
    Option<Box<dyn QueryLogSink>>,
    Arc<dyn ApiDataSource>,
    Option<Arc<DbClient>>,
) {
    let use_sqlite_sink = config
        .logging
        .query_log_sinks
        .contains(&"sqlite".to_string());

    let mut db_client: Option<Arc<DbClient>> = None;

    if use_sqlite_sink {
        info!("SQLite sink enabled. Initializing shared DbClient.");
        let client = Arc::new(DbClient::new(config.logging.sqlite_path.clone()));
        // Initialize schema immediately (safe to do here)
        if let Err(e) = client.initialize() {
            tracing::error!("Failed to initialize SQLite database: {}", e);
        } else {
            db_client = Some(client);
        }
    }

    if let Some(client) = db_client.clone() {
        // If DbClient is available (because sink is enabled), use it for API too.
        info!("Using PersistentStatsSource (SQLite) for API.");
        (
            None, // Memory sink disabled to save RAM since we have SQLite
            Arc::new(PersistentStatsSource::new(client)),
            db_client,
        )
    } else {
        info!("SQLite sink disabled. Using InMemoryStatsSource for API.");
        let sink = MemoryLogSink::new(100);
        let buffer = sink.clone_buffer();
        (
            Some(Box::new(sink)),
            Arc::new(InMemoryStatsSource::new(stats, buffer)),
            None,
        )
    }
}
