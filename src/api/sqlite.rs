use super::source::ApiDataSource;
use crate::db::DbClient;
use crate::logger::types::QueryLogEntry;
use crate::stats::StatsSnapshot;
use async_trait::async_trait;
use std::sync::Arc;

pub struct SqliteDataSource {
    db: Arc<DbClient>,
}

impl SqliteDataSource {
    pub fn new(db: Arc<DbClient>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl ApiDataSource for SqliteDataSource {
    async fn get_stats(&self) -> StatsSnapshot {
        self.db.get_stats()
    }

    async fn get_logs(&self, limit: usize) -> Vec<QueryLogEntry> {
        self.db.get_logs(limit)
    }
}
