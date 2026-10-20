use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about = "Local diagnostics for Picket")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print runtime information without accessing any account or personal data.
    Doctor,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Args::parse().command {
        Command::Doctor => println!(
            "{}",
            serde_json::to_string_pretty(&picket_core::runtime_info())?
        ),
    }
    Ok(())
}
