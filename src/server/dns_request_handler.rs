//! DNS Request Handler implementation.
//!
//! This module contains the core logic for handling incoming DNS requests.
//! It orchestrates:
//! 1. Query parsing
//! 2. Local record lookup
//! 3. Blocklist checking
//! 4. Caching
//! 5. Upstream resolution

use crate::config::Config;
use crate::engine::{AppState, BlocklistMatcher};
use crate::logger::{QueryLogAction, QueryLogEntry, QueryLogger};
use crate::resolver::DnsResolver;
use crate::stats::StatsCollector;
use anyhow::Result;
use arc_swap::ArcSwap;
use hickory_server::authority::MessageResponseBuilder;
use hickory_server::proto::op::ResponseCode;
use hickory_server::proto::rr::Name;
use hickory_server::proto::rr::{
    rdata::{A, AAAA},
    RData, Record, RecordType,
};
use hickory_server::server::{Request, RequestHandler, ResponseHandler, ResponseInfo};
use moka::future::Cache;
use rustc_hash::FxHashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tracing::{error, info};

use super::types::{LogContext, QueryContext};

/// Handles incoming DNS queries by coordinating checking, caching, and resolution.
#[derive(Clone)]
pub struct DnsHandler {
    /// Application configuration.
    config: Config,
    /// Metrics collector.
    stats: Arc<StatsCollector>,
    /// Query logger.
    logger: Arc<QueryLogger>,
    /// Application state (e.g., pause status).
    app_state: AppState,
    /// Hot-swappable blocklist matcher.
    blocklist: Arc<ArcSwap<Arc<dyn BlocklistMatcher>>>,
    /// Upstream resolver implementation.
    resolver: Arc<dyn DnsResolver>,
    /// In-memory response cache.
    #[allow(clippy::type_complexity)]
    cache: Cache<(Arc<str>, RecordType), (Arc<Vec<Record>>, Instant, Instant)>,
    /// Static local records loaded from config.
    local_records: Arc<FxHashMap<String, Arc<Vec<Record>>>>,
}

impl DnsHandler {
    /// Creates a new `DnsHandler`.
    pub fn new(
        config: Config,
        stats: Arc<StatsCollector>,
        logger: Arc<QueryLogger>,
        blocklist: Arc<dyn BlocklistMatcher>,
        resolver: Arc<dyn DnsResolver>,
        app_state: AppState,
    ) -> Self {
        let cache = Cache::builder().max_capacity(config.cache.capacity).build();

        // Pre-calculate local records for faster lookup
        // Note: Currently supports A and AAAA records only
        let mut local_map = FxHashMap::default();
        let local_ttl = config.cache.min_ttl;
        for (domain, ip) in &config.local_records {
            let mut records = Vec::new();
            if let Ok(name) = Name::from_str(domain) {
                match ip {
                    std::net::IpAddr::V4(ipv4) => {
                        let rdata = RData::A(A(*ipv4));
                        let record = Record::from_rdata(name, local_ttl, rdata);
                        records.push(record);
                    }
                    std::net::IpAddr::V6(ipv6) => {
                        let rdata = RData::AAAA(AAAA(*ipv6));
                        let record = Record::from_rdata(name, local_ttl, rdata);
                        records.push(record);
                    }
                }
            }
            if !records.is_empty() {
                local_map.insert(domain.clone(), Arc::new(records));
            }
        }

        Self {
            config,
            stats,
            logger,
            app_state,
            blocklist: Arc::new(ArcSwap::new(Arc::new(blocklist))),
            resolver,
            cache,
            local_records: Arc::new(local_map),
        }
    }

