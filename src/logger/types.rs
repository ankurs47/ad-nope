use hickory_server::proto::rr::RecordType;
use std::net::IpAddr;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct QueryLogEntry {
    pub client_ip: IpAddr,
    pub domain: Arc<str>,
    pub query_type: RecordType,
    pub action: QueryLogAction,
    pub source_id: Option<u8>, // If blocked
    pub upstream: Option<String>,
    pub latency_ms: u64,
    pub ttl_remaining: Option<u64>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum QueryLogAction {
    Allowed,
    Blocked,
    Cached,
    Forwarded,
    Local,
}

pub trait QueryLogSink: Send + Sync {
    fn log(&self, entry: &QueryLogEntry);
}
