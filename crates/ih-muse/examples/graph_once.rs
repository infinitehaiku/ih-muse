//! Collect one bounded native graph batch and optionally deliver it to Poet.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use ih_muse::graph_muse::{GraphMuse, GraphMuseConfig, KubernetesContext};
use ih_muse_client::GraphPoetClient;
use uuid::Uuid;

fn setting(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required"))
}

fn private_token(path: &str) -> String {
    let metadata = fs::metadata(path).expect("graph token file must exist");
    assert!(metadata.is_file() && metadata.len() > 0 && metadata.len() <= 4096);
    assert_eq!(
        metadata.permissions().mode() & 0o077,
        0,
        "token file must be owner-only"
    );
    let token = fs::read_to_string(path).expect("graph token file must be UTF-8");
    assert!(
        !token.contains(['\r', '\n', '\0']),
        "token contains control bytes"
    );
    token
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
        let token = private_token(&setting("IH_GRAPH_POET_TOKEN_FILE"));
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
