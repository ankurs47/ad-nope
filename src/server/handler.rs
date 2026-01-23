use crate::config::Config;
use crate::engine::BlocklistMatcher;
use crate::logger::{QueryLogAction, QueryLogEntry, QueryLogger};
use crate::resolver::DnsResolver;
use crate::stats::StatsCollector;
use hickory_server::authority::MessageResponseBuilder;
use hickory_server::proto::op::ResponseCode;
use hickory_server::proto::rr::{
    rdata::{A, AAAA},
    RData, Record, RecordType,
};
use hickory_server::server::{Request, RequestHandler, ResponseHandler, ResponseInfo};
use moka::future::Cache;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tracing::{error, info};

use super::types::{LogContext, QueryContext};

#[derive(Clone)]
pub struct DnsHandler {
    config: Config,
    stats: Arc<StatsCollector>,
    logger: Arc<QueryLogger>,
    // Wrap in RwLock for hot-reloading
    blocklist: Arc<tokio::sync::RwLock<Arc<dyn BlocklistMatcher>>>,
    resolver: Arc<dyn DnsResolver>,
    // Cache: Key(Name, Type) -> (Records, ValidUntil, StaleUntil)
    cache: Cache<(String, RecordType), (Vec<Record>, Instant, Instant)>,
}

impl DnsHandler {
    pub fn new(
        config: Config,
        stats: Arc<StatsCollector>,
        logger: Arc<QueryLogger>,
        blocklist: Arc<dyn BlocklistMatcher>,
        resolver: Arc<dyn DnsResolver>,
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

    async fn get_query_info(&self, request: &Request) -> QueryContext {
        let query = request.queries().first().expect("No query in request");
        let name = query.name();
        let name_str = name.to_string().trim_end_matches('.').to_lowercase();
        QueryContext {
            name: name_str,
            qtype: query.query_type(),
            start: Instant::now(),
        }
    }

    async fn check_blocklist(&self, name: &str) -> Option<u8> {
        let lock = self.blocklist.read().await;
        lock.check(name)
    }

    async fn check_cache(
        &self,
        name: &str,
        qtype: RecordType,
    ) -> Option<(Vec<Record>, bool, Option<u64>)> {
        if let Some((records, valid_until, stale_until)) =
            self.cache.get(&(name.to_string(), qtype)).await
        {
            let now = Instant::now();
            let ttl = if valid_until > now {
                Some(valid_until.duration_since(now).as_secs())
            } else {
                Some(0)
            };

            if now < valid_until {
                return Some((records, false, ttl));
            } else if now < stale_until {
                return Some((records, true, ttl));
            }
        }
        None
    }

    async fn serve_records<R: ResponseHandler>(
        &self,
        request: &Request,
        mut response_handle: R,
        records: Vec<Record>,
        query: QueryContext,
        log: LogContext,
    ) -> ResponseInfo {
        let mut header = hickory_server::proto::op::Header::response_from_request(request.header());
        header.set_authoritative(false);
        let builder = MessageResponseBuilder::from_message_request(request);
        let response = builder.build(header, records.iter(), &[], &[], &[]);
        let info = response_handle
            .send_response(response)
            .await
            .expect("Failed to send response");

        self.logger
            .log(QueryLogEntry {
                client_ip: request.src().to_string(),
                domain: query.name,
                query_type: query.qtype.to_string(),
                action: log.action,
                source_id: log.source_id,
                upstream: log.upstream,
                latency_ms: query.start.elapsed().as_millis() as u64,
                ttl_remaining: log.ttl_remaining,
            })
            .await;

        info
    }

    async fn serve_blocked<R: ResponseHandler>(
        &self,
        request: &Request,
        mut response_handle: R,
        query: QueryContext,
        source_id: u8,
    ) -> ResponseInfo {
        self.stats.inc_blocked_by_source(source_id);

        let mut records = Vec::new();
        match query.qtype {
            RecordType::A => {
                let rdata = RData::A("0.0.0.0".parse().unwrap());
                let record = Record::from_rdata(request.queries()[0].name().into(), 60, rdata);
                records.push(record);
            }
            RecordType::AAAA => {
                let rdata = RData::AAAA("::".parse().unwrap());
                let record = Record::from_rdata(request.queries()[0].name().into(), 60, rdata);
                records.push(record);
            }
            _ => {}
        }

        if records.is_empty() {
            let mut header =
                hickory_server::proto::op::Header::response_from_request(request.header());
            header.set_authoritative(false);
            header.set_response_code(ResponseCode::NXDomain);
            let builder = MessageResponseBuilder::from_message_request(request);
            let response = builder.build(header, &[], &[], &[], &[]);
            let info = response_handle
                .send_response(response)
                .await
                .expect("Failed to send response");

            self.logger
                .log(QueryLogEntry {
                    client_ip: request.src().to_string(),
                    domain: query.name.clone(),
                    query_type: query.qtype.to_string(),
                    action: QueryLogAction::Blocked,
                    source_id: Some(source_id),
                    upstream: None,
                    latency_ms: query.start.elapsed().as_millis() as u64,
                    ttl_remaining: None,
                })
                .await;
            return info;
        }

        self.serve_records(
            request,
            response_handle,
            records,
            query,
            LogContext {
                action: QueryLogAction::Blocked,
                source_id: Some(source_id),
                upstream: None,
                ttl_remaining: None,
            },
        )
        .await
    }
    async fn handle_local_records<R: ResponseHandler>(
        &self,
        request: &Request,
        response_handle: R,
        query: &QueryContext,
    ) -> Result<ResponseInfo, R> {
        if let Some(ip_str) = self.config.local_records.get(&query.name) {
            let mut records = Vec::new();
            if let Ok(ip_addr) = ip_str.parse::<std::net::IpAddr>() {
                match (query.qtype, ip_addr) {
                    (RecordType::A, std::net::IpAddr::V4(ipv4)) => {
                        let rdata = RData::A(A(ipv4));
                        let record =
                            Record::from_rdata(request.queries()[0].name().into(), 300, rdata);
                        records.push(record);
                    }
                    (RecordType::AAAA, std::net::IpAddr::V6(ipv6)) => {
                        let rdata = RData::AAAA(AAAA(ipv6));
                        let record =
                            Record::from_rdata(request.queries()[0].name().into(), 300, rdata);
                        records.push(record);
                    }
                    _ => {
                        // Mismatch type (e.g. AAAA query for v4 IP) or other types.
                        // Return NODATA (empty records).
                    }
                }
            }

            Ok(self
                .serve_records(
                    request,
                    response_handle,
                    records,
                    query.clone(),
                    LogContext {
                        action: QueryLogAction::Local,
                        source_id: None,
                        upstream: Some("local-config".to_string()),
                        ttl_remaining: None,
                    },
                )
                .await)
        } else {
            Err(response_handle)
        }
    }

    async fn handle_blocked_request<R: ResponseHandler>(
        &self,
        request: &Request,
        response_handle: R,
        query: &QueryContext,
    ) -> Result<ResponseInfo, R> {
        // 1. Check Blocklist
        if let Some(source_id) = self.check_blocklist(&query.name).await {
            Ok(self
                .serve_blocked(request, response_handle, query.clone(), source_id)
                .await)
        } else {
            Err(response_handle)
        }
    }

    async fn handle_cached_response<R: ResponseHandler>(
        &self,
        request: &Request,
        response_handle: R,
        query: &QueryContext,
    ) -> Result<ResponseInfo, R> {
        // 2. Check Cache
        if let Some((records, is_stale, ttl)) = self.check_cache(&query.name, query.qtype).await {
            self.stats.inc_cache_hit();
            if is_stale {
                // Trigger background refresh
                let _resolver = self.resolver.clone();
                let _cache = self.cache.clone();
                let _q_name = query.name.clone();
                tokio::spawn(async move {
                    // Re-resolve (Placeholder)
                });
            }

            Ok(self
                .serve_records(
                    request,
                    response_handle,
                    records,
                    query.clone(),
                    LogContext {
                        action: QueryLogAction::Cached,
                        source_id: None,
                        upstream: None,
                        ttl_remaining: ttl,
                    },
                )
                .await)
        } else {
            Err(response_handle)
        }
    }

    async fn resolve_and_serve<R: ResponseHandler>(
        &self,
        request: &Request,
        mut response_handle: R,
        query: QueryContext,
    ) -> ResponseInfo {
        // 3. Resolve Upstream
        match self.resolver.resolve(&query.name, query.qtype).await {
            Ok((records, upstream_name)) => {
                if !records.is_empty() {
                    let min_ttl = records.iter().map(|r| r.ttl()).min().unwrap_or(60);
                    let valid_until = Instant::now() + Duration::from_secs(min_ttl as u64);
                    let stale_until =
                        valid_until + Duration::from_secs(self.config.cache.grace_period_sec);
                    self.cache
                        .insert(
                            (query.name.clone(), query.qtype),
                            (records.clone(), valid_until, stale_until),
                        )
                        .await;
                }

                self.serve_records(
                    request,
                    response_handle,
                    records,
                    query,
                    LogContext {
                        action: QueryLogAction::Forwarded,
                        source_id: None,
                        upstream: Some(upstream_name),
                        ttl_remaining: None,
                    },
                )
                .await
            }
            Err(e) => {
                error!("Upstream resolution failed for {}: {}", query.name, e);
                let mut header =
                    hickory_server::proto::op::Header::response_from_request(request.header());
                header.set_authoritative(false);
                header.set_response_code(ResponseCode::ServFail);

                let builder = MessageResponseBuilder::from_message_request(request);
                let response = builder.build(header, &[], &[], &[], &[]);
                let info = response_handle
                    .send_response(response)
                    .await
                    .expect("Failed to send response");

                self.logger
                    .log(QueryLogEntry {
                        client_ip: request.src().to_string(),
                        domain: query.name,
                        query_type: query.qtype.to_string(),
                        action: QueryLogAction::Forwarded,
                        source_id: None,
                        upstream: Some(format!("error: {}", e)),
                        latency_ms: query.start.elapsed().as_millis() as u64,
                        ttl_remaining: None,
                    })
                    .await;

                info
            }
        }
    }
}

#[async_trait::async_trait]
impl RequestHandler for DnsHandler {
    async fn handle_request<R: ResponseHandler>(
        &self,
        request: &Request,
        response_handle: R,
    ) -> ResponseInfo {
        self.stats.inc_queries();

        let query_ctx = self.get_query_info(request).await;

        let response_handle = match self
            .handle_local_records(request, response_handle, &query_ctx)
            .await
        {
            Ok(info) => return info,
            Err(handle) => handle,
        };

        let response_handle = match self
            .handle_blocked_request(request, response_handle, &query_ctx)
            .await
        {
            Ok(info) => return info,
            Err(handle) => handle,
        };

        let response_handle = match self
            .handle_cached_response(request, response_handle, &query_ctx)
            .await
        {
            Ok(info) => return info,
            Err(handle) => handle,
        };

        self.resolve_and_serve(request, response_handle, query_ctx)
            .await
    }
}
