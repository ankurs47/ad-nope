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
