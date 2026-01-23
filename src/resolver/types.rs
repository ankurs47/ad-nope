use anyhow::Result;
use hickory_resolver::proto::rr::{Record, RecordType};
use hickory_resolver::TokioResolver;

/// Abstract upstream resolver for mocking and switching implementations.
#[async_trait::async_trait]
pub trait DnsResolver: Send + Sync {
    async fn resolve(&self, name: &str, query_type: RecordType) -> Result<(Vec<Record>, String)>;
}

pub struct Upstream {
    pub url: String,
    pub resolver: TokioResolver,
}
