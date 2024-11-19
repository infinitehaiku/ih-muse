// crates/ih-muse-cli/src/commands/register_element_kind.rs

use clap::Args;
use ih_muse_core::{MuseResult, Transport};
use ih_muse_proto::ElementKindRegistration;

use crate::common::CommonArgs;

#[derive(Args)]
pub struct RegisterElementKindArgs {
    #[clap(flatten)]
    pub common: CommonArgs,

    /// Element kind code
    #[arg(short, long)]
    pub code: String,

    /// Parent element kind code
    #[arg(short, long)]
    pub parent_code: Option<String>,

    /// Element kind name
    #[arg(short, long)]
    pub name: String,

    /// Element kind name
    #[arg(short, long)]
    pub description: String,
}

pub async fn execute(args: RegisterElementKindArgs) -> MuseResult<()> {
    let client = super::utils::create_poet_client(&args.common.poet_url);

    let payload = [ElementKindRegistration::new(
        &args.code,
        args.parent_code.as_deref(),
        &args.name,
        &args.description,
    )];

    client.register_element_kinds(&payload).await?;
    println!("Element kind registered successfully");
    Ok(())
}
