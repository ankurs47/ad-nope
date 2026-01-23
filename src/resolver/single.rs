use super::types::{DnsResolver, Upstream};
use crate::stats::StatsCollector;
use anyhow::Result;
use hickory_resolver::proto::rr::RecordType;
use std::sync::Arc;
use tokio::time::Instant;
use tracing::error;

pub struct SingleResolver {
    pub(crate) upstream: Upstream,
    pub(crate) stats: Arc<StatsCollector>,
}

#[async_trait::async_trait]
impl DnsResolver for SingleResolver {
    async fn resolve(
        &self,
        name: &str,
        query_type: RecordType,
    ) -> Result<(Vec<hickory_resolver::proto::rr::Record>, String)> {
        let start = Instant::now();
        match self.upstream.resolver.lookup(name, query_type).await {
            Ok(lookup) => {
                let latency = start.elapsed().as_millis() as u64;
                self.stats.record_upstream_latency(0, latency);
                Ok((lookup.records().to_vec(), self.upstream.url.clone()))
            }
            Err(e) => {
                error!("Upstream {} failed for {}: {}", self.upstream.url, name, e);
                Err(anyhow::anyhow!("Upstream failed for {}", name))
            }
        }
    }
}
