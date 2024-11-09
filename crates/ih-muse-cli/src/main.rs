// crates/ih-muse-cli/src/main.rs

use clap::{Parser, Subcommand};

mod commands;
mod common;

use commands::{
    is_ready::IsReadyArgs, record::RecordArgs, register_element::RegisterElementArgs,
    register_element_kind::RegisterElementKindArgs, register_metric::RegisterMetricArgs,
    replay::ReplayArgs, send_metric::SendMetricArgs,
};

#[derive(Parser)]
#[command(name = "ih-muse-cli", version = "0.1.0", about = "CLI for ih-muse")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Check if the poet server is up and healthy
    IsReady(IsReadyArgs),
    /// Register an element kind with the poet server
    RegisterElementKind(RegisterElementKindArgs),
    /// Register an element with the poet server
    RegisterElement(RegisterElementArgs),
    /// Register a metric definition
    RegisterMetric(RegisterMetricArgs),
    /// Register a metric with the poet server
    SendMetric(SendMetricArgs),
    /// Record a session
    Record(RecordArgs),
    /// Replay a recorded session
    Replay(ReplayArgs),
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::IsReady(args) => commands::is_ready::execute(args).await,
        Commands::RegisterElement(args) => commands::register_element::execute(args).await,
        Commands::RegisterElementKind(args) => commands::register_element_kind::execute(args).await,
        Commands::RegisterMetric(args) => commands::register_metric::execute(args).await,
        Commands::SendMetric(args) => commands::send_metric::execute(args).await,
        Commands::Record(args) => commands::record::execute(args).await,
        Commands::Replay(args) => commands::replay::execute(args).await,
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
