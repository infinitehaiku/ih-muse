// tests/it/test_cluster.rs
use super::common::{TestContext, DEFAULT_WAIT_TIME};
use ih_muse::ClientType;
use tokio::time::sleep;

#[tokio::test]
async fn test_cluster_monitor_updates_state() {
    let ctx = TestContext::new(ClientType::Poet).await;
    let local_elem_id = ctx.register_test_element().await;
    let state = ctx.muse.get_state();

    // Wait for cluster monitor to update
    sleep(DEFAULT_WAIT_TIME).await;

    // Verify state updates
    let nodes = state.get_nodes().await;
    assert!(
        !nodes.is_empty(),
        "State nodes should not be empty after monitoring"
    );

    let ranges = state.get_node_elem_ranges().await;
    assert!(
        !ranges.is_empty(),
        "State ranges should not be empty after monitoring"
    );

    let test_element_id = state.get_element_id(&local_elem_id).unwrap();
    let found_node_addr = state.find_element_node_addr(test_element_id);
    assert!(
        found_node_addr.is_some(),
        "Node address should be found for element"
    );
}