    /// Resolves a query using the upstream resolver and caches the result.
    async fn resolve_and_cache(
        &self,
        name: &str,
        qtype: RecordType,
    ) -> Result<(Arc<Vec<Record>>, String)> {
        let (records, upstream) = self.resolver.resolve(name, qtype).await?;
        let records = Arc::new(records);

        if !records.is_empty() {
            // Find the minimum TTL among all records
            #[allow(clippy::manual_map)]
            let record_min_ttl = records.iter().map(|r| r.ttl()).min().unwrap_or(300);

            // Enforce configured minimum TTL for caching purposes
            let effective_ttl = std::cmp::max(record_min_ttl, self.config.cache.min_ttl);

            let valid_until = Instant::now() + Duration::from_secs(effective_ttl as u64);
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

    /// Atomically updates the active blocklist matcher.
    pub async fn update_blocklist(&self, new_blocklist: Arc<dyn BlocklistMatcher>) {
        info!("Updating active blocklist...");
        self.blocklist.store(Arc::new(new_blocklist));
        info!("Active blocklist updated.");
    }

    /// Extracts query information from the incoming request.
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

    /// Checks if a domain is blocked.
    fn check_blocklist(&self, name: &str) -> Option<u8> {
        self.blocklist.load().check(name)
    }

    /// Checks the cache for a response.
    ///
    /// Returns `Some((records, is_stale, ttl))` if found.
    async fn check_cache(
        &self,
        name: Arc<str>,
        qtype: RecordType,
    ) -> Option<(Arc<Vec<Record>>, bool, Option<u64>)> {
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

    /// Sends DNS records to the client and logs the request.
    async fn serve_records<R: ResponseHandler, F>(
        &self,
        request: &Request,
        mut response_handle: R,
        records: &[Record],
        query: QueryContext,
        log_factory: F,
    ) -> ResponseInfo
    where
        F: FnOnce() -> LogContext + Send,
    {
        let mut header = hickory_server::proto::op::Header::response_from_request(request.header());
        header.set_authoritative(false);
        let builder = MessageResponseBuilder::from_message_request(request);
        let response = builder.build(header, records.iter(), &[], &[], &[]);
        let info = response_handle
            .send_response(response)
            .await
            .expect("Failed to send response");

        if self.config.logging.enable {
            let log = log_factory();
            self.logger
                .log(QueryLogEntry {
                    client_ip: request.src().ip(),
                    domain: query.name.clone(),
                    query_type: query.qtype,
                    action: log.action,
                    source_id: log.source_id,
                    upstream: log.upstream.map(|s| s.into_owned()),
                    latency_ms: query.start.elapsed().as_millis() as u64,
                    ttl_remaining: log.ttl_remaining,
                })
                .await;
        }

        info
    }

    /// Handles a blocked request by returning 0.0.0.0, ::, or NXDOMAIN.
    async fn serve_blocked<R: ResponseHandler>(
        &self,
        request: &Request,
        mut response_handle: R,
        query: QueryContext,
        source_id: u8,
    ) -> ResponseInfo {
        self.stats.inc_blocked_by_source(source_id);
        self.stats.inc_client_block(request.src().ip());
        self.stats.inc_domain_block(query.name.clone());

        let record_opt = match query.qtype {
            RecordType::A => {
                let rdata = RData::A("0.0.0.0".parse().unwrap());
                Some(Record::from_rdata(
                    Name::from_str(&query.name).unwrap_or_default(),
                    60,
                    rdata,
                ))
            }
            RecordType::AAAA => {
                let rdata = RData::AAAA("::".parse().unwrap());
                Some(Record::from_rdata(
                    Name::from_str(&query.name).unwrap_or_default(),
                    60,
                    rdata,
                ))
            }
            _ => None,
        };

        if let Some(record) = record_opt {
            let records = std::slice::from_ref(&record);
            self.serve_records(request, response_handle, records, query, || LogContext {
                action: QueryLogAction::Blocked,
                source_id: Some(source_id),
                upstream: None,
                ttl_remaining: None,
            })
            .await
        } else {
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

            if self.config.logging.enable {
                self.logger
                    .log(QueryLogEntry {
                        client_ip: request.src().ip(),
                        domain: query.name.clone(),
                        query_type: query.qtype,
                        action: QueryLogAction::Blocked,
                        source_id: Some(source_id),
                        upstream: None,
                        latency_ms: query.start.elapsed().as_millis() as u64,
                        ttl_remaining: None,
                    })
                    .await;
            }
            info
        }
    }

    async fn handle_local_records<R: ResponseHandler>(
        &self,
        request: &Request,
        response_handle: R,
        query: &QueryContext,
    ) -> Result<ResponseInfo, R> {
        if let Some(records) = self.local_records.get(&*query.name) {
            if !records.is_empty() && records[0].record_type() == query.qtype {
                Ok(self
                    .serve_records(
                        request,
                        response_handle,
                        records, // Pass slice
                        query.clone(),
                        || LogContext {
                            action: QueryLogAction::Local,
                            source_id: None,
                            upstream: Some(std::borrow::Cow::Borrowed("local-config")),
                            ttl_remaining: None,
                        },
                    )
                    .await)
            } else {
                Err(response_handle)
            }
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
        // 1. Check Pause State
        if !self.app_state.is_blocking_active() {
            // Blocking is paused, behave as if no blocklist match found.
            return Err(response_handle);
        }

        // 2. Check Blocklist
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
                    &records, // Pass slice
                    query.clone(),
                    || LogContext {
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
                self.serve_records(request, response_handle, &records, query, move || {
                    LogContext {
                        action: QueryLogAction::Forwarded,
                        source_id: None,
                        upstream: Some(std::borrow::Cow::Owned(upstream_name)),
                        ttl_remaining: None,
                    }
                })
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

                if self.config.logging.enable {
                    self.logger
                        .log(QueryLogEntry {
                            client_ip: request.src().ip(),
                            domain: query.name.clone(),
                            query_type: query.qtype,
                            action: QueryLogAction::Forwarded,
                            source_id: None,
                            upstream: Some(format!("error: {}", e)),
                            latency_ms: query.start.elapsed().as_millis() as u64,
                            ttl_remaining: None,
                        })
                        .await;
                }

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
        self.stats.inc_client_query(request.src().ip());

        let query_ctx = self.get_query_info(request).await;
        self.stats.inc_domain_query(query_ctx.name.clone());

        // Step 1: Check Local Records
        let response_handle = match self
            .handle_local_records(request, response_handle, &query_ctx)
            .await
        {
            Ok(info) => return info,
            Err(handle) => handle,
        };

        // Step 2: Check Blocklists (if blocking is active)
        let response_handle = match self
            .handle_blocked_request(request, response_handle, &query_ctx)
            .await
        {
            Ok(info) => return info,
            Err(handle) => handle,
        };

        // Step 3: Check Cache
        let response_handle = match self
            .handle_cached_response(request, response_handle, &query_ctx)
            .await
        {
            Ok(info) => return info,
            Err(handle) => handle,
        };

        // Step 4: Resolve Upstream
        self.resolve_and_serve(request, response_handle, query_ctx)
            .await
    }
}
