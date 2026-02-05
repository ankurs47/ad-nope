use super::source::ApiDataSource;
use crate::logger::types::QueryLogEntry;
use crate::stats::{StatsCollector, StatsSnapshot};
use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::{Arc, RwLock};

pub struct MemoryDataSource {
    stats: Arc<StatsCollector>,
    logs_buffer: Arc<RwLock<VecDeque<QueryLogEntry>>>,
}

impl MemoryDataSource {
    pub fn new(
        stats: Arc<StatsCollector>,
        logs_buffer: Arc<RwLock<VecDeque<QueryLogEntry>>>,
    ) -> Self {
        Self { stats, logs_buffer }
    }
}

#[async_trait]
impl ApiDataSource for MemoryDataSource {
    async fn get_stats(&self) -> StatsSnapshot {
        self.stats.get_snapshot()
    }

    async fn get_logs(&self, limit: usize) -> Vec<QueryLogEntry> {
        let buffer = self.logs_buffer.read().unwrap();
        buffer.iter().rev().take(limit).cloned().collect()
    }
}
