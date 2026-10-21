use std::process::Command;

use picket_core::{event::Event, store::Store};
use serde_json::json;

#[test]
fn verify_reports_first_broken_sequence_to_the_user() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let database = directory.path().join("synthetic.db");
    let mut store = Store::open(&database)?;
    for number in 1..=3 {
        let event: Event = serde_json::from_value(json!({
            "schema_version": 1,
            "key": {
                "account": "synthetic-account",
                "source": "gmail",
                "resource": format!("message-{number}"),
                "revision": "1"
            },
            "occurred_at": "2026-10-21T12:00:00Z",
            "ingested_at": "2026-10-21T12:00:00Z",
            "payload": {
                "type": "effect",
                "data": {
                    "action": "email_send",
                    "targets": ["person@example.test"],
                    "targets_complete": true,
                    "origin": "live",
                    "external_change": false
                }
            }
        }))?;
        store.append(&event)?;
    }
    drop(store);
    let clean = Command::new(env!("CARGO_BIN_EXE_picket"))
        .args(["verify", "--database"])
        .arg(&database)
        .output()?;
    assert!(clean.status.success());
    assert!(String::from_utf8_lossy(&clean.stdout).contains("through sequence 3"));

    let connection = rusqlite::Connection::open(&database)?;
    connection.execute("DELETE FROM events WHERE seq=2", [])?;
    drop(connection);
    let damaged = Command::new(env!("CARGO_BIN_EXE_picket"))
        .args(["verify", "--database"])
        .arg(&database)
        .output()?;
    assert!(!damaged.status.success());
    assert!(String::from_utf8_lossy(&damaged.stderr).contains("sequence 2"));
    Ok(())
}
