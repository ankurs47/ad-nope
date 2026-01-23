use crate::config::Config;
use crate::stats::StatsCollector;
use anyhow::{Context, Result};
use hickory_resolver::config::{NameServerConfig, ResolverConfig, ResolverOpts};
use hickory_resolver::lookup_ip::LookupIp;
use hickory_resolver::name_server::TokioConnectionProvider;
use hickory_resolver::proto::xfer::Protocol;
use hickory_resolver::{Resolver, TokioResolver};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::time::Instant;
use tracing::{error, info};
use url::Url;

/// Abstract upstream resolver for mocking and switching implementations.
#[async_trait::async_trait]
pub trait DnsResolver: Send + Sync {
    async fn resolve(
        &self,
        name: &str,
        query_type: hickory_resolver::proto::rr::RecordType,
    ) -> Result<(Vec<hickory_resolver::proto::rr::Record>, String)>;
}

pub struct Upstream {
    pub url: String,
    pub resolver: TokioResolver,
}

pub struct UpstreamResolver {
    upstreams: Vec<Upstream>,
    parallel: bool,
    stats: Arc<StatsCollector>,
}

impl UpstreamResolver {
    pub async fn new(config: Config, stats: Arc<StatsCollector>) -> Result<Self> {
        let mut upstreams = Vec::new();

        // Custom Upstreams
        if config.upstream_servers.is_empty() {
            info!("No upstream servers configured, using Cloudflare/Google default.");
            let mut opts = ResolverOpts::default();
            opts.cache_size = 0;
            opts.timeout = std::time::Duration::from_millis(config.upstream_timeout_ms);

            let resolver = Resolver::builder_with_config(
                ResolverConfig::cloudflare_tls(),
                TokioConnectionProvider::default(),
            )
            .with_options(opts)
            .build();

            upstreams.push(Upstream {
                url: "cloudflare-tls".to_string(),
                resolver,
            });
        } else {
            for (idx, upstream_url) in config.upstream_servers.iter().enumerate() {
                // ... same logic to build ns_cfg ...
                let url = Url::parse(upstream_url).context("Failed to parse upstream URL")?;

                let protocol = match url.scheme() {
                    "udp" => Protocol::Udp,
                    "tcp" => Protocol::Tcp,
                    "tls" => Protocol::Tls,
                    "https" => Protocol::Https,
                    "quic" => Protocol::Quic,
                    "h3" => Protocol::H3,
                    _ => Protocol::Udp,
                };

                let port = url.port().unwrap_or(match protocol {
                    Protocol::Udp | Protocol::Tcp => 53,
                    Protocol::Tls => 853,
                    Protocol::Https | Protocol::H3 => 443,
                    Protocol::Quic => 853,
                    _ => 53,
                });

                let host_str = url.host_str().unwrap_or("0.0.0.0");

                let socket_addr: SocketAddr = if let Ok(addr) = host_str.parse() {
                    SocketAddr::new(addr, port)
                } else {
                    let bootstrap_config = if config.bootstrap_dns.is_empty() {
                        ResolverConfig::google()
                    } else {
                        let mut cfg = ResolverConfig::new();
                        for ip in &config.bootstrap_dns {
                            if let Ok(sa) = ip.parse::<SocketAddr>() {
                                cfg.add_name_server(NameServerConfig::new(sa, Protocol::Udp));
                            }
                        }
                        cfg
                    };
                    let mut builder = Resolver::builder_with_config(
                        bootstrap_config,
                        TokioConnectionProvider::default(),
                    );
                    *builder.options_mut() = ResolverOpts::default();
                    let bootstrap = builder.build();
                    info!("Bootstrapping upstream host: {}", host_str);
                    match bootstrap.lookup_ip(host_str).await {
                        Ok(lookup) => {
                            let lookup: LookupIp = lookup;
                            let ip: std::net::IpAddr = lookup
                                .into_iter()
                                .next()
                                .context("No IP found for bootstrap host")?;
                            SocketAddr::new(ip, port)
                        }
                        Err(e) => {
                            error!("Failed to bootstrap {}: {}", host_str, e);
                            continue;
                        }
                    }
                };

                let mut ns_cfg = NameServerConfig::new(socket_addr, protocol);
                if matches!(
                    protocol,
                    Protocol::Tls | Protocol::Https | Protocol::Quic | Protocol::H3
                ) {
                    ns_cfg.tls_dns_name = Some(host_str.to_string());
                }

                let mut resolver_config = ResolverConfig::new();
                resolver_config.add_name_server(ns_cfg);

                let mut opts = ResolverOpts::default();
                opts.cache_size = 0;
                opts.timeout = std::time::Duration::from_millis(config.upstream_timeout_ms);

                let resolver = Resolver::builder_with_config(
                    resolver_config,
                    TokioConnectionProvider::default(),
                )
                .with_options(opts)
                .build();

                upstreams.push(Upstream {
                    url: upstream_url.clone(),
                    resolver,
                });

                info!(
                    "Added upstream: [{}] {} ({})",
                    idx, upstream_url, socket_addr
                );
            }
        }

        Ok(Self {
            upstreams,
            parallel: config.parallel_queries,
            stats,
        })
    }
}

#[async_trait::async_trait]
impl DnsResolver for UpstreamResolver {
    async fn resolve(
        &self,
        name: &str,
        query_type: hickory_resolver::proto::rr::RecordType,
    ) -> Result<(Vec<hickory_resolver::proto::rr::Record>, String)> {
        let start = Instant::now();

        if self.parallel && self.upstreams.len() > 1 {
            let mut futures = Vec::new();
            for (idx, upstream) in self.upstreams.iter().enumerate() {
                let name = name.to_string();
                let f = async move {
                    let res = upstream.resolver.lookup(name, query_type).await;
                    (res, upstream.url.clone(), idx)
                };
                futures.push(Box::pin(f));
            }

            // We need to handle failures: if one fails, we should wait for others.
            // But select_all returns the first one that completes.
            let mut futures = futures;
            while !futures.is_empty() {
                let ((res, url, idx), _index, remaining) =
                    futures::future::select_all(futures).await;
                futures = remaining;

                match res {
                    Ok(lookup) => {
                        let latency = start.elapsed().as_millis() as u64;
                        self.stats.record_upstream_latency(idx, latency);
                        return Ok((lookup.records().to_vec(), url));
                    }
                    Err(e) => {
                        error!("Upstream {} failed for {}: {}", url, name, e);
                    }
                }
            }
            Err(anyhow::anyhow!("All upstreams failed for {}", name))
        } else {
            // Serial
            for (idx, upstream) in self.upstreams.iter().enumerate() {
                match upstream.resolver.lookup(name, query_type).await {
                    Ok(lookup) => {
                        let latency = start.elapsed().as_millis() as u64;
                        self.stats.record_upstream_latency(idx, latency);
                        return Ok((lookup.records().to_vec(), upstream.url.clone()));
                    }
                    Err(e) => {
                        error!("Upstream {} failed for {}: {}", upstream.url, name, e);
                        continue;
                    }
                }
            }
            Err(anyhow::anyhow!("All upstreams failed for {}", name))
        }
    }
}
