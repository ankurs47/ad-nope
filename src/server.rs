use crate::config::Config;
use crate::engine::BlocklistMatcher;
use crate::logger::{LogAction, LogEntry, QueryLogger};
use crate::resolver::DnsResolver;
use crate::stats::StatsCollector;
use hickory_server::authority::MessageResponseBuilder;
use hickory_server::proto::op::ResponseCode;
use hickory_server::proto::rr::{RData, Record, RecordType};
use hickory_server::server::{Request, RequestHandler, ResponseHandler, ResponseInfo};
use moka::future::Cache;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::Instant;
use tracing::{error, info};

#[derive(Clone)]
pub struct DnsHandler {
    config: Config,
    stats: Arc<StatsCollector>,
    logger: Arc<QueryLogger>,
    // Wrap in RwLock for hot-reloading
    blocklist: Arc<tokio::sync::RwLock<Arc<dyn BlocklistMatcher>>>,
    resolver: Arc<dyn DnsResolver>,
    // Cache: Key(Name, Type) -> (Records, ValidUntil, StaleUntil)
    // Cache: Key(Name, Type) -> (Records, ValidUntil, StaleUntil)
    // We store Vec<Record> mostly.
    cache: Cache<(String, RecordType), (Vec<Record>, Instant, Instant)>,
}

impl DnsHandler {
    pub fn new(
        config: Config,
        stats: Arc<StatsCollector>,
        logger: Arc<QueryLogger>,
        blocklist: Arc<dyn BlocklistMatcher>,
        resolver: Arc<dyn DnsResolver>, // Changed to trait object
    ) -> Self {
        let cache = Cache::builder().max_capacity(config.cache.capacity).build();

        Self {
            config,
            stats,
            logger,
            blocklist: Arc::new(tokio::sync::RwLock::new(blocklist)),
            resolver,
            cache,
        }
    }

    pub async fn update_blocklist(&self, new_blocklist: Arc<dyn BlocklistMatcher>) {
        info!("Updating active blocklist...");
        let mut lock = self.blocklist.write().await;
        *lock = new_blocklist;
        info!("Active blocklist updated.");
    }
}

