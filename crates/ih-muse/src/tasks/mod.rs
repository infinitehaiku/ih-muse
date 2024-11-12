// crates/ih-muse/src/tasks/mod.rs

mod cluster_monitor;
mod element_registration;
mod init_task;
mod intervals;
mod metric_sender;

pub use cluster_monitor::start_cluster_monitor;
pub use element_registration::start_element_registration_task;
pub use init_task::start_init_task;
pub use intervals::calculate_interval_duration;
pub use metric_sender::start_metric_sender_task;
