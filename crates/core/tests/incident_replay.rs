use chrono::{DateTime, TimeZone, Utc};
use picket_core::{
    engine::{ActorEvidence, FindingDetail, Route},
    event::{
        AccountPolicy, Action, Coverage, CoverageState, Effect, Event, EventKey, EventPayload,
        Feedback, NoticeKind, Origin, Source,
    },
    incidents::OutboxKind,
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
        payload,
    }
}

fn policy() -> Event {
    event(
        Source::Picket,
        "policy-1",
        0,
        EventPayload::Policy(AccountPolicy {
            id: "no-send".into(),
            forbidden_actions: vec![Action::EmailSend],
            forbidden_targets: vec![],
            high_consequence: true,
            enabled: true,
        }),
    )
}

fn effect() -> Event {
    event(
        Source::Gmail,
        "message-1",
        10,
        EventPayload::Effect(Effect {
            action: Action::EmailSend,
            targets: vec!["a@example.test".into()],
            targets_complete: true,
            calendar_start: None,
            origin: Origin::Live,
            external_change: false,
        }),
    )
}

#[test]
fn restart_and_retry_keep_one_alert_and_late_feedback_creates_correction()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("synthetic.db");
    let mut store = Store::open(&path)?;
    store.append_assessed(&policy(), at(0))?;
    let (_, first) = store.append_assessed(&effect(), at(10))?;
    assert_eq!(first.queued, 1);
    let initial = store.pending_notifications()?;
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0].kind, OutboxKind::Alert);
    let first_id = initial[0].id.clone();
    assert_eq!(store.append_assessed(&effect(), at(10))?.1.queued, 0);
    drop(store);

    let mut reopened = Store::open(&path)?;
    assert_eq!(reopened.reconcile(at(10))?.revised, 0);
    assert_eq!(reopened.pending_notifications()?.len(), 1);
    assert!(reopened.mark_notification_delivered(&first_id)?);
    assert!(!reopened.mark_notification_delivered(&first_id)?);
    let finding = reopened
        .current_findings()?
        .into_iter()
        .find_map(|(_, finding)| {
            matches!(finding.detail, FindingDetail::Effect(_)).then_some(finding.id)
        })
        .ok_or("effect finding missing")?;
    let feedback = event(
        Source::Picket,
        "feedback-1",
        11,
        EventPayload::Feedback(Feedback {
            finding,
            expected: Some(true),
            actor: None,
        }),
    );
    reopened.append_assessed(&feedback, at(11))?;
    let pending = reopened.pending_notifications()?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].kind, OutboxKind::Correction);
    assert_eq!(reopened.reconcile(at(20))?.queued, 0);
    assert_eq!(reopened.pending_notifications()?.len(), 1);
    let (_, assessment) = reopened
        .current_findings()?
        .into_iter()
        .find(|(_, finding)| matches!(finding.detail, FindingDetail::Effect(_)))
        .ok_or("effect finding missing")?;
    assert_eq!(assessment.route, Route::Timeline);
    if let FindingDetail::Effect(value) = assessment.detail {
        assert_eq!(value.actor, ActorEvidence::Unknown);
    }
    Ok(())
}

#[test]
fn medium_review_waits_and_replay_does_not_duplicate_outbox()
-> Result<(), Box<dyn std::error::Error>> {
    let mut store = Store::memory()?;
    let mut medium = policy();
    if let EventPayload::Policy(value) = &mut medium.payload {
        value.high_consequence = false;
    }
    store.append_assessed(&medium, at(0))?;
    store.append_assessed(&effect(), at(10))?;
    assert!(store.pending_notifications()?.is_empty());
    assert_eq!(store.reconcile(at(15))?.queued, 1);
    assert_eq!(store.reconcile(at(15))?.queued, 0);
    assert_eq!(store.pending_notifications()?.len(), 1);
    Ok(())
}

#[test]
fn legacy_store_migrates_without_losing_events() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("legacy.db");
    let mut original = Store::open(&path)?;
    original.append(&policy())?;
    drop(original);
    let connection = rusqlite::Connection::open(&path)?;
    connection.execute_batch(
        "DROP TABLE collector_cursors;
         DROP TABLE engine_state;
         DROP TABLE notification_outbox;
         DROP TABLE assessment_current;
         DROP TABLE assessment_revisions;
         PRAGMA user_version=1;",
    )?;
    drop(connection);
    let mut migrated = Store::open(&path)?;
    assert_eq!(migrated.verify()?.retained_events, 1);
    migrated.append_assessed(&effect(), at(10))?;
    assert_eq!(migrated.pending_notifications()?.len(), 1);
    Ok(())
}

