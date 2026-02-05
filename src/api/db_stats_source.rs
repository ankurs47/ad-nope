//! Database-backed implementation of the API data source.
//!
//! This data source delegates requests to the SQLite `DbClient`.
//! It allows the API to serve historical data even if the application restarts.

use super::source::ApiDataSource;
use crate::db::DbClient;
use crate::logger::types::QueryLogEntry;
use crate::stats::StatsSnapshot;
use async_trait::async_trait;
use std::sync::Arc;

/// An API data source that reads from a persistent SQLite database.
pub struct PersistentStatsSource {
    /// Client for database operations.
    db: Arc<DbClient>,
}

impl PersistentStatsSource {
    /// Creates a new `PersistentStatsSource`.
    pub fn new(db: Arc<DbClient>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl ApiDataSource for PersistentStatsSource {
    async fn get_stats(&self) -> StatsSnapshot {
        self.db.get_stats()
    }

    async fn get_logs(&self, limit: usize) -> Vec<QueryLogEntry> {
        self.db.get_logs(limit)
    }
}
