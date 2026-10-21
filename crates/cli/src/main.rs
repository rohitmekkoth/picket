use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Parses the diagnostic command without starting the desktop runtime.
#[derive(Parser)]
#[command(version, about = "Local diagnostics for Picket")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

/// Keeps diagnostics explicit as operational commands are added.
#[derive(Subcommand)]
enum Command {
    /// Print runtime information without accessing any account or personal data.
    Doctor,
    /// Verify the first broken sequence in a local Picket event log.
    Verify {
        #[arg(long)]
        database: PathBuf,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("picket: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    match Args::parse().command {
        Command::Doctor => println!(
            "{}",
            serde_json::to_string_pretty(&picket_core::runtime_info())?
        ),
        Command::Verify { database } => {
            let result = picket_core::store::Store::open_read_only(&database)?.verify()?;
            println!(
                "log valid through sequence {} ({} retained events)",
                result.through_sequence, result.retained_events
            );
        }
    }
    Ok(())
}
