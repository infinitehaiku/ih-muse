//! Collect one bounded native graph batch and optionally deliver it to Poet.

use ih_muse::graph_muse::{GraphMuse, GraphMuseConfig, KubernetesContext};
use ih_muse::secret_file::read_secret_file;
use ih_muse_client::GraphPoetClient;
use uuid::Uuid;

fn setting(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required"))
}

/// Read a credential file, panicking with a secret-free reason when it is refused.
fn private_token(path: &str, label: &str) -> String {
    read_secret_file(path, label).unwrap_or_else(|error| panic!("{error}"))
}

#[tokio::main]
async fn main() {
    let config = GraphMuseConfig {
        organization: setting("IH_GRAPH_ORGANIZATION"),
        owner_id: setting("IH_GRAPH_OWNER"),
        source_id: setting("IH_GRAPH_SOURCE"),
        environment_id: setting("IH_GRAPH_ENVIRONMENT"),
        host_id: setting("IH_GRAPH_HOST"),
        kubernetes: KubernetesContext::from_environment(),
        rustvello_runner: None,
        max_processes: 8,
    };
    let mut muse = GraphMuse::new(config).expect("invalid graph Muse configuration");
    let batch = muse.collect(&[]).expect("native graph collection failed");
    let request = muse
        .intake_request(Uuid::new_v4().to_string(), batch)
        .expect("native graph batch is invalid");
    if let Ok(endpoint) = std::env::var("IH_GRAPH_POET_ENDPOINT") {
        let token = private_token(&setting("IH_GRAPH_POET_TOKEN_FILE"), "graph Poet token");
        GraphPoetClient::new(endpoint, token)
            .expect("invalid Poet endpoint")
            .publish(&request)
            .await
            .expect("Poet rejected graph batch");
    }
    println!(
        "{}",
        serde_json::to_string(&request).expect("graph request serializes")
    );
}
