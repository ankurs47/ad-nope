use crate::config::LoggingConfig;
use crate::logger::types::{QueryLogAction, QueryLogEntry, QueryLogSink};
use tracing::info;

pub struct ConsoleLogSink {
    config: LoggingConfig,
    blocklist_names: Vec<String>,
}

impl ConsoleLogSink {
    pub fn new(config: LoggingConfig, blocklist_names: Vec<String>) -> Self {
        Self {
            config,
            blocklist_names,
        }
    }
}

impl QueryLogSink for ConsoleLogSink {
    fn log(&self, entry: &QueryLogEntry) {
        if !self.config.enable {
            return;
        }

        let should_log = match entry.action {
            QueryLogAction::Blocked => self.config.log_blocked,
            _ => self.config.log_all_queries,
        };

        if should_log {
            if self.config.format == "json" {
                // Resolve source name if blocked
                let src_name = entry
                    .source_id
                    .and_then(|id| self.blocklist_names.get(id as usize));

                // Structured JSON logging via tracing
                info!(
                    target: "dns_query",
                    client = %entry.client_ip,
                    domain = %entry.domain,
                    r#type = %entry.query_type,
                    action = ?entry.action,
                    src_id = ?entry.source_id,
                    src_name = ?src_name,
                    upstream = ?entry.upstream,
                    lat = %entry.latency_ms,
                    ttl = ?entry.ttl_remaining
                );
            } else {
                // Text format
                let action_str = match entry.action {
                    QueryLogAction::Local => "fetched from local_records".to_string(),
                    QueryLogAction::Blocked => {
                        let name = entry
                            .source_id
                            .and_then(|id| {
                                self.blocklist_names.get(id as usize).map(|s| s.as_str())
                            })
                            .unwrap_or("Unknown");
                        format!("blocked and blocklist {} blocked it", name)
                    }
                    QueryLogAction::Cached => {
                        if let Some(ttl) = entry.ttl_remaining {
                            format!("fetched from cache. TTL remaining: {}s", ttl)
                        } else {
                            "fetched from cache".to_string()
                        }
                    }
                    QueryLogAction::Forwarded => {
                        if let Some(ref up) = entry.upstream {
                            format!("fetched from upstream {}", up)
                        } else {
                            "fetched from upstream".to_string()
                        }
                    }
                    QueryLogAction::Allowed => "Allowed".to_string(), // Fallback or unused?
                };

                info!(
                    "[{}] {} {} ({}) -> {} [{}ms]",
                    entry.query_type,
                    entry.client_ip,
                    entry.domain,
                    entry.query_type, // Duplicated in requirement? "type of request, query, time". I put type in [] and query in domain.
                    action_str,
                    entry.latency_ms
                );
            }
        }
    }
}
