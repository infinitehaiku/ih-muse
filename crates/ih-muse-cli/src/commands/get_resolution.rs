// crates/ih-muse-cli/src/commands/register_metric.rs

use clap::Args;
use ih_muse_core::{Error, Transport};

use crate::common::CommonArgs;

#[derive(Args)]
pub struct GetFinestResolutionArgs {
    #[clap(flatten)]
    pub common: CommonArgs,
}

pub async fn execute(args: GetFinestResolutionArgs) -> Result<(), Error> {
    let client = super::utils::create_poet_client(&args.common.poet_url);
    let finest_resolution = client.get_finest_resolution().await?;
    println!("Finest Resolution: {finest_resolution}");
    Ok(())
}
