// tests/it/test_metrics.rs
use super::common::TestContext;

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

    // Retrieve and verify metrics, once the sender task delivered them
    let metrics = ctx.wait_for_metrics(element_id, 1).await;

    assert!(!metrics.is_empty(), "No metrics retrieved");
    println!("Retrieved metrics: {:?}", metrics);
    assert!(
        metrics.iter().any(|metric| metric.element_id == element_id),
        "Sent metric not found"
    );
}
