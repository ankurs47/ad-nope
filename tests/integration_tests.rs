use ad_nope::config::Config;
use ad_nope::logger::QueryLogger;
use ad_nope::stats::StatsCollector;

// --- Mocks ---
// (Mocks removed as they are unused for now)

#[tokio::test]
async fn test_component_instantiation() {
    let config = Config::default();
    let _stats = StatsCollector::new(10);
    // Logger
    let _logger = QueryLogger::new(config.logging.clone());

    // Components
    // We can't easily test DnsHandler without mocks, but we verified it compiles.
}
