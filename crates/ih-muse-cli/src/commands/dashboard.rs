// crates/ih-muse-cli/src/commands/dashboard.rs

//! `dashboard check` and `dashboard schema`: offline tools for Muse authors
//! writing dashboard definitions.

use std::path::PathBuf;

use clap::{Args, Subcommand};
use ih_muse_proto::dashboard::dashboard_definition_schema;
use ih_muse_proto::DashboardDefinition;

/// Arguments of the `dashboard` command group.
#[derive(Args)]
pub struct DashboardArgs {
    #[command(subcommand)]
    pub command: DashboardCommand,
}

/// Offline dashboard definition tools.
#[derive(Subcommand)]
pub enum DashboardCommand {
    /// Parse and validate dashboard definition files; exits non-zero on any failure
    Check {
        /// Definition files (JSON, one DashboardDefinition each)
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Print the JSON Schema of a dashboard definition
    Schema,
}

/// Runs the command and returns the process exit code.
pub fn execute(args: DashboardArgs) -> i32 {
    match args.command {
        DashboardCommand::Check { files } => {
            let mut failed = false;
            for file in &files {
                match check_file(file) {
                    Ok(definition) => println!(
                        "OK    {}: {} revision {} ({} panels, {} blocks)",
                        file.display(),
                        definition.id,
                        definition.revision,
                        definition.panels.len(),
                        definition.blocks.len()
                    ),
                    Err(error) => {
                        failed = true;
                        println!("ERROR {}: {error}", file.display());
                    }
                }
            }
            i32::from(failed)
        }
        DashboardCommand::Schema => {
            let schema = dashboard_definition_schema();
            println!(
                "{}",
                serde_json::to_string_pretty(&schema).expect("schema serializes")
            );
            0
        }
    }
}

/// Reads, parses and validates one definition file.
fn check_file(file: &PathBuf) -> Result<DashboardDefinition, String> {
    let text = std::fs::read_to_string(file).map_err(|error| format!("read: {error}"))?;
    let definition: DashboardDefinition =
        serde_json::from_str(&text).map_err(|error| format!("parse: {error}"))?;
    definition
        .validate()
        .map_err(|error| format!("invalid: {error}"))?;
    Ok(definition)
}
