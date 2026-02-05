//! In-memory implementation of the API data source.
//!
//! This data source is used when no persistent storage (SQLite) is configured.
//! It serves real-time statistics directly from the `StatsCollector` and recent
//! logs from a fixed-size ring buffer.

use super::source::ApiDataSource;
use crate::logger::types::QueryLogEntry;
use crate::stats::{StatsCollector, StatsSnapshot};
use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::{Arc, RwLock};

/// An API data source that reads from in-memory structures.
pub struct InMemoryStatsSource {
    /// Reference to the live statistics collector.
    stats: Arc<StatsCollector>,
    /// Shared buffer containing the most recent query logs.
    logs_buffer: Arc<RwLock<VecDeque<QueryLogEntry>>>,
}

impl InMemoryStatsSource {
    /// Creates a new `InMemoryStatsSource`.
    pub fn new(
        stats: Arc<StatsCollector>,
        logs_buffer: Arc<RwLock<VecDeque<QueryLogEntry>>>,
    ) -> Self {
        Self { stats, logs_buffer }
    }
}

#[async_trait]
impl ApiDataSource for InMemoryStatsSource {
    async fn get_stats(&self) -> StatsSnapshot {
        self.stats.get_snapshot()
    }

    async fn get_logs(&self, limit: usize) -> Vec<QueryLogEntry> {
        let buffer = self.logs_buffer.read().unwrap();
        buffer.iter().rev().take(limit).cloned().collect()
    }
}
