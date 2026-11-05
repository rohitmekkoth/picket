//! Rule levels and boundaries through the store and outbox. Synthetic data only.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, TimeZone, Utc};
use picket_core::{
    engine::{ClaimStatus, Finding, FindingDetail, Route},
    event::{
        Action, Boundary, BoundaryKind, Claim, ClaimChannel, ContactObserved, Coverage,
        CoverageState, Effect, Event, EventKey, EventPayload, Level, NoticeKind, Origin, RuleSet,
        Source,
    },
    rules,
    store::Store,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn at(minute: i64) -> DateTime<Utc> {
    Utc.timestamp_nanos((1_790_000_000 + minute * 60) * 1_000_000_000)
}

fn event(source: Source, resource: &str, minute: i64, payload: EventPayload) -> Event {
    Event {
        schema_version: 1,
        key: EventKey {
            account: "acct".into(),
            source,
            resource: resource.into(),
            revision: minute.to_string(),
        },
        occurred_at: at(minute),
        ingested_at: at(minute),
        source_time_quality: None,
        adapter: None,
        payload,
    }
}

fn email(resource: &str, minute: i64, to: &str) -> Event {
    event(
        Source::Gmail,
        resource,
        minute,
        EventPayload::Effect(Effect {
            action: Action::EmailSend,
            targets: vec![to.into()],
            targets_complete: true,
            calendar_start: None,
            origin: Origin::Live,
            external_change: false,
            target_names: BTreeMap::new(),
            email: None,
            calendar: None,
        }),
    )
}

fn deletion(resource: &str, minute: i64, external: bool) -> Event {
    let mut value = email(resource, minute, "x@example.test");
    value.key.source = Source::Calendar;
    if let EventPayload::Effect(effect) = &mut value.payload {
        effect.action = Action::CalendarDelete;
        effect.external_change = external;
    }
    value
}

fn rules_event(minute: i64, rules: RuleSet) -> Event {
    event(Source::Picket, "rules", minute, EventPayload::Rules(rules))
}

fn boundary(id: &str, level: Level, since: i64, kind: BoundaryKind) -> Boundary {
    Boundary {
        id: id.into(),
        level,
        since: at(since),
        kind,
    }
}

fn with_boundaries(boundaries: Vec<Boundary>) -> RuleSet {
    RuleSet {
        levels: BTreeMap::new(),
        boundaries,
    }
}

fn effect_route(store: &Store, resource: &str) -> Result<(Route, Vec<String>)> {
    store
        .current_findings()?
        .into_iter()
        .find_map(|(_, finding)| match finding.detail {
            FindingDetail::Effect(value) if value.effect.resource == resource => {
                Some((finding.route, value.boundaries))
            }
            _ => None,
        })
        .ok_or_else(|| format!("finding for {resource} missing").into())
}

fn finding_of(store: &Store, matches: impl Fn(&Finding) -> bool) -> Result<Finding> {
    store
        .current_findings()?
        .into_iter()
        .map(|(_, finding)| finding)
        .find(|finding| matches(finding))
        .ok_or_else(|| "finding missing".into())
}

#[test]
fn contact_and_calendar_boundaries_apply_from_their_start_and_alert_immediately() -> Result<()> {
    let mut store = Store::memory()?;
    store.append_assessed(&email("before", 1, "help@bank.example"), at(1))?;
    store.append_assessed(
        &rules_event(
            2,
            with_boundaries(vec![
                boundary(
                    "bank",
                    Level::Urgent,
                    2,
                    BoundaryKind::Contact {
                        addresses: vec![],
                        domains: vec!["bank.example".into()],
                        actions: vec![Action::EmailSend],
                    },
                ),
                boundary(
                    "deletes",
                    Level::Review,
                    2,
                    BoundaryKind::CalendarChange {
                        actions: vec![Action::CalendarDelete],
                    },
                ),
            ]),
        ),
        at(2),
    )?;
    let (_, alert) = store.append_assessed(&email("after", 3, "help@bank.example"), at(3))?;
    assert_eq!(alert.queued, 1, "a boundary does not wait for follow-up");
    assert_eq!(
        effect_route(&store, "after")?,
        (Route::HighReview, vec!["bank".to_string()])
    );
    assert_eq!(
        effect_route(&store, "before")?,
        (Route::Timeline, Vec::new()),
        "a new boundary never flags the past"
    );
    store.append_assessed(&deletion("mine", 4, false), at(4))?;
    store.append_assessed(&deletion("organizer", 5, true), at(5))?;
    assert_eq!(effect_route(&store, "mine")?.0, Route::MediumReview);
    assert_eq!(
        effect_route(&store, "organizer")?.0,
        Route::Timeline,
        "an organizer's change is not the user crossing a boundary"
    );
    Ok(())
}

#[test]
fn bursts_count_matching_effects_in_their_window() -> Result<()> {
    let mut store = Store::memory()?;
    store.append_assessed(
        &rules_event(
            0,
            with_boundaries(vec![boundary(
                "burst",
                Level::Review,
                0,
                BoundaryKind::Burst {
                    count: 2,
                    window_minutes: 10,
                    actions: vec![Action::EmailSend],
                },
            )]),
        ),
        at(0),
    )?;
    for (index, minute) in [1, 2, 3, 30].into_iter().enumerate() {
        store.append_assessed(
            &email(&format!("m{index}"), minute, "a@example.test"),
            at(minute),
        )?;
    }
    assert_eq!(effect_route(&store, "m1")?.0, Route::Timeline);
    assert_eq!(
        effect_route(&store, "m2")?.0,
        Route::MediumReview,
        "third in ten minutes"
    );
    assert_eq!(
        effect_route(&store, "m3")?.0,
        Route::Timeline,
        "outside the window"
    );
    Ok(())
}

#[test]
fn new_recipients_are_people_never_seen_in_mail_before() -> Result<()> {
    let mut store = Store::memory()?;
    let mut baseline = email("old", 0, "friend@example.test");
    if let EventPayload::Effect(effect) = &mut baseline.payload {
        effect.origin = Origin::Baseline;
    }
    store.append_assessed(&baseline, at(0))?;
    store.append_assessed(
        &event(
            Source::Gmail,
            "inbox",
            0,
            EventPayload::ContactObserved(ContactObserved {
                address: "writer@example.test".into(),
                origin: Origin::Baseline,
            }),
        ),
        at(0),
    )?;
    store.append_assessed(
        &rules_event(
            1,
            with_boundaries(vec![boundary(
                "new",
                Level::Review,
                1,
                BoundaryKind::NewRecipient,
            )]),
        ),
        at(1),
    )?;
    store.append_assessed(&email("to-friend", 2, "friend@example.test"), at(2))?;
    store.append_assessed(&email("to-writer", 3, "writer@example.test"), at(3))?;
    store.append_assessed(&email("to-stranger", 4, "stranger@example.test"), at(4))?;
    store.append_assessed(&email("again", 5, "stranger@example.test"), at(5))?;
    assert_eq!(effect_route(&store, "to-friend")?.0, Route::Timeline);
    assert_eq!(effect_route(&store, "to-writer")?.0, Route::Timeline);
    assert_eq!(effect_route(&store, "to-stranger")?.0, Route::MediumReview);
    assert_eq!(
        effect_route(&store, "again")?.0,
        Route::Timeline,
        "the second email is no longer to someone new"
    );
    // A contact learned late, dated before the email, revises it.
    store.append_assessed(
        &event(
            Source::Gmail,
            "late",
            1,
            EventPayload::ContactObserved(ContactObserved {
                address: "stranger@example.test".into(),
                origin: Origin::CatchUp,
            }),
        ),
        at(6),
    )?;
    assert_eq!(effect_route(&store, "to-stranger")?.0, Route::Timeline);
    Ok(())
}

#[test]
fn levels_route_reports_notices_and_gaps_and_off_records_only() -> Result<()> {
    let mut store = Store::memory()?;
    let healthy = event(
        Source::Gmail,
        "coverage",
        0,
        EventPayload::Coverage(Coverage {
            capability: "effects".into(),
            state: CoverageState::Healthy,
            cursor: None,
            gap_since: None,
            reason: None,
            partial_pages: None,
        }),
    );
    store.append_assessed(&healthy, at(0))?;
    let report = event(
        Source::Messages,
        "m1#claim0",
        1,
        EventPayload::Claim(Claim {
            agent: "instinct".into(),
            task: None,
            action: Action::EmailSend,
            targets: vec!["sam@example.test".into()],
            provider_resource: None,
            target_names: Vec::new(),
            channel: ClaimChannel::UserAccount,
        }),
    );
    store.append_assessed(&report, at(1))?;
    let mut later = healthy.clone();
    later.key.revision = "60".into();
    later.occurred_at = at(60);
    later.ingested_at = at(60);
    store.append_assessed(&later, at(60))?;
    store.reconcile(at(60))?;
    let claim = finding_of(&store, |finding| {
        matches!(&finding.detail, FindingDetail::Claim(check)
            if check.status == ClaimStatus::NotObservedInHealthySource)
    })?;
    assert_eq!(
        claim.route,
        Route::MediumReview,
        "an unseen reported send needs review"
    );
    let purchase = event(
        Source::Gmail,
        "receipt",
        61,
        EventPayload::Notice {
            kind: NoticeKind::Purchase,
            origin: Origin::Live,
        },
    );
    let (_, queued) = store.append_assessed(&purchase, at(61))?;
    assert_eq!(queued.queued, 0, "purchases go to the summary");
    let notice = finding_of(&store, |finding| {
        matches!(finding.detail, FindingDetail::Notice(_))
    })?;
    assert_eq!(notice.route, Route::Summary);

    let mut quiet = RuleSet::default();
    quiet
        .levels
        .insert(rules::REPORT_NOT_OBSERVED.into(), Level::Off);
    quiet.levels.insert(rules::PURCHASE.into(), Level::Urgent);
    let (_, changed) = store.append_assessed(&rules_event(62, quiet), at(62))?;
    assert_eq!(
        changed.queued, 0,
        "changing a rule never notifies about the past"
    );
    assert_eq!(
        finding_of(&store, |finding| finding.id == claim.id)?.route,
        Route::Timeline
    );
    assert_eq!(
        finding_of(&store, |finding| finding.id == notice.id)?.route,
        Route::HighReview
    );
    let mut second = purchase.clone();
    second.key.resource = "receipt-2".into();
    second.occurred_at = at(63);
    second.ingested_at = at(63);
    assert_eq!(
        store.append_assessed(&second, at(63))?.1.queued,
        1,
        "a new purchase now alerts right away"
    );
    Ok(())
}

#[test]
fn an_engine_upgrade_reassesses_history_without_a_burst_of_alerts() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("upgrade.db");
    let mut store = Store::open(&path)?;
    let notice = event(
        Source::Gmail,
        "forward",
        0,
        EventPayload::Notice {
            kind: NoticeKind::Forwarding,
            origin: Origin::Live,
        },
    );
    store.append_assessed(&notice, at(0))?;
    let id = store
        .pending_notifications()?
        .first()
        .map(|item| item.id.clone())
        .ok_or("alert missing")?;
    store.mark_notification_delivered(&id, chrono::Utc::now(), false)?;
    drop(store);
    // An older engine assessed this database; its findings are revised on open.
    let connection = rusqlite::Connection::open(&path)?;
    connection.execute("UPDATE engine_state SET engine_version=2 WHERE id=1", [])?;
    connection.execute(
        "UPDATE assessment_revisions SET payload=CAST(replace(CAST(payload AS TEXT),
           'MediumReview', 'HighReview') AS BLOB)",
        [],
    )?;
    drop(connection);
    let mut reopened = Store::open(&path)?;
    let summary = reopened.reconcile(at(1) + Duration::minutes(1))?;
    assert!(summary.revised > 0, "the new engine reassesses");
    assert_eq!(summary.queued, 0);
    assert!(reopened.pending_notifications()?.is_empty());
    Ok(())
}

