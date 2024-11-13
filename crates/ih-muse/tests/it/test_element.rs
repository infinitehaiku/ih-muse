// tests/it/test_element.rs
use super::common::TestContext;
use ih_muse::ClientType;

#[tokio::test]
async fn test_element_registration() {
    let ctx = TestContext::new(ClientType::Poet).await;
    let local_elem_id = ctx.register_test_element().await;
    let state = ctx.muse.get_state();

    let element_id = state
        .get_element_id(&local_elem_id)
        .expect("Element was not registered");

    println!(
        "Element with LocalElementId {:?} was registered with ElementId {:?}",
        local_elem_id, element_id
    );
}
