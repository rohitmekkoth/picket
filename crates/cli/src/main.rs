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
    /// Print a text-free timeline of captured, extracted, reported and observed inputs
    /// since a time, with the engine's assessment, for live validation runs.
    Review {
        #[arg(long)]
        database: PathBuf,
        /// RFC 3339 timestamp; earlier inputs are left out of the timeline.
        #[arg(long)]
        since: chrono::DateTime<chrono::Utc>,
        /// Assess as of this RFC 3339 timestamp instead of now.
        #[arg(long)]
        at: Option<chrono::DateTime<chrono::Utc>>,
    },
    /// Count recorded undecodable agent messages that the current decoder can read.
    /// Prints counts only; needs Full Disk Access to read Messages.
    MessagesCheck {
        #[arg(long)]
        database: PathBuf,
        /// Defaults to ~/Library/Messages/chat.db.
        #[arg(long)]
        chat_database: Option<PathBuf>,
    },
    /// Evaluate the deterministic engine on fixed synthetic cases.
    Eval {
        #[arg(long, required = true)]
        mock: bool,
        #[arg(long, default_value = "target/eval/mock-report.json")]
        report: PathBuf,
    },
    /// Re-extract an agent conversation since a time with the current extractor, in a
    /// scratch copy of the log, and print its review timeline. Reads message text from
    /// Messages, so it needs Full Disk Access; the database itself is never written.
    #[cfg(feature = "model")]
    ReplayExtraction {
        #[arg(long)]
        database: PathBuf,
        #[arg(long)]
        since: chrono::DateTime<chrono::Utc>,
        #[arg(long)]
        model_file: PathBuf,
        #[arg(long, default_value = "qwen3.5-4b-q4_k_m")]
        model: String,
        /// Defaults to ~/Library/Messages/chat.db.
        #[arg(long)]
        chat_database: Option<PathBuf>,
    },
    /// Evaluate a local model on the labeled synthetic extraction suite.
    #[cfg(feature = "model")]
    ExtractionEval {
        /// One of the pinned candidate ids, for example qwen3.5-4b-q4_k_m.
        #[arg(long)]
        model: String,
        #[arg(long)]
        model_file: PathBuf,
        #[arg(long, default_value = "eval/extraction/cases.json")]
        suite: PathBuf,
        #[arg(long, default_value = "target/eval/extraction-report.json")]
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
        Command::MessagesCheck {
            database,
            chat_database,
        } => {
            use picket_core::event::{EventPayload, MessageText};
            let chat_path = match chat_database {
                Some(path) => path,
                None => PathBuf::from(std::env::var("HOME")?).join("Library/Messages/chat.db"),
            };
            let chat = picket_core::messages::ChatDb::open(&chat_path)
                .map_err(|error| format!("Messages unreadable: {error}"))?;
            let undecodable: std::collections::BTreeSet<_> =
                picket_core::store::Store::open_read_only(&database)?
                    .events()?
                    .into_iter()
                    .filter(|row| {
                        matches!(&row.event.payload, EventPayload::Message(message)
                            if message.text == MessageText::Undecodable)
                    })
                    .map(|row| row.event.key.resource)
                    .collect();
            let readable = undecodable
                .iter()
                .filter(|guid| chat.text(guid).is_some())
                .count();
            println!(
                "{} messages recorded as undecodable; {readable} decode with the current decoder",
                undecodable.len()
            );
        }
        Command::Review {
            database,
            since,
            at,
        } => {
            let events: Vec<_> = picket_core::store::Store::open_read_only(&database)?
                .events()?
                .into_iter()
                .map(|row| row.event)
                .collect();
            let at = at.unwrap_or_else(chrono::Utc::now);
            for line in picket_core::review::timeline(&events, since, at) {
                println!("{line}");
            }
        }
        #[cfg(feature = "model")]
        Command::ReplayExtraction {
            database,
            since,
            model_file,
            model,
            chat_database,
        } => {
            use chrono::{Local, Offset, TimeZone};
            use picket_core::{event::EventPayload, ledger::Progress, store::Store};
            let chat_path = match chat_database {
                Some(path) => path,
                None => PathBuf::from(std::env::var("HOME")?).join("Library/Messages/chat.db"),
            };
            let chat = picket_core::messages::ChatDb::open(&chat_path)
                .map_err(|error| format!("Messages unreadable: {error}"))?;
            let events: Vec<_> = Store::open_read_only(&database)?
                .events()?
                .into_iter()
                .map(|row| row.event)
                .collect();
            let account = events
                .iter()
                .find_map(|event| match event.payload {
                    EventPayload::Message(_) => Some(event.key.account.clone()),
                    _ => None,
                })
                .ok_or("no captured messages")?;
            let mut scratch = Store::memory()?;
            for event in picket_core::review::without_extractions_since(events, since) {
                scratch.append(&event)?;
            }
            let spec = picket_model::spec(&model).ok_or("unknown model id")?;
            let mut runner = picket_model::Runner::load(&model_file, spec)?;
            let zone = |at: chrono::DateTime<chrono::Utc>| {
                Local.offset_from_utc_datetime(&at.naive_utc()).fix()
            };
            let mut text = |guid: &str| chat.text(guid);
            loop {
                let progress = picket_core::extract::extract_pending(
                    &mut scratch,
                    Some(&mut runner),
                    &mut text,
                    &zone,
                    &account,
                    chrono::Utc::now(),
                )?;
                if progress == Progress::CaughtUp {
                    break;
                }
            }
            let replayed: Vec<_> = scratch.events()?.into_iter().map(|row| row.event).collect();
            for line in picket_core::review::timeline(&replayed, since, chrono::Utc::now()) {
                println!("{line}");
            }
        }
        #[cfg(feature = "model")]
        Command::ExtractionEval {
            model,
            model_file,
            suite,
            report,
        } => {
            let spec = picket_model::spec(&model).ok_or("unknown model id")?;
            let suite = picket_core::extract_eval::load(&std::fs::read_to_string(suite)?)?;
            let mut runner = picket_model::Runner::load(&model_file, spec)?;
            let started = std::time::Instant::now();
            let result = picket_core::extract_eval::evaluate(&mut runner, &suite)?;
            let elapsed = started.elapsed();
            if let Some(parent) = report.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&report, serde_json::to_vec_pretty(&result)?)?;
            for line in &result.safety_errors {
                println!("safety  {line}");
            }
            for line in &result.utility_misses {
                println!("miss    {line}");
            }
            println!(
                "{}: {} cases, {} safety errors, {} utility misses, {} failed calls, {:.1}s per case",
                result.model,
                result.cases,
                result.safety_errors.len(),
                result.utility_misses.len(),
                result.failed_calls,
                elapsed.as_secs_f64() / result.cases.max(1) as f64
            );
        }
        Command::Eval { mock: _, report } => {
            let result = picket_core::eval::mock_report()?;
            if let Some(parent) = report.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&report, serde_json::to_vec_pretty(&result)?)?;
            // Derived Debug output ignores width, so format names before padding them.
            let ratio = |hit: usize, total: usize| format!("{hit}/{total}");
            println!(
                "{:<14}{:>6}{:>9}{:>9}{:>10}{:>9}{:>9}{:>14}{:>8}",
                "mode",
                "cases",
                "agent",
                "link",
                "conflict",
                "high",
                "medium",
                "false/benign",
                "gap"
            );
            for (mode, counts) in &result.modes {
                println!(
                    "{:<14}{:>6}{:>9}{:>9}{:>10}{:>9}{:>9}{:>14}{:>8}",
                    format!("{mode:?}"),
                    counts.cases,
                    ratio(counts.correct_named_agents, counts.named_agents),
                    ratio(counts.linked_effects, counts.linkable_effects),
                    ratio(counts.detected_conflicts, counts.explicit_conflicts),
                    ratio(counts.correct_high, counts.expected_high),
                    ratio(counts.correct_medium, counts.expected_medium),
                    ratio(counts.alerts_on_benign, counts.benign_effects),
                    ratio(
                        counts.surfaced_coverage_gaps,
                        counts.applicable_coverage_gaps
                    ),
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
