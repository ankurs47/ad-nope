use crate::logger::QueryLogAction;
use hickory_server::proto::rr::RecordType;
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone)]
pub struct QueryContext {
    pub name: Arc<str>,
    pub qtype: RecordType,
    pub start: Instant,
}

use std::borrow::Cow;

pub struct LogContext {
    pub action: QueryLogAction,
    pub source_id: Option<u8>,
    pub upstream: Option<Cow<'static, str>>,
    pub ttl_remaining: Option<u64>,
}
