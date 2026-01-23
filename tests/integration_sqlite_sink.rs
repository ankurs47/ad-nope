use ad_nope::config::LoggingConfig;
use ad_nope::logger::{QueryLogAction, QueryLogEntry, QueryLogger};
use rusqlite::Connection;
use std::fs;
use std::time::Duration;

#[tokio::test]
async fn test_sqlite_sink_logging() {
    let db_path = "test_sink.db";
    // Clean up previous run
    let _ = fs::remove_file(db_path);

    let config = LoggingConfig {
        enable: true,
        log_all_queries: true,
        log_blocked: true,
        format: "text".to_string(),
        target: "console".to_string(),
        level: "info".to_string(),
        file_path: None,
        syslog_addr: None,
        query_log_sinks: vec!["sqlite".to_string()],
        sqlite_path: db_path.to_string(),
        sqlite_retention_hours: 24,
    };

    let blocklist_names = vec!["test_list".to_string()];
    let logger = QueryLogger::new(config, blocklist_names);

    // Send a log entry with a port
    let entry = QueryLogEntry {
        client_ip: "192.168.1.100:12345".to_string(), // Input has port
        domain: "example.com".to_string(),
        query_type: "A".to_string(),
        action: QueryLogAction::Forwarded,
        source_id: None,
        upstream: Some("8.8.8.8".to_string()),
        latency_ms: 15,
        ttl_remaining: None,
    };

    logger.log(entry.clone()).await;

    // Wait for async write (background thread)
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify SQLite content
    let conn = Connection::open(db_path).expect("Failed to open test DB");

    let mut stmt = conn
        .prepare("SELECT client_ip, domain, upstream FROM query_logs")
        .unwrap();
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .unwrap();

    let mut found = false;
    for row in rows {
        let (ip, domain, upstream) = row.unwrap();
        // Verify IP does NOT have port
        if ip == "192.168.1.100" && domain == "example.com" && upstream == "8.8.8.8" {
            found = true;
            break;
        }
    }

    assert!(found, "Log entry not found in SQLite DB");

    // Clean up
    let _ = fs::remove_file(db_path);
}
