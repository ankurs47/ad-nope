use crate::config::LoggingConfig;
use crate::logger::types::{QueryLogEntry, QueryLogSink};
use rusqlite::{params, Connection, Result};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{error, info};

pub struct SqliteLogSink {
    tx: Sender<QueryLogEntry>,
}

impl SqliteLogSink {
    pub fn new(config: LoggingConfig, blocklist_names: Vec<String>) -> Self {
        let (tx, rx) = mpsc::channel::<QueryLogEntry>();
        let db_path = config.sqlite_path.clone();
        let retention_hours = config.sqlite_retention_hours;

        thread::spawn(move || {
            if let Err(e) = run_sqlite_writer(db_path, retention_hours, rx, blocklist_names) {
                error!("SQLite writer failed: {}", e);
            }
        });

        Self { tx }
    }
}

impl QueryLogSink for SqliteLogSink {
    fn log(&self, entry: &QueryLogEntry) {
        // Use try_send to avoid blocking the caller (Tokio thread) if the channel buffer is full?
        // std::sync::mpsc::channel is unbounded, so send won't block unless OOM.
        // It's safe to use send() here.
        if let Err(e) = self.tx.send(entry.clone()) {
            error!("Failed to send log to SQLite writer: {}", e);
        }
    }
}

fn run_sqlite_writer(
    db_path: String,
    retention_hours: u64,
    rx: Receiver<QueryLogEntry>,
    blocklist_names: Vec<String>,
) -> Result<()> {
    let conn = Connection::open(&db_path)?;

    // Initialize Schema
    conn.execute(
        "CREATE TABLE IF NOT EXISTS query_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp INTEGER NOT NULL,
            client_ip TEXT NOT NULL,
            domain TEXT NOT NULL,
            query_type TEXT NOT NULL,
            action TEXT NOT NULL,
            blocklist_name TEXT,
            upstream TEXT,
            latency_ms INTEGER
        )",
        [],
    )?;

    // Indices
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_logs_timestamp ON query_logs(timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_logs_domain ON query_logs(domain)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_logs_client_ip ON query_logs(client_ip)",
        [],
    )?;

    info!("SQLite log sink initialized at {}", db_path);

    let mut last_cleanup = SystemTime::now();

    while let Ok(entry) = rx.recv() {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let blocklist_name = entry
            .source_id
            .and_then(|id| blocklist_names.get(id as usize).map(|s| s.as_str()));

        let client_ip_stripped =
            if let Ok(socket_addr) = entry.client_ip.parse::<std::net::SocketAddr>() {
                socket_addr.ip().to_string()
            } else {
                entry.client_ip.clone() // Fallback if parsing fails (e.g. already stripped or invalid)
            };

        // Insert
        let res = conn.execute(
            "INSERT INTO query_logs (
                timestamp, client_ip, domain, query_type,
                action, blocklist_name, upstream, latency_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                timestamp,
                client_ip_stripped,
                entry.domain,
                entry.query_type,
                format!("{:?}", entry.action), // Enum to string
                blocklist_name,
                entry.upstream,
                entry.latency_ms as i64
            ],
        );

        if let Err(e) = res {
            error!("Failed to insert log entry into SQLite: {}", e);
        }

        // Periodic retention cleanup (e.g., every hour)
        if last_cleanup.elapsed().unwrap_or_default() > Duration::from_secs(3600) {
            let cutoff = timestamp - (retention_hours * 3600) as i64;
            if let Err(e) = conn.execute(
                "DELETE FROM query_logs WHERE timestamp < ?1",
                params![cutoff],
            ) {
                error!("Failed to purge old logs: {}", e);
            }
            last_cleanup = SystemTime::now();
        }
    }

    info!("SQLite writer stopping.");
    Ok(())
}
