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
    /// Replay retained observations at a fixed UTC time and print privacy-safe counts.
    Replay {
        #[arg(long)]
        database: PathBuf,
        /// RFC 3339 timestamp, for example 2026-10-21T12:00:00Z.
        #[arg(long)]
        at: chrono::DateTime<chrono::Utc>,
    },
    /// Evaluate the deterministic engine on fixed synthetic cases.
    Eval {
        #[arg(long, required = true)]
        mock: bool,
        #[arg(long, default_value = "target/eval/mock-report.json")]
        report: PathBuf,
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
        Command::Replay { database, at } => {
            use picket_core::engine::{ActorEvidence, Route};
            let events: Vec<_> = picket_core::store::Store::open_read_only(&database)?
                .events()?
                .into_iter()
                .map(|row| row.event)
                .collect();
            let assessments = picket_core::engine::assess(&events, at);
            let count = |predicate: fn(&picket_core::engine::Assessment) -> bool| {
                assessments
                    .iter()
                    .filter(|assessment| predicate(assessment))
                    .count()
            };
            println!(
                "replayed {} effects: {} high review, {} medium review, {} timeline; {} actor unknown, {} reported, {} runtime linked, {} conflicting, {} user confirmed",
                assessments.len(),
                count(|a| a.route == Route::HighReview),
                count(|a| a.route == Route::MediumReview),
                count(|a| a.route == Route::Timeline),
                count(|a| a.actor == ActorEvidence::Unknown),
                count(|a| matches!(a.actor, ActorEvidence::Reported(_))),
                count(|a| matches!(a.actor, ActorEvidence::RuntimeLinked(_))),
                count(|a| matches!(a.actor, ActorEvidence::Conflicting(_))),
                count(|a| matches!(a.actor, ActorEvidence::UserConfirmed(_))),
            );
        }
        Command::Eval { mock: _, report } => {
            let result = picket_core::eval::mock_report()?;
            if let Some(parent) = report.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&report, serde_json::to_vec_pretty(&result)?)?;
            println!(
                "mode           cases   agent    link   conflict  high   medium  false/benign  gap"
            );
            for (mode, counts) in &result.modes {
                println!(
                    "{mode:<14?} {:>5} {:>3}/{:<3} {:>3}/{:<3} {:>3}/{:<3} {:>3}/{:<3} {:>3}/{:<3} {:>3}/{:<3} {:>3}/{:<3}",
                    counts.cases,
                    counts.correct_named_agents,
                    counts.named_agents,
                    counts.linked_effects,
                    counts.linkable_effects,
                    counts.detected_conflicts,
                    counts.explicit_conflicts,
                    counts.correct_high,
                    counts.expected_high,
                    counts.correct_medium,
                    counts.expected_medium,
                    counts.alerts_on_benign,
                    counts.benign_effects,
                    counts.surfaced_coverage_gaps,
                    counts.applicable_coverage_gaps,
                );
            }
            println!(
                "mock cases: {}; gates: {}; report: {}",
                result.totals.cases,
                if result.gates_passed { "pass" } else { "FAIL" },
                report.display()
            );
            if !result.gates_passed {
                return Err(
                    format!("mock evaluation failed: {}", result.failures.join(", ")).into(),
                );
            }
        }
    }
    Ok(())
}
