pub mod console_sink;
pub mod sqlite_sink;
pub mod types;

pub use self::console_sink::ConsoleLogSink;
pub use self::sqlite_sink::SqliteLogSink;
pub use self::types::{QueryLogAction, QueryLogEntry, QueryLogSink};

use crate::config::LoggingConfig;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct QueryLogger {
    sinks: Vec<mpsc::Sender<QueryLogEntry>>,
}

impl QueryLogger {
    pub fn new(config: LoggingConfig, blocklist_names: Vec<String>) -> Arc<Self> {
        let mut sinks = Vec::new();

        for sink_type in &config.query_log_sinks {
            if sink_type == "console" {
                let (tx, mut rx) = mpsc::channel(1000);
                let console_sink = ConsoleLogSink::new(config.clone(), blocklist_names.clone());
                let sink = Box::new(console_sink);

                tokio::spawn(async move {
                    while let Some(entry) = rx.recv().await {
                        sink.log(&entry);
                    }
                });
                sinks.push(tx);
            } else if sink_type == "sqlite" {
                let (tx, mut rx) = mpsc::channel(1000);
                let sqlite_sink = SqliteLogSink::new(config.clone(), blocklist_names.clone());
                let sink = Box::new(sqlite_sink);

                tokio::spawn(async move {
                    while let Some(entry) = rx.recv().await {
                        sink.log(&entry);
                    }
                });
                sinks.push(tx);
            } else {
                eprintln!("Unknown log sink type: {}", sink_type);
            }
        }

        Arc::new(Self { sinks })
    }

    pub async fn log(&self, entry: QueryLogEntry) {
        let len = self.sinks.len();
        for (i, sink) in self.sinks.iter().enumerate() {
            // Fire and forget, don't block caller if buffer full
            if i == len - 1 {
                let _ = sink.try_send(entry);
                break;
            }
            let _ = sink.try_send(entry.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code)]
    struct TestLogSink {
        pub logs: std::sync::Mutex<Vec<QueryLogEntry>>,
    }

    impl QueryLogSink for TestLogSink {
        fn log(&self, entry: &QueryLogEntry) {
            self.logs.lock().unwrap().push(entry.clone());
        }
    }

    #[test]
    fn test_log_entry_fields() {
        let entry = QueryLogEntry {
            client_ip: "127.0.0.1".parse().unwrap(),
            domain: "example.com".into(),
            query_type: hickory_server::proto::rr::RecordType::A,
            action: QueryLogAction::Local,
            source_id: None,
            upstream: None,
            latency_ms: 10,
            ttl_remaining: None,
        };
        assert_eq!(entry.client_ip.to_string(), "127.0.0.1");
        assert_eq!(entry.action, QueryLogAction::Local);
    }
}
