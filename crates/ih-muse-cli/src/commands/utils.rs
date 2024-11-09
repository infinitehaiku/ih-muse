// crates/ih-muse-cli/src/commands/utils.rs

use std::sync::Arc;

use ih_muse_client::{PoetClient, PoetEndpoint};
use ih_muse_core::DummyCacheStrategy;

pub fn create_poet_client(poet_url: &str) -> PoetClient {
    let endpoints = vec![PoetEndpoint {
        url: poet_url.to_string(),
    }];
    PoetClient::new(endpoints)
}
