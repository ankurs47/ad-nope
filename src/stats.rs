use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::time::{self, Duration};
use tracing::info;

#[derive(Debug)]
pub struct StatsCollector {
    // Basic Counters
    total_queries: AtomicU64,
    blocked_queries: AtomicU64,
    cache_hits: AtomicU64,

    // Granular Stats
    // Max 256 sources (u8 key).
    // Just using a fixed-size array for O(1) mostly-lock-free access slightly better than a Map
    // wrapped in RwLock for high contention updates?
    // Actually, for just 256 items, a fixed array of AtomicU64 is perfect and purely lock-free.
    blocks_by_source: [AtomicU64; 256],

    // Upstream Latency Tracking
    // We'll support up to 16 upstreams for stats purposes.
    // Storing (TotalTimeMs, Count) requires a struct, but we want lock-free.
    // We'll just separate TotalTime and Count arrays.
    upstream_total_ms: [AtomicU64; 16],
    upstream_count: [AtomicU64; 16],
    upstream_names: Vec<String>,
    blocklist_names: Vec<String>,

    log_interval: Duration,
}

impl StatsCollector {
    pub fn new(
        log_interval_sec: u64,
        upstream_names: Vec<String>,
        blocklist_names: Vec<String>,
    ) -> Arc<Self> {
        let stats = Arc::new(Self {
            total_queries: AtomicU64::new(0),
            blocked_queries: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            blocks_by_source: [0; 256].map(|_| AtomicU64::new(0)),
            upstream_total_ms: [0; 16].map(|_| AtomicU64::new(0)),
            upstream_count: [0; 16].map(|_| AtomicU64::new(0)),
            upstream_names,
            blocklist_names,
            log_interval: Duration::from_secs(log_interval_sec),
        });

        // Spawn background dumper
        let stats_clone = stats.clone();
        tokio::spawn(async move {
            stats_clone.run_logger().await;
        });

        stats
    }

    pub fn inc_queries(&self) {
        self.total_queries.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_blocked(&self) {
        self.blocked_queries.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_blocked_by_source(&self, source_id: u8) {
        self.inc_blocked();
        self.blocks_by_source[source_id as usize].fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_upstream_latency(&self, upstream_idx: usize, ms: u64) {
        if upstream_idx < 16 {
            self.upstream_total_ms[upstream_idx].fetch_add(ms, Ordering::Relaxed);
            self.upstream_count[upstream_idx].fetch_add(1, Ordering::Relaxed);
        }
    }

    async fn run_logger(&self) {
        let mut interval = time::interval(self.log_interval);
        loop {
            interval.tick().await;
            self.dump_stats();
        }
    }

    fn dump_stats(&self) {
        let total = self.total_queries.load(Ordering::Relaxed);
        let blocked = self.blocked_queries.load(Ordering::Relaxed);
        let hits = self.cache_hits.load(Ordering::Relaxed);

        let mut upstream_stats = String::new();
        for i in 0..16 {
            let count = self.upstream_count[i].load(Ordering::Relaxed);
            if count > 0 {
                let total_ms = self.upstream_total_ms[i].load(Ordering::Relaxed);
                let avg = total_ms as f64 / count as f64;

                let name = self
                    .upstream_names
                    .get(i)
                    .map(|s| s.as_str())
                    .unwrap_or("Unknown");
                upstream_stats.push_str(&format!("[{}: {:.1}ms] ", name, avg));
            }
        }

        let mut block_stats = String::new();
        if blocked > 0 {
            block_stats.push_str(" BlockStats: ");
            for i in 0..256 {
                let count = self.blocks_by_source[i].load(Ordering::Relaxed);
                if count > 0 {
                    let name = self
                        .blocklist_names
                        .get(i)
                        .map(|s| s.as_str())
                        .unwrap_or("Unknown");
                    let pct = (count as f64 / blocked as f64) * 100.0;
                    block_stats.push_str(&format!("[{}: {} ({:.1}%)] ", name, count, pct));
                }
            }
        }

        info!(
            "STATS DUMP: Total: {}, Blocked: {} ({:.1}%), CacheHits: {} ({:.1}%), Upstreams: {}{}",
            total,
            blocked,
            if total > 0 {
                (blocked as f64 / total as f64) * 100.0
            } else {
                0.0
            },
            hits,
            if total > 0 {
                (hits as f64 / total as f64) * 100.0
            } else {
                0.0
            },
            upstream_stats,
            block_stats
        );
    }
}
