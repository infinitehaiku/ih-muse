// crates/ih-muse-cli/src/commands/is_ready.rs

use clap::Args;

use super::utils::create_poet_client;
use ih_muse_core::{MuseResult, Transport};

#[derive(Args)]
pub struct IsReadyArgs {
    /// Poet server URL
    #[arg(short, long, default_value = "http://localhost:8000")]
    pub poet_url: String,
}

pub async fn execute(args: IsReadyArgs) -> MuseResult<()> {
    let client = create_poet_client(&args.poet_url);
    client.health_check().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_is_ready_success() {
        // Mock the PoetClient and its check_health method
        // Use a test poet server or mock the responses

        // Assuming the test passes
        assert!(execute(IsReadyArgs {
            poet_url: "http://localhost:8000".into()
        })
        .await
        .is_ok());
    }
}
