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
    ) -> Result<Vec<hickory_resolver::proto::rr::Record>>;
}

pub struct UpstreamResolver {
    resolver: TokioResolver,
    stats: Arc<StatsCollector>,
}

impl UpstreamResolver {
    pub async fn new(config: Config, stats: Arc<StatsCollector>) -> Result<Self> {
        let mut resolver_config = ResolverConfig::new();

        // Custom Upstreams
        if config.upstream_servers.is_empty() {
            info!("No upstream servers configured, using Cloudflare/Google default.");
            resolver_config = ResolverConfig::cloudflare_tls();
        } else {
            for (idx, upstream_url) in config.upstream_servers.iter().enumerate() {
                // Parse URL: udp://1.1.1.1:53 or https://dns.google/dns-query
                // NOTE: Hickory Config requires IP addresses for NameServerConfig usually.
                // If the user configured a hostname (e.g. tls://dns.google), we must resolve it first using Bootstrap DNS.

                let url = Url::parse(upstream_url).context("Failed to parse upstream URL")?;
                let port = url.port().unwrap_or(53);
                let host_str = url.host_str().unwrap_or("0.0.0.0");

                // Resolve IP if needed
                let socket_addr: SocketAddr = if let Ok(addr) = host_str.parse() {
                    SocketAddr::new(addr, port)
                } else {
                    // Start a temporary bootstrap resolver
                    let bootstrap_config = if config.bootstrap_dns.is_empty() {
                        ResolverConfig::google()
                    } else {
                        // Build config from bootstrap IPs
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

                let protocol = match url.scheme() {
                    "udp" => Protocol::Udp,
                    "tcp" => Protocol::Tcp,
                    "tls" => Protocol::Tls,
                    "https" => Protocol::Https,
                    "quic" => Protocol::Quic,
                    "h3" => Protocol::H3,
                    _ => Protocol::Udp,
                };

                let mut ns_cfg = NameServerConfig::new(socket_addr, protocol);

                // For TLS/HTTPS/QUIC/H3, we need the tls_dns_name (SNI)
                if matches!(
                    protocol,
                    Protocol::Tls | Protocol::Https | Protocol::Quic | Protocol::H3
                ) {
                    ns_cfg.tls_dns_name = Some(host_str.to_string());
                }

                resolver_config.add_name_server(ns_cfg);
                info!(
                    "Added upstream: [{}] {} ({})",
                    idx, upstream_url, socket_addr
                );
            }
        }

        let mut opts = ResolverOpts::default();
        opts.cache_size = 0; // We define our own cache layer

        let mut builder =
            Resolver::builder_with_config(resolver_config, TokioConnectionProvider::default());
        *builder.options_mut() = opts;
        let resolver = builder.build();
        Ok(Self { resolver, stats })
    }

    // Moving logic to trait impl
}

#[async_trait::async_trait]
impl DnsResolver for UpstreamResolver {
    async fn resolve(
        &self,
        name: &str,
        query_type: hickory_resolver::proto::rr::RecordType,
    ) -> Result<Vec<hickory_resolver::proto::rr::Record>> {
        let start = Instant::now();
        let res = self.resolver.lookup(name, query_type).await;
        let latency = start.elapsed().as_millis() as u64;

        self.stats.record_upstream_latency(0, latency);

        match res {
            Ok(lookup) => Ok(lookup.records().to_vec()),
            Err(e) => Err(anyhow::anyhow!("Resolution failed: {}", e)),
        }
    }
}
