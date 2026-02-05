use crate::logger::types::QueryLogEntry;
use crate::stats::StatsSnapshot;
use async_trait::async_trait;

#[async_trait]
pub trait ApiDataSource: Send + Sync {
    async fn get_stats(&self) -> StatsSnapshot;
    async fn get_logs(&self, limit: usize) -> Vec<QueryLogEntry>;
}
