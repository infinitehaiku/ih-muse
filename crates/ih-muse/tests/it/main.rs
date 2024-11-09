#[tokio::test]
async fn test_send_metric() {
    // Setup test config
    let cache_strategy = Arc::new(ih_muse_core::cache::TimeBasedCache::new());
    let config = ih_muse::Config {
        endpoints: vec!["http://localhost:8000".to_string()],
        cache_strategy,
    };
    let muse = ih_muse::Muse::new(config);

    // Test sending a metric
    let result = muse
        .send_metric("test_element".to_string(), "cpu_usage".to_string(), 42.0)
        .await;

    assert!(result.is_ok());
}
