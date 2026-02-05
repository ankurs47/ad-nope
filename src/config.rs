//! Configuration module for `ad-nope`.
//!
//! This module defines the structure and default values for the application's configuration.
//! It uses `serde` for serialization/deserialization and `toml` for the file format.
//!
//! # Example Config
//! ```toml
//! host = "0.0.0.0"
//! port = 5300
//!
//! [cache]
//! enable = true
//! capacity = 10000
//! ```

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

/// Main configuration struct holding all settings for the DNS server.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Config {
    /// The IP address to bind the DNS server to (e.g., "0.0.0.0").
    #[serde(default = "default_host")]
    pub host: String,

    /// The port to listen on (e.g., 53 or 5300).
    #[serde(default = "default_port")]
    pub port: u16,

    /// List of upstream DNS servers (e.g., "8.8.8.8:53" or "h3://dns.google").
    #[serde(default = "default_upstream_servers")]
    pub upstream_servers: Vec<String>,

    /// Strategy for resolving queries (e.g., "parallel", "round-robin").
    #[serde(default = "default_resolution_policy")]
    pub resolution_policy: String,

    /// Timeout in milliseconds for upstream queries.
    #[serde(default = "default_upstream_timeout_ms")]
    pub upstream_timeout_ms: u64,

    /// Bootstrap DNS servers used to resolve DoD (DNS-over-HTTPS/TLS) upstreams.
    #[serde(default = "default_bootstrap_dns")]
    pub bootstrap_dns: Vec<String>,

    /// Map of blocklist names to their URLs.
    #[serde(default = "default_blocklists")]
    pub blocklists: HashMap<String, String>,

    /// List of domains to explicitly allow (bypass blocklists).
    #[serde(default)]
    pub allowlist: Vec<String>,

    /// Caching configuration.
    #[serde(default)]
    pub cache: CacheConfig,

    /// Blocklist update configuration.
    #[serde(default)]
    pub updates: UpdateConfig,

    /// Logging configuration.
    #[serde(default)]
    pub logging: LoggingConfig,

    /// Local DNS records (static overrides).
    #[serde(default)]
    pub local_records: HashMap<String, std::net::IpAddr>,

    /// Statistics collection configuration.
    #[serde(default)]
    pub stats: StatsConfig,
}

/// Configuration for the in-memory DNS cache.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CacheConfig {
    /// Whether to enable caching.
    #[serde(default = "default_cache_enable")]
    pub enable: bool,

    /// Maximum number of records to cache.
    #[serde(default = "default_cache_capacity")]
    pub capacity: u64,

    /// Time in seconds to keep stale records while refreshing in background.
    #[serde(default = "default_grace_period")]
    pub grace_period_sec: u64,

    /// Minimum TTL to enforce for cached records.
    #[serde(default = "default_min_ttl")]
    pub min_ttl: u32,
}

/// Configuration for blocklist updates.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct UpdateConfig {
    /// How often to check for blocklist updates (in hours).
    #[serde(default = "default_update_interval")]
    pub interval_hours: u64,

    /// Number of concurrent downloads allowed during update.
    #[serde(default = "default_concurrent_downloads")]
    pub concurrent_downloads: usize,
}

fn default_min_ttl() -> u32 {
    300
}

/// Configuration for query logging.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LoggingConfig {
    /// Global switch to enable/disable logging.
    #[serde(default = "default_log_enable")]
    pub enable: bool,

    /// Whether to log blocked queries.
    #[serde(default = "default_log_blocked")]
    pub log_blocked: bool,

    /// Whether to log all queries (allowed & blocked).
    #[serde(default = "default_log_all_queries")]
    pub log_all_queries: bool,

    /// Format of the logs (e.g., "text", "json").
    #[serde(default = "default_log_format")]
    pub format: String,

    /// Target for logs (e.g., "console", "syslog").
    #[serde(default = "default_log_target")]
    pub target: String,

    /// Log level (e.g., "info", "debug").
    #[serde(default = "default_log_level")]
    pub level: String,

    /// Optional file path if logging to a file.
    #[serde(default)]
    pub file_path: Option<String>,

    /// Optional syslog address if sending to syslog.
    #[serde(default)]
    pub syslog_addr: Option<String>,

    /// Sinks for structured query logs (e.g., ["console", "sqlite"]).
    #[serde(default = "default_query_log_sinks")]
    pub query_log_sinks: Vec<String>,

    /// Path to the SQLite database for logs.
    #[serde(default = "default_sqlite_path")]
    pub sqlite_path: String,

    /// Retention period for SQLite logs in hours.
    #[serde(default = "default_sqlite_retention_hours")]
    pub sqlite_retention_hours: u64,
}

/// Configuration for metrics and statistics.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct StatsConfig {
    /// Whether to collect statistics.
    #[serde(default = "default_stats_enable")]
    pub enable: bool,

    /// Interval in seconds for dumping stats to logs.
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
    /// Loads the configuration from a specific file path.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or if the TOML parsing fails.
    pub async fn load(path: impl AsRef<Path>) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .await
            .context("Failed to read config file")?;
        let config: Config = toml::from_str(&contents).context("Failed to parse config TOML")?;
        Ok(config)
    }

    /// Returns a sorted list of configured blocklists.
    ///
    /// The sorting is done by the blocklist name (key).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_values() {
        let config = Config::default();
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 5300);
        assert_eq!(config.upstream_servers, vec!["h3://dns.google/dns-query"]);
        assert!(config.blocklists.contains_key("default"));
        assert!(config.cache.enable);
        assert_eq!(config.cache.capacity, 10000);
    }

    #[test]
    fn test_sorted_blocklists() {
        let mut config = Config::default();
        config.blocklists.clear();
        config
            .blocklists
            .insert("b".to_string(), "url2".to_string());
        config
            .blocklists
            .insert("a".to_string(), "url1".to_string());
        config
            .blocklists
            .insert("c".to_string(), "url3".to_string());

        let sorted = config.get_blocklists_sorted();
        assert_eq!(sorted.len(), 3);
        assert_eq!(sorted[0].0, "a");
        assert_eq!(sorted[1].0, "b");
        assert_eq!(sorted[2].0, "c");
    }

    #[test]
    fn test_toml_deserialization() {
        let toml_str = r#"
            host = "127.0.0.1"
            port = 9999
            [cache]
            capacity = 500
        "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 9999);
        assert_eq!(config.cache.capacity, 500);
        // Defaults should still hold for missing fields
        assert!(config.cache.enable);
    }
}
