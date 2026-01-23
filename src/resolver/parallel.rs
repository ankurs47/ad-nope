use super::types::{DnsResolver, Upstream};
use crate::stats::StatsCollector;
use anyhow::Result;
use hickory_resolver::proto::rr::RecordType;
use std::sync::Arc;
use tokio::time::Instant;
use tracing::error;

pub struct ParallelResolver {
    pub(crate) upstreams: Vec<Upstream>,
    pub(crate) stats: Arc<StatsCollector>,
}

#[async_trait::async_trait]
impl DnsResolver for ParallelResolver {
    async fn resolve(
        &self,
        name: &str,
        query_type: RecordType,
    ) -> Result<(Vec<hickory_resolver::proto::rr::Record>, String)> {
        let start = Instant::now();

        let mut futures = Vec::new();
        for (idx, upstream) in self.upstreams.iter().enumerate() {
            let name = name.to_string();
            let f = async move {
                let res = upstream.resolver.lookup(name, query_type).await;
                (res, upstream.url.clone(), idx)
            };
            futures.push(Box::pin(f));
        }

        let mut futures = futures;
        while !futures.is_empty() {
            let ((res, url, idx), _index, remaining) = futures::future::select_all(futures).await;
            futures = remaining;

            match res {
                Ok(lookup) => {
                    let latency = start.elapsed().as_millis() as u64;
                    self.stats.record_upstream_latency(idx, latency);
                    return Ok((lookup.records().to_vec(), url));
                }
                Err(e) => {
                    error!("Upstream {} failed for {}: {}", url, name, e);
                }
            }
        }
        Err(anyhow::anyhow!("All upstreams failed for {}", name))
    }
}
