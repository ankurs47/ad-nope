use crate::config::LoggingConfig;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;

#[derive(Debug)]
pub struct LogEntry {
    pub client_ip: String,
    pub domain: String,
    pub query_type: String,
    pub action: LogAction,
    pub source_id: Option<u8>, // If blocked
    pub latency_ms: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LogAction {
    Allowed,
    Blocked,
    Cached,
    Forwarded,
}

pub struct QueryLogger {
    tx: mpsc::Sender<LogEntry>,
}

impl QueryLogger {
    pub fn new(config: LoggingConfig) -> Arc<Self> {
        let (tx, mut rx) = mpsc::channel(1000); // Buffer up to 1000 logs

        // Spawn async logger task
        tokio::spawn(async move {
            while let Some(entry) = rx.recv().await {
                Self::process_log(&config, entry);
            }
        });

        Arc::new(Self { tx })
    }

    pub async fn log(&self, entry: LogEntry) {
        // Fire and forget, don't block caller if buffer full (drop log instead of stalling DNS)
        let _ = self.tx.try_send(entry);
    }

    fn process_log(config: &LoggingConfig, entry: LogEntry) {
        if !config.enable {
            return;
        }

        let should_log = match entry.action {
            LogAction::Blocked => config.log_blocked,
            _ => config.log_all_queries,
        };

        if should_log {
            if config.format == "json" {
                // Structured JSON logging via tracing
                info!(
                    target: "dns_query",
                    client = %entry.client_ip,
                    domain = %entry.domain,
                    type = %entry.query_type,
                    action = ?entry.action,
                    src = ?entry.source_id,
                    lat = %entry.latency_ms
                );
            } else {
                // Text format
                let action_str = match entry.action {
                    LogAction::Blocked => format!("BLOCKED[src={:?}]", entry.source_id),
                    LogAction::Allowed => "ALLOWED".to_string(),
                    LogAction::Cached => "CACHED".to_string(),
                    LogAction::Forwarded => "FORWARDED".to_string(),
                };

                info!(
                    "{} {} ({}) -> {} [{}ms]",
                    entry.client_ip, entry.domain, entry.query_type, action_str, entry.latency_ms
                );
            }
        }
    }
}
