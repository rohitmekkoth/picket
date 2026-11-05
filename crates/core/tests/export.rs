use chrono::{DateTime, TimeZone, Utc};
use picket_core::{
    event::{AccountPolicy, Action, Effect, Event, EventKey, EventPayload, Origin, Source},
    export,
    store::Store,
};

fn at(minute: i64) -> DateTime<Utc> {
    Utc.timestamp_nanos((1_700_000_000 + minute * 60) * 1_000_000_000)
}

fn event(source: Source, resource: &str, minute: i64, payload: EventPayload) -> Event {
    Event {
        schema_version: 1,
        key: EventKey {
            account: "synthetic".into(),
            source,
            resource: resource.into(),
            revision: "1".into(),
        },
        occurred_at: at(minute),
        ingested_at: at(minute),
        source_time_quality: None,
        adapter: None,
        payload,
    }
}

fn store_with_incident() -> Result<Store, Box<dyn std::error::Error>> {
    let mut store = Store::memory()?;
    let policy = AccountPolicy {
        id: "no-send".into(),
        forbidden_actions: vec![Action::EmailSend],
        forbidden_targets: vec![],
        high_consequence: true,
        enabled: true,
    };
    store.append_assessed(
        &event(Source::Picket, "policy-1", 0, EventPayload::Policy(policy)),
        at(0),
    )?;
    let effect = Effect {
        target_names: Default::default(),
        action: Action::EmailSend,
        targets: vec!["sam@example.test".into()],
        targets_complete: true,
        calendar_start: None,
        origin: Origin::Live,
        external_change: false,
        email: None,
        calendar: None,
    };
    store.append_assessed(
        &event(Source::Gmail, "message-1", 10, EventPayload::Effect(effect)),
        at(10),
    )?;
    Ok(store)
}

#[test]
fn an_incident_exports_as_a_readable_report() -> Result<(), Box<dyn std::error::Error>> {
    let mut store = store_with_incident()?;
    let id = store.incidents()?.first().ok_or("no incident")?.id.clone();
    let detail = store.incident(&id)?.ok_or("no detail")?;
    let report = export::incident_markdown(&detail, at(20), &Utc, "0.0.0-test");
    assert!(report.starts_with("# "), "{report}");
    for section in [
        "- Severity: Urgent",
        "## Why this was flagged",
        "## What you asked",
        "## What happened",
    ] {
        assert!(report.contains(section), "missing {section}:\n{report}");
    }
    assert!(
        report.contains("Sent an email to sam@example.test"),
        "{report}"
    );
    assert!(report.contains("Exported by Picket 0.0.0-test"));
    Ok(())
}

#[test]
fn the_full_export_is_one_json_document_with_every_event() -> Result<(), Box<dyn std::error::Error>>
{
    let store = store_with_incident()?;
    let mut out = Vec::new();
    export::write_all(&store, &mut out, at(20), "0.0.0-test")?;
    let value: serde_json::Value = serde_json::from_slice(&out)?;
    assert_eq!(value["format"], "picket-export");
    assert_eq!(value["events"].as_array().map(Vec::len), Some(2));
    assert_eq!(value["events"][0]["sequence"], 1);
    assert_eq!(value["events"][0]["hash"].as_str().map(str::len), Some(64));
    assert!(
        value["findings"]
            .as_array()
            .is_some_and(|findings| !findings.is_empty())
    );
    assert_eq!(value["settings"]["retentionDays"], 180);
    Ok(())
}
