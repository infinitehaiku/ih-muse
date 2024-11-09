// ih-muse-core/src/cache.rs

use std::time::{Duration, Instant};

use async_trait::async_trait;
use hashbrown::hash_map::HashMap;
use tokio::sync::RwLock;

/// Trait for cache strategies.
#[async_trait]
pub trait CacheStrategy {
    async fn should_send(&self, element: &str, metric: &str, interval: Duration) -> bool;

    async fn update_last_sent(&mut self, element: &str, metric: &str);
}

pub struct TimeBasedCache {
    last_sent: RwLock<HashMap<(String, String), Instant>>,
}

impl TimeBasedCache {
    pub fn new() -> Self {
        Self {
            last_sent: RwLock::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl CacheStrategy for TimeBasedCache {
    async fn should_send(&self, element_id: &str, metric_id: &str, resolution: Duration) -> bool {
        let key = (element_id.to_string(), metric_id.to_string());
        let now = Instant::now();
        let mut last_sent = self.last_sent.write().await;

        match last_sent.get(&key) {
            Some(&timestamp) => {
                if now.duration_since(timestamp) >= resolution {
                    last_sent.insert(key.clone(), now);
                    true
                } else {
                    false
                }
            }
            None => {
                last_sent.insert(key.clone(), now);
                true
            }
        }
    }

    async fn update_last_sent(&mut self, element_id: &str, metric_id: &str) {
        let key = (element_id.to_string(), metric_id.to_string());
        let now = Instant::now();
        let mut last_sent = self.last_sent.write().await;
        last_sent.insert(key, now);
    }
}

// Dummy cache strategy for demonstration purposes
pub struct DummyCacheStrategy;

impl DummyCacheStrategy {
    pub fn new() -> Self {
        DummyCacheStrategy
    }
}

#[async_trait]
impl CacheStrategy for DummyCacheStrategy {
    async fn should_send(&self, _element: &str, _metric: &str, _interval: Duration) -> bool {
        true
    }

    async fn update_last_sent(&mut self, _element: &str, _metric: &str) {}
}
