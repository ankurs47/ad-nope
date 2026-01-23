#[derive(Debug, Clone)]
pub struct QueryLogEntry {
    pub client_ip: String,
    pub domain: String,
    pub query_type: String,
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