#[async_trait::async_trait]
impl RequestHandler for DnsHandler {
    async fn handle_request<R: ResponseHandler>(
        &self,
        request: &Request,
        mut response_handle: R,
    ) -> ResponseInfo {
        let start = Instant::now();
        self.stats.inc_queries();

        // let builder = MessageResponseBuilder::from_message_request(request); // Created locally to avoid move errors
        let mut header = hickory_server::proto::op::Header::response_from_request(request.header());
        header.set_authoritative(false); // We are a proxy/recursive resolver mostly

        // Fail fast for no queries
        // request.query() returns &LowerQuery which is a wrapper around a single query.
        // It doesn't support is_empty(). Assuming valid query if we got this far or API guarantees it.

        // request.query() returns &LowerQuery. LowerQuery wrapper.

        let query = request.query();
        let name = query.name();
        let name_str = name.to_string().trim_end_matches('.').to_string(); // Normalize
        let qtype = query.query_type();

        // response.add_query(query.clone()); // LowerQuery -> Query?
        // Builder handles this. Note request.query() might not implement Clone easily or is different type.

        // 1. Check Blocklist
        // Acquire read lock (very fast, low contention usually)
        let source_id = {
            let lock = self.blocklist.read().await;
            lock.check(&name_str)
        };

        if let Some(source_id) = source_id {
            self.stats.inc_blocked_by_source(source_id);

            // Return 0.0.0.0 for A records, :: for AAAA
            let mut records = Vec::new();
            match qtype {
                RecordType::A => {
                    let rdata = RData::A("0.0.0.0".parse().unwrap());
                    let record = Record::from_rdata(query.name().into(), 60, rdata);
                    records.push(record);
                }
                RecordType::AAAA => {
                    let rdata = RData::AAAA("::".parse().unwrap());
                    let record = Record::from_rdata(query.name().into(), 60, rdata);
                    records.push(record);
                }
                _ => {
                    header.set_response_code(ResponseCode::NXDomain);
                }
            }

            let builder = MessageResponseBuilder::from_message_request(request);
            let response = builder.build(header, records.iter(), &[], &[], &[]);
            let info = response_handle
                .send_response(response)
                .await
                .expect("Failed to send response");

            self.logger
                .log(LogEntry {
                    client_ip: request.src().to_string(),
                    domain: name_str.clone(),
                    query_type: qtype.to_string(),
                    action: LogAction::Blocked,
                    source_id: Some(source_id),
                    latency_ms: start.elapsed().as_millis() as u64,
                })
                .await;

            return info;
        }

        // 2. Check Cache
        let cache_key = (name_str.clone(), qtype);
        if let Some((records, valid_until, stale_until)) = self.cache.get(&cache_key).await {
            let now = Instant::now();
            if now < valid_until {
                // HIT & VALID
                self.stats.inc_cache_hit();
                // Send response
                let builder = MessageResponseBuilder::from_message_request(request);
                let response = builder.build(header, records.iter(), &[], &[], &[]);
                let info = response_handle
                    .send_response(response)
                    .await
                    .expect("Failed to send response");

                self.logger
                    .log(LogEntry {
                        client_ip: request.src().to_string(),
                        domain: name_str,
                        query_type: qtype.to_string(),
                        action: LogAction::Cached,
                        source_id: None,
                        latency_ms: start.elapsed().as_millis() as u64,
                    })
                    .await;

                return info;
            } else if now < stale_until {
                // SWR (Stale While Revalidate)
                // Serve stale immediately
                self.stats.inc_cache_hit(); // Count as hit

                let builder = MessageResponseBuilder::from_message_request(request);
                let response = builder.build(header, records.iter(), &[], &[], &[]);
                let info = response_handle
                    .send_response(response)
                    .await
                    .expect("Failed to send response");

                // Trigger background refresh
                let _resolver = self.resolver.clone();
                let _cache = self.cache.clone();
                let _q_name = name_str.clone();
                // let q_type = qtype; // unused

                tokio::spawn(async move {
                    // Re-resolve (Placeholder)
                });

                return info;
            }
        }

        // 3. Resolve Upstream
        match self.resolver.resolve(&name_str, qtype).await {
            Ok(records) => {
                // let records = lookup.records().to_vec(); // Now returns Vec<Record> directly

                // Cache it
                if !records.is_empty() {
                    // Use MIN TTL of records
                    let min_ttl = records.iter().map(|r| r.ttl()).min().unwrap_or(60);
                    let valid_until = Instant::now() + Duration::from_secs(min_ttl as u64);
                    let stale_until =
                        valid_until + Duration::from_secs(self.config.cache.grace_period_sec);

                    self.cache
                        .insert(cache_key, (records.clone(), valid_until, stale_until))
                        .await;
                }

                let builder = MessageResponseBuilder::from_message_request(request);
                let response = builder.build(header, records.iter(), &[], &[], &[]);
                let info = response_handle
                    .send_response(response)
                    .await
                    .expect("Failed to send response");

                self.logger
                    .log(LogEntry {
                        client_ip: request.src().to_string(),
                        domain: name_str,
                        query_type: qtype.to_string(),
                        action: LogAction::Forwarded,
                        source_id: None,
                        latency_ms: start.elapsed().as_millis() as u64,
                    })
                    .await;

                info
            }
            Err(e) => {
                error!("Upstream resolution failed for {}: {}", name_str, e);
                // ServFail
                header.set_response_code(ResponseCode::ServFail);
                let builder = MessageResponseBuilder::from_message_request(request);
                let response = builder.build(header, &[], &[], &[], &[]);
                response_handle
                    .send_response(response)
                    .await
                    .expect("Failed to send response")
            }
        }
    }
}
