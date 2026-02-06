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
/// 1. An optional `MemoryLogSink` (if no SQLite sink is used).
/// 2. The `ApiDataSource` (either SQLite-backed or Memory-backed).
/// 3. The optional shared `DbClient`.
pub fn create_persistent_source(db_client: DbClient) -> Arc<dyn ApiDataSource> {
    info!("Using PersistentStatsSource (SQLite) for API.");
    Arc::new(PersistentStatsSource::new(db_client))
}

pub fn create_memory_source(
    stats: Arc<StatsCollector>,
) -> (Option<Box<dyn QueryLogSink>>, Arc<dyn ApiDataSource>) {
    info!("Using InMemoryStatsSource for API.");
    let sink = MemoryLogSink::new(100);
    let buffer = sink.clone_buffer();
    (
        Some(Box::new(sink)),
        Arc::new(InMemoryStatsSource::new(stats, buffer)),
    )
}

pub fn init_sqlite_db(config: &Config) -> Option<DbClient> {
    if !config
        .logging
        .query_log_sinks
        .contains(&"sqlite".to_string())
    {
        return None;
    }

    match DbClient::new(config.logging.sqlite_path.clone()) {
        Ok(client) => {
            if let Err(e) = client.initialize() {
                tracing::error!("Failed to initialize SQLite database: {}", e);
                return None;
            }
            Some(client)
        }
        Err(e) => {
            tracing::error!("Failed to open SQLite database: {}", e);
            None
        }
    }
}
