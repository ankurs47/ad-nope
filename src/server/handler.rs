use crate::config::Config;
use crate::engine::BlocklistMatcher;
use crate::logger::{QueryLogAction, QueryLogEntry, QueryLogger};
use crate::resolver::DnsResolver;
use crate::stats::StatsCollector;
use anyhow::Result;
use arc_swap::ArcSwap;
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
    // We use Arc<ArcSwap<Arc<dyn ...>>> because Arc<dyn Trait> is a fat pointer
    // and ArcSwap only supports Sized types (thin pointers) for the inner T unless using feature.
    // Double Arc is a simple workaround.
    blocklist: Arc<ArcSwap<Arc<dyn BlocklistMatcher>>>,
    resolver: Arc<dyn DnsResolver>,
    // Cache: Key(Name, Type) -> (Records, ValidUntil, StaleUntil)
    #[allow(clippy::type_complexity)]
    cache: Cache<(Arc<str>, RecordType), (Vec<Record>, Instant, Instant)>,
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
            blocklist: Arc::new(ArcSwap::new(Arc::new(blocklist))),
            resolver,
            cache,
        }
    }

    async fn resolve_and_cache(
        &self,
        name: &str,
        qtype: RecordType,
    ) -> Result<(Vec<Record>, String)> {
        let (records, upstream) = self.resolver.resolve(name, qtype).await?;
        if !records.is_empty() {
            let min_ttl = records.iter().map(|r| r.ttl()).min().unwrap_or(300);
            let valid_until = Instant::now() + Duration::from_secs(min_ttl as u64);
            let stale_until = valid_until + Duration::from_secs(self.config.cache.grace_period_sec);
            self.cache
                .insert(
                    (Arc::from(name), qtype),
                    (records.clone(), valid_until, stale_until),
                )
                .await;
        }
        Ok((records, upstream))
    }

    pub async fn update_blocklist(&self, new_blocklist: Arc<dyn BlocklistMatcher>) {
        info!("Updating active blocklist...");
        self.blocklist.store(Arc::new(new_blocklist));
        info!("Active blocklist updated.");
    }

    async fn get_query_info(&self, request: &Request) -> QueryContext {
        let query = request.queries().first().expect("No query in request");
        let name = query.name();

        let mut name_str = name.to_string();
        if name_str.ends_with('.') {
            name_str.pop();
        }
        name_str.make_ascii_lowercase();

        QueryContext {
            name: name_str.into(),
            qtype: query.query_type(),
            start: Instant::now(),
        }
    }

    fn check_blocklist(&self, name: &str) -> Option<u8> {
        self.blocklist.load().check(name)
    }

    async fn check_cache(
        &self,
        name: Arc<str>,
        qtype: RecordType,
    ) -> Option<(Vec<Record>, bool, Option<u64>)> {
        if let Some((records, valid_until, stale_until)) = self.cache.get(&(name, qtype)).await {
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
                domain: query.name.to_string(),
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
                    domain: query.name.to_string(),
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
        if let Some(&ip_addr) = self.config.local_records.get(&*query.name) {
            let mut records = Vec::new();
            // IP is already parsed in config
            match (query.qtype, ip_addr) {
                (RecordType::A, std::net::IpAddr::V4(ipv4)) => {
                    let rdata = RData::A(A(ipv4));
                    let record = Record::from_rdata(request.queries()[0].name().into(), 300, rdata);
                    records.push(record);
                }
                (RecordType::AAAA, std::net::IpAddr::V6(ipv6)) => {
                    let rdata = RData::AAAA(AAAA(ipv6));
                    let record = Record::from_rdata(request.queries()[0].name().into(), 300, rdata);
                    records.push(record);
                }
                _ => {
                    // Mismatch type
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
        if let Some(source_id) = self.check_blocklist(&query.name) {
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
        if let Some((records, is_stale, ttl)) =
            self.check_cache(query.name.clone(), query.qtype).await
        {
            self.stats.inc_cache_hit();
            if is_stale {
                // Trigger background refresh
                let handler = self.clone();
                let q_name = query.name.clone();
                let q_type = query.qtype;

                tokio::spawn(async move {
                    if let Err(e) = handler.resolve_and_cache(&q_name, q_type).await {
                        error!("Background re-resolve failed for {}: {}", q_name, e);
                    }
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
        match self.resolve_and_cache(&query.name, query.qtype).await {
            Ok((records, upstream_name)) => {
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
                        domain: query.name.to_string(),
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
