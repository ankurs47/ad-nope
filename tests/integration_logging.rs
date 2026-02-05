use ad_nope::config::LoggingConfig;

#[tokio::test]
async fn test_logging_config_instantiation() {
    let config = LoggingConfig {
        enable: true,
        log_all_queries: true,
        log_blocked: true,
        format: "text".to_string(),
        target: "console".to_string(),
        level: "info".to_string(),
        file_path: None,
        syslog_addr: None,
        query_log_sinks: vec!["console".to_string()],
        sqlite_path: "ad-nope.db".to_string(),
        sqlite_retention_hours: 168,
    };

    use ad_nope::logger::{QueryLogAction, QueryLogEntry, QueryLogger};
    let logger = QueryLogger::new(config.clone(), vec![], vec![], None);

    logger
        .log(QueryLogEntry {
            client_ip: "1.2.3.4".parse().unwrap(),
            domain: "test.com".into(),
            query_type: hickory_server::proto::rr::RecordType::A,
            action: QueryLogAction::Local,
            source_id: None,
            upstream: None,
            latency_ms: 0,
            ttl_remaining: None,
        })
        .await;

    // Allow time for async task to process
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
}
