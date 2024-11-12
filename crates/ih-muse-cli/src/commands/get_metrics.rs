// crates/ih-muse-cli/src/commands/register_metric.rs

use clap::Args;

use super::utils::create_poet_client;
use crate::common::CommonArgs;
use ih_muse_core::{Error, Transport};
use ih_muse_proto::MetricQuery;

#[derive(Args)]
pub struct GetMetricsArgs {
    #[clap(flatten)]
    pub common: CommonArgs,
}

pub async fn execute(args: GetMetricsArgs) -> Result<(), Error> {
    let client = create_poet_client(&args.common.poet_url);

    match client.get_metrics(&MetricQuery::default()).await {
        Ok(metrics) => {
            println!("Getting all Metrics:");
            for metric in metrics {
                println!("  - {:?}", metric);
            }
        }
        Err(e) => {
            eprintln!("Failed to get metrics: {}", e);
            return Err(e);
        }
    }

    Ok(())
}
