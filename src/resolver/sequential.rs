use super::types::{DnsResolver, Upstream};
use crate::stats::StatsCollector;
use anyhow::Result;
use hickory_resolver::proto::rr::RecordType;
use std::sync::Arc;
use tokio::time::Instant;
use tracing::error;

pub struct SequentialResolver {
    pub(crate) upstreams: Vec<Upstream>,
    pub(crate) stats: Arc<StatsCollector>,
}

#[async_trait::async_trait]
impl DnsResolver for SequentialResolver {
    async fn resolve(
        &self,
        name: &str,
        query_type: RecordType,
    ) -> Result<(Vec<hickory_resolver::proto::rr::Record>, String)> {
        let start = Instant::now();
        for (idx, upstream) in self.upstreams.iter().enumerate() {
            match upstream.resolver.lookup(name, query_type).await {
                Ok(lookup) => {
                    let latency = start.elapsed().as_millis() as u64;
                    self.stats.record_upstream_latency(idx, latency);
                    return Ok((lookup.records().to_vec(), upstream.url.clone()));
                }
                Err(e) => {
                    error!("Upstream {} failed for {}: {}", upstream.url, name, e);
                    continue;
                }
            }
        }
        Err(anyhow::anyhow!("All upstreams failed for {}", name))
    }
}
