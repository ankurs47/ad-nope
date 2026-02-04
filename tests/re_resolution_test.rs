use ad_nope::config::Config;
use ad_nope::engine::BlocklistMatcher;
use ad_nope::logger::QueryLogger;
use ad_nope::resolver::DnsResolver;
use ad_nope::server::DnsHandler;
use ad_nope::stats::StatsCollector;
use hickory_server::proto::op::ResponseCode;
use hickory_server::proto::rr::{Name, RData, Record, RecordType};
// use hickory_server::server::{RequestHandler, ResponseHandler, ResponseInfo};
use hickory_server::ServerFuture;
// use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;

struct MockBlocklist;
impl BlocklistMatcher for MockBlocklist {
    fn check(&self, _domain: &str) -> Option<u8> {
        None
    }
}

struct MockResolver {
    call_count: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl DnsResolver for MockResolver {
    async fn resolve(
        &self,
        name: &str,
        _qtype: RecordType,
    ) -> anyhow::Result<(Vec<Record>, String)> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        let name = Name::from_ascii(name).unwrap();
        // TTL = 1 second
        let record = Record::from_rdata(name, 1, RData::A("1.2.3.4".parse().unwrap()));
        Ok((vec![record], "mock".to_string()))
    }
}

#[tokio::test]
async fn test_background_re_resolution() {
    let mut config = Config::default();
    config.cache.grace_period_sec = 5; // Stale window: [1s, 6s]
    config.cache.capacity = 1000;

    let stats = StatsCollector::new(10, vec![], vec![]);
    let logger = QueryLogger::new(config.logging.clone(), vec![]);
    let call_count = Arc::new(AtomicUsize::new(0));
    let resolver = Arc::new(MockResolver {
        call_count: call_count.clone(),
    });
    let blocklist = Arc::new(MockBlocklist);

    let handler = DnsHandler::new(config.clone(), stats, logger, blocklist, resolver);

    // Start Server
    let mut server = ServerFuture::new(handler);
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    server.register_socket(socket);

    // Spawn server in background
    tokio::spawn(async move {
        let _ = server.block_until_done().await;
    });

    // Create a client socket
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    client.connect(addr).await.unwrap();

    // 1. First Query
    let mut query_msg = hickory_server::proto::op::Message::new();
    query_msg.add_query(hickory_server::proto::op::Query::query(
        Name::from_ascii("example.com.").unwrap(),
        RecordType::A,
    ));
    query_msg.set_id(1234);

    let query_bytes = query_msg.to_vec().unwrap();
    client.send(&query_bytes).await.unwrap();

    let mut buf = [0u8; 512];
    let (len, _) = client.recv_from(&mut buf).await.unwrap();
    let resp = hickory_server::proto::op::Message::from_vec(&buf[..len]).unwrap();

    assert_eq!(resp.response_code(), ResponseCode::NoError);
    assert!(!resp.answers().is_empty());
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "Should call resolver once"
    );

    // 2. Wait for TTL (1s) to expire, but stay within grace period (5s)
    // We wait 1.2s
    tokio::time::sleep(Duration::from_millis(1200)).await;

    // 3. Second Query (Should receive stale response AND trigger background refresh)
    query_msg.set_id(5678);
    let query_bytes = query_msg.to_vec().unwrap();
    client.send(&query_bytes).await.unwrap();

    let (len, _) = client.recv_from(&mut buf).await.unwrap();
    let resp = hickory_server::proto::op::Message::from_vec(&buf[..len]).unwrap();

    assert_eq!(resp.response_code(), ResponseCode::NoError);
    // Should verify call_count increases.
    // Background task might take a moment.

    // Check loop
    let mut calls = 0;
    for _ in 0..20 {
        // Increased wait cycles
        calls = call_count.load(Ordering::SeqCst);
        if calls >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert!(
        calls >= 2,
        "Resolver should be called a second time (background refresh). Current: {}",
        calls
    );
}
