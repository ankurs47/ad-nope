use hickory_server::proto::rr::RecordType;
use serde::Serialize;
use std::net::IpAddr;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize)]
pub struct QueryLogEntry {
    pub client_ip: IpAddr,
    #[serde(serialize_with = "serialize_arc_str")]
    pub domain: Arc<str>,
    #[serde(serialize_with = "serialize_record_type")]
    pub query_type: RecordType,
    pub action: QueryLogAction,
    pub source_id: Option<u8>, // If blocked
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    pub latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_remaining: Option<u64>,
}

fn serialize_record_type<S>(rt: &RecordType, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&rt.to_string())
}

fn serialize_arc_str<S>(domain: &Arc<str>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(domain)
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize)]
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