#[test]
fn rule_edits_are_versioned_validated_and_previewed() -> Result<()> {
    let mut store = Store::memory()?;
    store.append_assessed(&email("old", 1, "help@bank.example"), at(1))?;
    store.append_assessed(&email("other", 2, "sam@example.test"), at(2))?;
    let now = at(10);
    let kind = BoundaryKind::Contact {
        addresses: vec![],
        domains: vec![" @Bank.Example ".into()],
        actions: vec![Action::EmailSend, Action::EmailSend],
    };
    let preview = store.preview_boundary(kind.clone(), now)?;
    assert_eq!(preview.count, 1, "the preview looks at the past");
    assert_eq!(preview.hits[0].people[0].address, "help@bank.example");

    let id = store.save_boundary(None, Level::Urgent, kind, now)?;
    assert_eq!(id, "b-1");
    let view = store.rules(now)?;
    assert_eq!(view.boundaries.len(), 1);
    assert_eq!(view.boundaries[0].recent, 1);
    assert!(matches!(&view.boundaries[0].boundary.kind,
        BoundaryKind::Contact { domains, actions, .. }
            if domains == &["bank.example".to_string()] && actions.len() == 1));
    assert_eq!(
        effect_route(&store, "old")?.0,
        Route::Timeline,
        "saving does not flag what the preview showed"
    );

    // Changing a boundary keeps when it started.
    let later = at(20);
    store.save_boundary(
        Some(&id),
        Level::Review,
        BoundaryKind::CalendarChange {
            actions: vec![Action::CalendarDelete],
        },
        later,
    )?;
    let view = store.rules(later)?;
    assert_eq!(view.boundaries[0].boundary.since, now);
    assert_eq!(view.boundaries[0].boundary.level, Level::Review);
    assert!(
        store
            .save_boundary(
                Some("b-9"),
                Level::Review,
                BoundaryKind::NewRecipient,
                later
            )
            .is_err()
    );
    assert!(
        store
            .preview_boundary(
                BoundaryKind::Contact {
                    addresses: vec!["not an address".into()],
                    domains: vec![],
                    actions: vec![Action::EmailSend],
                },
                later,
            )
            .is_err()
    );

    store.set_rule_level(rules::OTP, Level::Review, later)?;
    assert!(
        store
            .set_rule_level("request.unknown", Level::Off, later)
            .is_err()
    );
    let otp = store
        .rules(later)?
        .levels
        .into_iter()
        .find(|item| item.id == rules::OTP)
        .ok_or("rule missing")?;
    assert_eq!((otp.level, otp.default), (Level::Review, Level::Off));
    assert!(store.delete_boundary(&id, later)?);
    assert!(!store.delete_boundary(&id, later)?);
    assert!(store.rules(later)?.boundaries.is_empty());
    Ok(())
}
