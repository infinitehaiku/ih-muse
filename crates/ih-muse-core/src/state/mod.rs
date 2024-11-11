// crates/ih-muse/src/state.rs

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::sync::OnceLock;

use arc_swap::ArcSwap;
use imbl::{HashMap, HashSet, Vector};
use tokio::sync::RwLock;
use uuid::Uuid;

use ih_muse_proto::*;

pub struct State {
    pub nodes: RwLock<HashMap<Uuid, SocketAddr>>,
    pub element_kinds: OnceLock<Arc<HashSet<String>>>,
    pub registered_metrics: OnceLock<Arc<HashMap<String, Arc<MetricDefinition>>>>,
    pub metric_order: Arc<ArcSwap<Vector<Arc<MetricDefinition>>>>,
    pub min_element_id: RwLock<Option<ElementId>>,
    pub max_element_id: RwLock<Option<ElementId>>,
    pub node_elem_ranges: RwLock<Vec<NodeElementRange>>,
    pub finest_resolution: AtomicU8,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    pub fn new() -> Self {
        Self {
            nodes: RwLock::new(HashMap::new()),
            element_kinds: OnceLock::new(),
            registered_metrics: OnceLock::new(),
            metric_order: Arc::new(ArcSwap::from_pointee(Vector::new())),
            min_element_id: RwLock::new(None),
            max_element_id: RwLock::new(None),
            node_elem_ranges: RwLock::new(Vec::new()),
            finest_resolution: TimestampResolution::default().as_u8().into(),
        }
    }

    /// Inits `element_kinds` only once. Subsequent calls will return an error.
    pub async fn init_element_kinds(&self, element_kinds: &[ElementKindRegistration]) {
        let codes_set = element_kinds
            .iter()
            .map(|kind| kind.code.clone())
            .collect::<HashSet<String>>();
        let _ = self.element_kinds.set(Arc::new(codes_set));
    }

    /// Check if an element kind code is valid.
    pub fn is_valid_element_kind_code(&self, element_kind_code: &str) -> bool {
        if let Some(kinds) = self.element_kinds.get() {
            kinds.contains(element_kind_code)
        } else {
            false
        }
    }

    /// Inits `registered_metrics` only once. Subsequent calls will return an error.
    pub async fn init_metrics(&self, metric_definitions: &[MetricDefinition]) {
        let metrics = metric_definitions
            .iter()
            .map(|m| (m.code.clone(), Arc::new(m.clone()))) // Map code to an Arc<MetricDefinition>
            .collect::<HashMap<String, Arc<MetricDefinition>>>();

        // Attempt to set `registered_metrics` only once
        let _ = self.registered_metrics.set(Arc::new(metrics));
    }

    /// Check if a metric code is valid.
    pub fn is_valid_metric_code(&self, metric_code: &str) -> bool {
        // Load the Arc<HashMap> from `registered_metrics` and check for the presence of the code
        self.registered_metrics
            .get()
            .map_or(false, |metrics| metrics.contains_key(metric_code))
    }

    /// Update `metric_order` atomically with a new order.
    /// ! This is only updated in one tasks, no concurrency issues
    /// * It can safely being read from multiple threads
    pub async fn update_metric_order(&self, metric_order: Vec<MetricDefinition>) {
        let ordered_metrics = metric_order
            .into_iter()
            .map(Arc::new) // Wrap each MetricDefinition in an Arc
            .collect::<Vector<_>>(); // Collect into an imbl Vector
        self.metric_order.store(Arc::new(ordered_metrics));
    }

    /// Update `finest_resolution` atomically.
    pub async fn update_finest_resolution(&self, finest_resolution: TimestampResolution) {
        self.finest_resolution
            .store(finest_resolution.as_u8(), Ordering::SeqCst);
    }

    /// Retrieve the current `finest_resolution` as `TimestampResolution`.
    pub fn get_finest_resolution(&self) -> TimestampResolution {
        TimestampResolution::from_u8(self.finest_resolution.load(Ordering::SeqCst))
    }
}
