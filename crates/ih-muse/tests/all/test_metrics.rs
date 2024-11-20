// tests/it/test_metrics.rs
use super::common::{TestContext, DEFAULT_WAIT_TIME};
use ih_muse_proto::MetricQuery;
use tokio::time::sleep;

#[tokio::test]
async fn test_send_and_receive_metric() {
    let ctx = TestContext::new(None).await;
    let local_elem_id = ctx.register_test_element().await;

    let state = ctx.muse.get_state();
    let element_id = state
        .get_element_id(&local_elem_id)
        .expect("Element was not registered");

    // Send metric
    ctx.muse
        .send_metric(local_elem_id, "cpu_usage", 42.0)
        .await
        .expect("Failed to send metric");

    sleep(DEFAULT_WAIT_TIME).await;

    // Retrieve and verify metrics
    let poet_client = ctx.muse.get_client();
    let query = MetricQuery {
        start_time: None,
        end_time: None,
        element_id: Some(element_id),
        parent_id: None,
        metric_id: None,
    };

    let metrics = poet_client
        .get_metrics(&query, None)
        .await
        .expect("Failed to get metrics");

    assert!(!metrics.is_empty(), "No metrics retrieved");
    println!("Retrieved metrics: {:?}", metrics);
    assert!(
        metrics.iter().any(|metric| metric.element_id == element_id),
        "Sent metric not found"
    );
}