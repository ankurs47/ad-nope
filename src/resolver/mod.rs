pub mod parallel;
pub mod round_robin;
pub mod sequential;
pub mod single;
pub mod types;

use crate::config::Config;
use crate::stats::StatsCollector;
use anyhow::{Context, Result};
use hickory_resolver::config::{NameServerConfig, ResolverConfig, ResolverOpts};
use hickory_resolver::lookup_ip::LookupIp;
use hickory_resolver::name_server::TokioConnectionProvider;
use hickory_resolver::proto::xfer::Protocol;
use hickory_resolver::Resolver;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{error, info};
use url::Url;

use self::parallel::ParallelResolver;
use self::round_robin::RoundRobinResolver;
use self::sequential::SequentialResolver;
use self::single::SingleResolver;
pub use self::types::{DnsResolver, Upstream};

pub async fn create_resolver(
    config: Config,
    stats: Arc<StatsCollector>,
) -> Result<Arc<dyn DnsResolver>> {
    let mut upstreams = Vec::new();

    if config.upstream_servers.is_empty() {
        upstreams.push(create_default_upstream(&config).await?);
    } else {
        match create_configured_upstreams(&config).await {
            Ok(configured) => upstreams.extend(configured),
            Err(e) => {
                error!("Failed to create configured upstreams: {}", e);
                // Fallback to default? Or fail?
                // For now, if no upstreams were created successfully (and we had config), we might want to fail or fallback.
                // But create_configured_upstreams returns a vector of valid upstreams.
            }
        }
    }

    if upstreams.is_empty() {
        return Err(anyhow::anyhow!("No valid upstreams available"));
    }

    if upstreams.len() == 1 {
        let upstream = upstreams.pop().unwrap();
        return Ok(Arc::new(SingleResolver { upstream, stats }) as Arc<dyn DnsResolver>);
    }

    match config.resolution_policy.as_str() {
        "sequential" => {
            Ok(Arc::new(SequentialResolver { upstreams, stats }) as Arc<dyn DnsResolver>)
        }
        "round-robin" => {
            Ok(Arc::new(RoundRobinResolver::new(upstreams, stats)) as Arc<dyn DnsResolver>)
        }
        "parallel" => Ok(Arc::new(ParallelResolver { upstreams, stats }) as Arc<dyn DnsResolver>),
        _ => {
            info!(
                "Unknown resolution policy '{}', defaulting to parallel",
                config.resolution_policy
            );
            Ok(Arc::new(ParallelResolver { upstreams, stats }) as Arc<dyn DnsResolver>)
        }
    }
}

async fn create_default_upstream(config: &Config) -> Result<Upstream> {
    info!("No upstream servers configured, using Google DNS (H3) default.");
    let url = "h3://dns.google/dns-query";
    match create_single_upstream(config, url, 0).await? {
        Some(u) => Ok(u),
        None => Err(anyhow::anyhow!("Failed to create default upstream")),
    }
}

async fn create_configured_upstreams(config: &Config) -> Result<Vec<Upstream>> {
    let mut upstreams = Vec::new();
    for (idx, upstream_url) in config.upstream_servers.iter().enumerate() {
        match create_single_upstream(config, upstream_url, idx).await {
            Ok(Some(upstream)) => upstreams.push(upstream),
            Ok(None) => {
                // Warning already logged in create_single_upstream
            }
            Err(e) => {
                error!("Critical error creating upstream {}: {}", upstream_url, e);
            }
        }
    }
    Ok(upstreams)
}

async fn create_single_upstream(
    config: &Config,
    upstream_url: &str,
    idx: usize,
) -> Result<Option<Upstream>> {
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

    let socket_addr = match bootstrap_host(config, host_str, port).await {
        Ok(addr) => addr,
        Err(e) => {
            error!("Failed to bootstrap {}: {}", host_str, e);
            return Ok(None);
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

    let resolver =
        Resolver::builder_with_config(resolver_config, TokioConnectionProvider::default())
            .with_options(opts)
            .build();

    info!(
        "Added upstream: [{}] {} ({})",
        idx, upstream_url, socket_addr
    );

    Ok(Some(Upstream {
        url: upstream_url.to_string(),
        resolver,
    }))
}

async fn bootstrap_host(config: &Config, host: &str, port: u16) -> Result<SocketAddr> {
    if let Ok(addr) = host.parse() {
        return Ok(SocketAddr::new(addr, port));
    }

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

    let mut builder =
        Resolver::builder_with_config(bootstrap_config, TokioConnectionProvider::default());
    *builder.options_mut() = ResolverOpts::default();
    let bootstrap = builder.build();

    info!("Bootstrapping upstream host: {}", host);
    let lookup: LookupIp = bootstrap.lookup_ip(host).await?;
    let ip = lookup
        .into_iter()
        .next()
        .context("No IP found for bootstrap host")?;

    Ok(SocketAddr::new(ip, port))
}