#[test]
fn failed_outbox_write_rolls_back_effect_and_assessment() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("atomic.db");
    let mut store = Store::open(&path)?;
    store.append_assessed(&policy(), at(0))?;
    drop(store);
    let connection = rusqlite::Connection::open(&path)?;
    connection.execute_batch(
        "CREATE TRIGGER reject_outbox BEFORE INSERT ON notification_outbox
         BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END;",
    )?;
    drop(connection);
    let mut store = Store::open(&path)?;
    assert!(store.append_assessed(&effect(), at(10)).is_err());
    assert_eq!(store.verify()?.retained_events, 1);
    assert!(store.current_findings()?.is_empty());
    assert!(store.pending_notifications()?.is_empty());
    Ok(())
}

#[test]
fn expected_feedback_cancels_an_undelivered_alert_without_new_notification()
-> Result<(), Box<dyn std::error::Error>> {
    let mut store = Store::memory()?;
    store.append_assessed(&policy(), at(0))?;
    store.append_assessed(&effect(), at(10))?;
    let finding = store
        .current_findings()?
        .into_iter()
        .find_map(|(_, item)| matches!(item.detail, FindingDetail::Effect(_)).then_some(item.id))
        .ok_or("effect finding missing")?;
    let feedback = event(
        Source::Picket,
        "feedback-1",
        11,
        EventPayload::Feedback(Feedback {
            finding,
            expected: Some(true),
            actor: None,
        }),
    );
    store.append_assessed(&feedback, at(11))?;
    assert!(store.pending_notifications()?.is_empty());
    Ok(())
}

#[test]
fn collector_gap_is_a_separate_finding_and_recovery_corrects_it()
-> Result<(), Box<dyn std::error::Error>> {
    let mut store = Store::memory()?;
    let gap = event(
        Source::Gmail,
        "coverage-1",
        10,
        EventPayload::Coverage(Coverage {
            capability: "effects".into(),
            state: CoverageState::Gap,
            cursor: None,
        }),
    );
    store.append_assessed(&gap, at(10))?;
    let alert = store.pending_notifications()?;
    assert_eq!(alert.len(), 1);
    assert_eq!(alert[0].kind, OutboxKind::Alert);
    assert!(store.mark_notification_delivered(&alert[0].id)?);
    let healthy = event(
        Source::Gmail,
        "coverage-2",
        20,
        EventPayload::Coverage(Coverage {
            capability: "effects".into(),
            state: CoverageState::Healthy,
            cursor: Some("synthetic-cursor".into()),
        }),
    );
    store.append_assessed(&healthy, at(20))?;
    let correction = store.pending_notifications()?;
    assert_eq!(correction.len(), 1);
    assert_eq!(correction[0].kind, OutboxKind::Correction);
    assert!(store.current_findings()?.iter().any(
        |(_, finding)| matches!(&finding.detail, FindingDetail::Coverage(value)
            if value.evidence.state == CoverageState::Healthy)
    ));
    Ok(())
}

#[test]
fn revisions_of_one_resource_share_an_incident_group_without_losing_effects()
-> Result<(), Box<dyn std::error::Error>> {
    let mut first = effect();
    first.key.source = Source::Calendar;
    let mut second = first.clone();
    second.key.revision = "2".into();
    second.occurred_at = at(12);
    second.ingested_at = at(12);
    let mut store = Store::memory()?;
    store.append_assessed(&first, at(10))?;
    store.append_assessed(&second, at(12))?;
    let effects: Vec<_> = store
        .current_findings()?
        .into_iter()
        .filter_map(|(_, finding)| {
            matches!(finding.detail, FindingDetail::Effect(_)).then_some(finding)
        })
        .collect();
    assert_eq!(effects.len(), 2);
    assert_ne!(effects[0].id, effects[1].id);
    assert_eq!(effects[0].group, effects[1].group);
    Ok(())
}

#[test]
fn outbox_batch_discloses_more_work_instead_of_silently_truncating()
-> Result<(), Box<dyn std::error::Error>> {
    let mut store = Store::memory()?;
    for number in 0..129 {
        let notice = event(
            Source::Gmail,
            &format!("notice-{number}"),
            10,
            EventPayload::Notice {
                kind: NoticeKind::AppAccess,
                origin: Origin::Live,
            },
        );
        store.append_assessed(&notice, at(10))?;
    }
    let batch = store.pending_notifications()?;
    assert_eq!(batch.len(), 128);
    assert!(batch.has_more);
    assert!(store.mark_notification_delivered(&batch[0].id)?);
    let next = store.pending_notifications()?;
    assert_eq!(next.len(), 128);
    assert!(!next.has_more);
    Ok(())
}
