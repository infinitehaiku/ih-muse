// crates/ih-muse-cli/src/commands/replay.rs

use std::path::PathBuf;

use clap::Args;

use ih_muse_core::MuseResult;
use ih_muse_record::FileReplayer;

#[derive(Args)]
pub struct ReplayArgs {
    /// Input file path
    #[arg(short, long)]
    pub input: PathBuf,
    // Additional arguments...
}

pub async fn execute(args: ReplayArgs) -> MuseResult<()> {
    let _replayer = FileReplayer::new(&args.input)?;
    // TODO
    // Create instance of ih_muse::Muse and replay
    Ok(())
}
