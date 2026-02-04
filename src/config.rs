use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,

    #[serde(default = "default_upstream_servers")]
    pub upstream_servers: Vec<String>,

    #[serde(default = "default_resolution_policy")]
    pub resolution_policy: String,

    #[serde(default = "default_upstream_timeout_ms")]
    pub upstream_timeout_ms: u64,

    #[serde(default = "default_bootstrap_dns")]
    pub bootstrap_dns: Vec<String>,

    #[serde(default = "default_blocklists")]
    pub blocklists: HashMap<String, String>,

    #[serde(default)]
    pub allowlist: Vec<String>,

    #[serde(default)]
    pub cache: CacheConfig,

    #[serde(default)]
    pub updates: UpdateConfig,

    #[serde(default)]
    pub logging: LoggingConfig,

    #[serde(default)]
    pub local_records: HashMap<String, std::net::IpAddr>,

    #[serde(default)]
    pub stats: StatsConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CacheConfig {
    #[serde(default = "default_cache_enable")]
    pub enable: bool,
    #[serde(default = "default_cache_capacity")]
    pub capacity: u64,
    #[serde(default = "default_grace_period")]
    pub grace_period_sec: u64,
    #[serde(default = "default_min_ttl")]
    pub min_ttl: u32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct UpdateConfig {
    #[serde(default = "default_update_interval")]
    pub interval_hours: u64,
    #[serde(default = "default_concurrent_downloads")]
    pub concurrent_downloads: usize,
}

fn default_min_ttl() -> u32 {
    300
}
#[derive(Debug, Deserialize, Clone)]
pub struct LoggingConfig {
    #[serde(default = "default_log_enable")]
    pub enable: bool,
    #[serde(default = "default_log_blocked")]
    pub log_blocked: bool,
    #[serde(default = "default_log_all_queries")]
    pub log_all_queries: bool,
    #[serde(default = "default_log_format")]
    pub format: String,
    #[serde(default = "default_log_target")]
    pub target: String,
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub syslog_addr: Option<String>,
    #[serde(default = "default_query_log_sinks")]
    pub query_log_sinks: Vec<String>,
    #[serde(default = "default_sqlite_path")]
    pub sqlite_path: String,
    #[serde(default = "default_sqlite_retention_hours")]
    pub sqlite_retention_hours: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct StatsConfig {
    #[serde(default = "default_stats_enable")]
    pub enable: bool,
    #[serde(default = "default_log_interval")]
    pub log_interval_seconds: u64,
}

// Defaults
fn default_host() -> String {
    "0.0.0.0".to_string()
}
fn default_port() -> u16 {
    5300
}
fn default_resolution_policy() -> String {
    "parallel".to_string()
}
fn default_upstream_timeout_ms() -> u64 {
    200
}
fn default_cache_enable() -> bool {
    true
}
fn default_cache_capacity() -> u64 {
    10000
}
fn default_grace_period() -> u64 {
    10
}
fn default_update_interval() -> u64 {
    24
}
fn default_concurrent_downloads() -> usize {
    4
}
fn default_log_enable() -> bool {
    true
}
fn default_log_blocked() -> bool {
    true
}
fn default_log_all_queries() -> bool {
    true
}
fn default_log_format() -> String {
    "text".to_string()
}
fn default_log_target() -> String {
    "console".to_string()
}
fn default_log_level() -> String {
    "info".to_string()
}
fn default_query_log_sinks() -> Vec<String> {
    vec!["console".to_string()]
}
fn default_sqlite_path() -> String {
    "ad-nope.db".to_string()
}
fn default_sqlite_retention_hours() -> u64 {
    168 // 7 days
}
fn default_stats_enable() -> bool {
    true
}
fn default_log_interval() -> u64 {
    300
}
fn default_blocklists() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert(
        "default".to_string(),
        "https://cdn.jsdelivr.net/gh/hagezi/dns-blocklists@latest/wildcard/pro-onlydomains.txt"
            .to_string(),
    );
    m
}
fn default_upstream_servers() -> Vec<String> {
    vec!["h3://dns.google/dns-query".to_string()]
}
fn default_bootstrap_dns() -> Vec<String> {
    vec!["8.8.8.8:53".to_string()]
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            upstream_servers: default_upstream_servers(),
            resolution_policy: default_resolution_policy(),
            upstream_timeout_ms: default_upstream_timeout_ms(),
            bootstrap_dns: default_bootstrap_dns(),
            blocklists: default_blocklists(),
            allowlist: vec![],
            cache: CacheConfig::default(),
            updates: UpdateConfig::default(),
            logging: LoggingConfig::default(),
            local_records: HashMap::new(),
            stats: StatsConfig::default(),
        }
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enable: default_cache_enable(),
            capacity: default_cache_capacity(),
            grace_period_sec: default_grace_period(),
            min_ttl: default_min_ttl(),
        }
    }
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            interval_hours: default_update_interval(),
            concurrent_downloads: default_concurrent_downloads(),
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            enable: default_log_enable(),
            log_blocked: default_log_blocked(),
            log_all_queries: default_log_all_queries(),
            format: default_log_format(),
            target: default_log_target(),
            level: default_log_level(),
            file_path: None,
            syslog_addr: None,
            query_log_sinks: default_query_log_sinks(),
            sqlite_path: default_sqlite_path(),
            sqlite_retention_hours: default_sqlite_retention_hours(),
        }
    }
}

impl Default for StatsConfig {
    fn default() -> Self {
        Self {
            enable: default_stats_enable(),
            log_interval_seconds: default_log_interval(),
        }
    }
}

impl Config {
    pub async fn load(path: impl AsRef<Path>) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .await
            .context("Failed to read config file")?;
        let config: Config = toml::from_str(&contents).context("Failed to parse config TOML")?;
        Ok(config)
    }

    pub fn get_blocklists_sorted(&self) -> Vec<(String, String)> {
        let mut list: Vec<_> = self
            .blocklists
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        list.sort_by(|a, b| a.0.cmp(&b.0));
        list
    }
}
