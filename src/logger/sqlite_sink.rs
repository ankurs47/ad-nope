use crate::config::LoggingConfig;
use crate::db::DbClient;
use crate::logger::types::{QueryLogEntry, QueryLogSink};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime};
use tracing::{error, info};

pub struct SqliteLogSink {
    tx: Sender<QueryLogEntry>,
}

impl SqliteLogSink {
    pub fn new(db: Arc<DbClient>, config: LoggingConfig, blocklist_names: Vec<String>) -> Self {
        let (tx, rx) = mpsc::channel::<QueryLogEntry>();
        let retention_hours = config.sqlite_retention_hours;

        thread::spawn(move || {
            if let Err(e) = run_sqlite_writer(db, retention_hours, rx, blocklist_names) {
                error!("SQLite writer failed: {}", e);
            }
        });

        Self { tx }
    }
}

impl QueryLogSink for SqliteLogSink {
    fn log(&self, entry: &QueryLogEntry) {
        if let Err(e) = self.tx.send(entry.clone()) {
            error!("Failed to send log to SQLite writer: {}", e);
        }
    }
}

fn run_sqlite_writer(
    db: Arc<DbClient>,
    retention_hours: u64,
    rx: Receiver<QueryLogEntry>,
    blocklist_names: Vec<String>,
) -> anyhow::Result<()> {
    // Initialize DB (ensure schema exists) - safe to call multiple times?
    // DbClient::initialize uses "CREATE IF NOT EXISTS", so yes.
    // However, if we share it, maybe we initialize once in main?
    // The plan says "run_sqlite_writer calling db.initialize()".
    // If multiple things call it, it's fine as long as it's idempotent.
    // The DbClient implementation I wrote has idempotent SQL.
    if let Err(e) = db.initialize() {
        error!("Failed to initialize database: {}", e);
        return Err(anyhow::anyhow!("Database initialization failed: {}", e));
    }

    let mut last_cleanup = SystemTime::now();

    while let Ok(entry) = rx.recv() {
        let blocklist_name = entry
            .source_id
            .and_then(|id| blocklist_names.get(id as usize).map(|s| s.as_str()));

        if let Err(e) = db.insert_log(&entry, blocklist_name) {
            error!("Failed to insert log entry: {}", e);
        }

        // Periodic retention cleanup (e.g., every hour)
        if last_cleanup.elapsed().unwrap_or_default() > Duration::from_secs(3600) {
            if let Err(e) = db.prune_logs(retention_hours) {
                error!("Failed to prune old logs: {}", e);
            }
            last_cleanup = SystemTime::now();
        }
    }

    info!("SQLite writer stopping.");
    Ok(())
}
