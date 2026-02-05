use crate::logger::types::{QueryLogAction, QueryLogEntry};
use crate::stats::{StatsSnapshot, TopItem};
use hickory_server::proto::rr::RecordType;
use rusqlite::{params, Connection, Result};
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{error, info};

pub struct DbClient {
    db_path: String,
}

impl DbClient {
    pub fn new(db_path: String) -> Self {
        Self { db_path }
    }

    fn get_connection(&self) -> Result<Connection> {
        Connection::open(&self.db_path)
    }

    pub fn initialize(&self) -> Result<()> {
        let conn = self.get_connection()?;

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
                latency_ms INTEGER,
                ttl_remaining INTEGER
            )",
            [],
        )?;

        // Migration: Add ttl_remaining if missing
        let column_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('query_logs') WHERE name='ttl_remaining'",
                [],
                |row| match row.get(0) {
                    Ok(count) => Ok(count),
                    Err(e) => Err(e),
                },
            )
            .unwrap_or(0)
            > 0;

        if !column_exists {
            info!("Applying migration: adding ttl_remaining column to query_logs");
            if let Err(e) = conn.execute(
                "ALTER TABLE query_logs ADD COLUMN ttl_remaining INTEGER",
                [],
            ) {
                error!("Failed to add ttl_remaining column: {}", e);
            }
        }

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

        info!("SQLite database initialized at {}", self.db_path);
        Ok(())
    }

    pub fn insert_log(&self, entry: &QueryLogEntry, blocklist_param: Option<&str>) -> Result<()> {
        let conn = self.get_connection()?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let client_ip_stripped = entry.client_ip.to_string();

        conn.execute(
            "INSERT INTO query_logs (
                timestamp, client_ip, domain, query_type,
                action, blocklist_name, upstream, latency_ms, ttl_remaining
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                timestamp,
                client_ip_stripped,
                entry.domain,
                entry.query_type.to_string(),
                format!("{:?}", entry.action), // Enum to string
                blocklist_param,
                entry.upstream,
                entry.latency_ms as i64,
                entry.ttl_remaining.map(|t| t as i64)
            ],
        )?;

        Ok(())
    }

    pub fn prune_logs(&self, retention_hours: u64) -> Result<()> {
        let conn = self.get_connection()?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let cutoff = timestamp - (retention_hours * 3600) as i64;

        conn.execute(
            "DELETE FROM query_logs WHERE timestamp < ?1",
            params![cutoff],
        )?;
        Ok(())
    }

    pub fn get_stats(&self) -> StatsSnapshot {
        let conn = match self.get_connection() {
            Ok(c) => c,
            Err(e) => {
                error!("Failed to open SQLite connection for stats: {}", e);
                return empty_snapshot();
            }
        };

        let total_queries: u64 = conn
            .query_row("SELECT COUNT(*) FROM query_logs", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(0) as u64;

        let blocked_queries: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM query_logs WHERE action = 'Blocked'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0) as u64;

        let cache_hits: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM query_logs WHERE action = 'Cached'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0) as u64;

        let get_top = |query: &str| -> Vec<TopItem> {
            let mut stmt = conn.prepare(query).unwrap();
            let rows = stmt
                .query_map([], |row| {
                    Ok(TopItem {
                        name: row.get(0)?,
                        count: row.get::<_, i64>(1)? as u64,
                    })
                })
                .unwrap();
            rows.filter_map(Result::ok).collect()
        };

        let top_clients = get_top("SELECT client_ip, COUNT(*) as c FROM query_logs GROUP BY client_ip ORDER BY c DESC LIMIT 5");
        let top_blocked_clients = get_top("SELECT client_ip, COUNT(*) as c FROM query_logs WHERE action = 'Blocked' GROUP BY client_ip ORDER BY c DESC LIMIT 5");
        let top_domains = get_top(
            "SELECT domain, COUNT(*) as c FROM query_logs GROUP BY domain ORDER BY c DESC LIMIT 5",
        );
        let top_blocked_domains = get_top("SELECT domain, COUNT(*) as c FROM query_logs WHERE action = 'Blocked' GROUP BY domain ORDER BY c DESC LIMIT 5");

        let started_at: u64 = conn
            .query_row("SELECT MIN(timestamp) FROM query_logs", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(0) as u64;

        let updated_at: u64 = conn
            .query_row("SELECT MAX(timestamp) FROM query_logs", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or_else(|_| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64
            }) as u64;

        StatsSnapshot {
            total_queries,
            blocked_queries,
            cache_hits,
            top_clients,
            top_blocked_clients,
            top_domains,
            top_blocked_domains,
            started_at,
            updated_at,
        }
    }

    pub fn get_logs(&self, limit: usize) -> Vec<QueryLogEntry> {
        let conn = match self.get_connection() {
            Ok(c) => c,
            Err(e) => {
                error!("Failed to open SQLite connection for logs: {}", e);
                return Vec::new();
            }
        };

        let mut stmt = conn
            .prepare(
                "SELECT client_ip, domain, query_type, action, blocklist_name, upstream, latency_ms, ttl_remaining 
                 FROM query_logs ORDER BY timestamp DESC LIMIT ?",
            )
            .unwrap();

        let rows = stmt
            .query_map([limit as i64], |row| {
                let client_ip_str: String = row.get(0)?;
                let domain: String = row.get(1)?;
                let query_type_str: String = row.get(2)?;
                let action_str: String = row.get(3)?;
                let _blocklist_name: Option<String> = row.get(4)?;
                let upstream: Option<String> = row.get(5)?;
                let latency_ms: i64 = row.get(6)?;
                let ttl_remaining: Option<i64> = row.get(7)?;

                Ok(QueryLogEntry {
                    client_ip: IpAddr::from_str(&client_ip_str)
                        .unwrap_or(IpAddr::from([0, 0, 0, 0])),
                    domain: Arc::from(domain),
                    query_type: RecordType::from_str(&query_type_str).unwrap_or(RecordType::A),
                    action: parse_action(&action_str),
                    source_id: None,
                    upstream,
                    latency_ms: latency_ms as u64,
                    ttl_remaining: ttl_remaining.map(|t| t as u64),
                })
            })
            .unwrap();

        rows.filter_map(Result::ok).collect()
    }
}

fn empty_snapshot() -> StatsSnapshot {
    StatsSnapshot {
        total_queries: 0,
        blocked_queries: 0,
        cache_hits: 0,
        top_clients: vec![],
        top_blocked_clients: vec![],
        top_domains: vec![],
        top_blocked_domains: vec![],
        started_at: 0,
        updated_at: 0,
    }
}

fn parse_action(s: &str) -> QueryLogAction {
    match s {
        "Allowed" => QueryLogAction::Allowed,
        "Blocked" => QueryLogAction::Blocked,
        "Cached" => QueryLogAction::Cached,
        "Forwarded" => QueryLogAction::Forwarded,
        "Local" => QueryLogAction::Local,
        _ => QueryLogAction::Allowed, // Fallback
    }
}
