// crates/ih-muse/src/tasks/init_task.rs

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::select;
use tokio_util::sync::CancellationToken;

use ih_muse_core::{Error, State, Transport};
use ih_muse_proto::{ElementKindRegistration, MetricDefinition};

pub async fn start_init_task(
    cancellation_token: CancellationToken,
    client: Arc<dyn Transport + Send + Sync>,
    state: Arc<State>,
    element_kinds: Vec<ElementKindRegistration>,
    metric_definitions: Vec<MetricDefinition>,
    is_initialized: Arc<AtomicBool>,
) {
    // Track the initialization step
    let mut step = InitializationStep::HealthCheck;

    loop {
        select! {
            _ = cancellation_token.cancelled() => {
                eprintln!("Initialization task was cancelled.");
                break;
            }
            result = perform_initialization_step(
                &client,
                &state,
                &element_kinds,
                &metric_definitions,
                &mut step,
            ) => {
                match result {
                    Ok(_) => {
                        if step == InitializationStep::Done {
                            is_initialized.store(true, Ordering::SeqCst);
                            println!("Initialization completed successfully.");
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("Initialization error at step {:?}: {:?}", step, e);
                        // Wait before retrying the same step
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        }
    }
}

#[derive(Debug, PartialEq)]
enum InitializationStep {
    HealthCheck,
    RegisterElementKinds,
    RegisterMetrics,
    GetMetricOrder,
    GetFinestResolution,
    Done,
}

async fn perform_initialization_step(
    client: &Arc<dyn Transport + Send + Sync>,
    state: &Arc<State>,
    element_kinds: &[ElementKindRegistration],
    metric_definitions: &[MetricDefinition],
    step: &mut InitializationStep,
) -> Result<(), Error> {
    match *step {
        InitializationStep::HealthCheck => {
            client.health_check().await?;
            *step = InitializationStep::RegisterElementKinds;
        }
        InitializationStep::RegisterElementKinds => {
            if !element_kinds.is_empty() {
                client.register_element_kinds(element_kinds).await?;
                state.init_element_kinds(element_kinds).await;
            }
            *step = InitializationStep::RegisterMetrics;
        }
        InitializationStep::RegisterMetrics => {
            if !metric_definitions.is_empty() {
                client.register_metrics(metric_definitions).await?;
                state.init_metrics(metric_definitions).await;
            }
            *step = InitializationStep::GetMetricOrder;
        }
        InitializationStep::GetMetricOrder => {
            let metric_order = client.get_metric_order().await?;
            state.update_metric_order(metric_order).await;
            *step = InitializationStep::GetFinestResolution;
        }
        InitializationStep::GetFinestResolution => {
            let finest_resolution = client.get_finest_resolution().await?;
            state.update_finest_resolution(finest_resolution).await;
            *step = InitializationStep::Done;
        }
        InitializationStep::Done => {}
    }
    Ok(())
}
