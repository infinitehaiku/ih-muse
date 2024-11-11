fn start_element_sender(&self) {
    let client = self.client.clone();
    let elements_buffer = self.elements_buffer.clone();
    let state = self.state.clone();

    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;

            let elements_to_send = {
                let mut buffer = elements_buffer.lock().await;
                std::mem::take(&mut *buffer)
            };

            if !elements_to_send.is_empty() {
                match client.register_elements(elements_to_send.clone()).await {
                    Ok(results) => {
                        // Update state with new element IDs
                        state.update_elements(&elements_to_send, &results).await;
                    }
                    Err(e) => {
                        eprintln!("Error registering elements: {:?}", e);
                    }
                }
            }
        }
    });
}
