use super::types::{DnsResolver, Upstream};
use crate::stats::StatsCollector;
use anyhow::Result;
use hickory_resolver::proto::rr::RecordType;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::time::Instant;
use tracing::error;

pub struct RoundRobinResolver {
    upstreams: Vec<Upstream>,
    stats: Arc<StatsCollector>,
    current: AtomicUsize,
}

impl RoundRobinResolver {
    pub fn new(upstreams: Vec<Upstream>, stats: Arc<StatsCollector>) -> Self {
        Self {
            upstreams,
            stats,
            current: AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl DnsResolver for RoundRobinResolver {
    async fn resolve(
        &self,
        name: &str,
        query_type: RecordType,
    ) -> Result<(Vec<hickory_resolver::proto::rr::Record>, String)> {
        let start = Instant::now();
        let start_index = self.current.fetch_add(1, Ordering::Relaxed) % self.upstreams.len();

        // Try starting from start_index, then loop around once
        for i in 0..self.upstreams.len() {
            let idx = (start_index + i) % self.upstreams.len();
            let upstream = &self.upstreams[idx];

            match crate::resolver::handle_lookup_result(
                upstream.resolver.lookup(name, query_type).await,
                &self.stats,
                idx,
                upstream.url.clone(),
                start,
            ) {
                Ok(val) => return Ok(val),
                Err(e) => {
                    error!("Upstream {} failed for {}: {}", upstream.url, name, e);
                    continue;
                }
            }
        }
        Err(anyhow::anyhow!("All upstreams failed for {}", name))
    }
}
