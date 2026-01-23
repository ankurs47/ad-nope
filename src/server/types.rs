use crate::logger::LogAction;
use hickory_server::proto::rr::RecordType;
use std::time::Instant;

#[derive(Clone)]
pub struct QueryContext {
    pub name: String,
    pub qtype: RecordType,
    pub start: Instant,
}

pub struct LogContext {
    pub action: LogAction,
    pub source_id: Option<u8>,
    pub upstream: Option<String>,
}
