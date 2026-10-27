//! Incremental assessment must equal a full replay of the same log at the same time.

mod support;

use std::collections::BTreeMap;

use chrono::Duration;
use picket_core::{
    engine::{self, Finding},
    store::{Batch, Store},
};
use support::{Rng, at, random_history};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn stored(store: &Store) -> Result<BTreeMap<String, Finding>> {
    Ok(store
        .current_findings()?
        .into_iter()
        .map(|(_, finding)| (finding.id.clone(), finding))
        .collect())
}

fn replayed(
    store: &Store,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<BTreeMap<String, Finding>> {
    let events: Vec<_> = store.events()?.into_iter().map(|row| row.event).collect();
    Ok(engine::findings(&events, now)
        .into_iter()
        .map(|finding| (finding.id.clone(), finding))
        .collect())
}

#[test]
fn incremental_findings_equal_full_replay_after_every_commit_and_restart() -> Result<()> {
    let directory = tempfile::tempdir()?;
    for seed in 0..150 {
        let path = directory.path().join(format!("picket-{seed}.db"));
        let mut store = Store::open(&path)?;
        let mut rng = Rng::new(seed);
        let history = random_history(seed, 30);
        let mut now = at(0);
        let mut pending = history.as_slice();
        while !pending.is_empty() {
            let take = (1 + rng.below(3) as usize).min(pending.len());
            let (batch, rest) = pending.split_at(take);
            pending = rest;
            now += Duration::seconds(rng.below(600) as i64);
            store.commit(
                Batch {
                    events: batch.to_vec(),
                    ..Batch::default()
                },
                now,
            )?;
            if rng.below(5) == 0 {
                drop(store);
                store = Store::open(&path)?;
                store.reconcile(now)?;
            }
            assert_eq!(stored(&store)?, replayed(&store, now)?, "seed {seed}");
        }
        now += Duration::hours(2);
        store.reconcile(now)?;
        assert_eq!(
            stored(&store)?,
            replayed(&store, now)?,
            "seed {seed} after deadlines"
        );
    }
    Ok(())
}

#[test]
fn a_late_tool_result_updates_the_action_count_of_later_task_effects() -> Result<()> {
    use picket_core::{
        engine::{ConflictReason, FindingDetail, RequestFit},
        event::{
            Action, Effect, Event, EventKey, EventPayload, Field, Origin, Request, Source,
            ToolOutcome, ToolResult,
        },
    };
    let event = |source: Source, resource: &str, minute: i64, payload: EventPayload| Event {
        schema_version: 1,
        key: EventKey {
            account: "acct-1".into(),
            source,
            resource: resource.into(),
            revision: "1".into(),
        },
        occurred_at: at(minute),
        ingested_at: at(minute),
        source_time_quality: None,
        adapter: None,
        payload,
    };
    let send = |resource: &str, minute| {
        event(
            Source::Gmail,
            resource,
            minute,
            EventPayload::Effect(Effect {
                action: Action::EmailSend,
                targets: vec!["a@example.test".into()],
                targets_complete: true,
                calendar_start: None,
                origin: Origin::Live,
                external_change: false,
                email: None,
                calendar: None,
            }),
        )
    };
    let tool = |resource: &str, minute| {
        event(
            Source::OpenClaw,
            &format!("call-{resource}"),
            minute,
            EventPayload::ToolResult(ToolResult {
                agent: "agent-1".into(),
                task: Some("task-1".into()),
                action: Action::EmailSend,
                targets: vec!["a@example.test".into()],
                provider_resource: Some(resource.into()),
                provider_revision: None,
                outcome: ToolOutcome::Succeeded,
            }),
        )
    };
    let request = event(
        Source::Messages,
        "request-1",
        0,
        EventPayload::Request(Request {
            agent: "agent-1".into(),
            task: "task-1".into(),
            actions: Field::Known(vec![Action::EmailSend]),
            targets: Field::Known(vec!["a@example.test".into()]),
            calendar_start: Field::ExplicitAny,
            prohibited_actions: vec![],
            max_actions: Field::Known(1),
            valid_until: None,
            requires_approval: Field::Known(false),
            approved: false,
        }),
    );
    let mut store = Store::memory()?;
    for input in [request, send("m1", 10), send("m2", 20), tool("m2", 21)] {
        store.append_assessed(&input, at(22))?;
    }
    store.append_assessed(&tool("m1", 23), at(23))?;
    let second = stored(&store)?
        .into_values()
        .find_map(|finding| match finding.detail {
            FindingDetail::Effect(value) if value.effect.resource == "m2" => {
                Some(value.request_fit)
            }
            _ => None,
        });
    assert_eq!(
        second,
        Some(RequestFit::Conflicts(vec![ConflictReason::TooManyActions]))
    );
    assert_eq!(stored(&store)?, replayed(&store, at(23))?);
    Ok(())
}

fn coverage_report(state: picket_core::event::CoverageState) -> picket_core::store::CoverageReport {
    use picket_core::{
        event::{Coverage, CoverageReason, CoverageState, Source},
        store::CoverageReport,
    };
    CoverageReport {
        account: "acct-1".into(),
        source: Source::Gmail,
        coverage: Coverage {
            capability: "effects".into(),
            reason: (state == CoverageState::Delayed).then_some(CoverageReason::RequestFailed),
            state,
            cursor: None,
            gap_since: None,
            partial_pages: None,
        },
    }
}

#[test]
fn coverage_is_logged_on_transitions_and_when_an_open_claim_needs_confirmation() -> Result<()> {
    use picket_core::{
        engine::{ClaimStatus, FindingDetail},
        event::{Action, Claim, CoverageState, Event, EventKey, EventPayload, Source},
    };
    let mut store = Store::memory()?;
    let log_size = |store: &Store| -> Result<usize> { Ok(store.events()?.len()) };
    let poll = |store: &mut Store, state, minute| {
        store.commit(
            Batch {
                coverage: vec![coverage_report(state)],
                ..Batch::default()
            },
            at(minute),
        )
    };
    for minute in 0..30 {
        poll(&mut store, CoverageState::Healthy, minute)?;
    }
    assert_eq!(log_size(&store)?, 1, "idle healthy polls add no events");
    poll(&mut store, CoverageState::Delayed, 30)?;
    poll(&mut store, CoverageState::Delayed, 31)?;
    poll(&mut store, CoverageState::Healthy, 32)?;
    assert_eq!(log_size(&store)?, 3, "only transitions are logged");

    let claim = Event {
        schema_version: 1,
        key: EventKey {
            account: "acct-1".into(),
            source: Source::Messages,
            resource: "claim-1".into(),
            revision: "1".into(),
        },
        occurred_at: at(40),
        ingested_at: at(40),
        source_time_quality: None,
        adapter: None,
        payload: EventPayload::Claim(Claim {
            agent: "agent-1".into(),
            task: None,
            action: Action::EmailSend,
            targets: vec!["a@example.test".into()],
            provider_resource: None,
        }),
    };
    store.append_assessed(&claim, at(40))?;
    for minute in 41..60 {
        poll(&mut store, CoverageState::Healthy, minute)?;
    }
    assert_eq!(
        log_size(&store)?,
        5,
        "one confirmation after the claim deadline"
    );
    let status = stored(&store)?
        .into_values()
        .find_map(|finding| match finding.detail {
            FindingDetail::Claim(value) => Some(value.status),
            _ => None,
        });
    assert_eq!(status, Some(ClaimStatus::NotObservedInHealthySource));
    assert_eq!(stored(&store)?, replayed(&store, at(59))?);
    Ok(())
}

#[test]
fn one_new_input_assesses_a_bounded_set_regardless_of_history() -> Result<()> {
    use picket_core::event::{Action, Effect, Event, EventKey, EventPayload, Origin, Source};
    let send = |index: i64| Event {
        schema_version: 1,
        key: EventKey {
            account: "acct-1".into(),
            source: Source::Gmail,
            resource: format!("message-{index}"),
            revision: "1".into(),
        },
        occurred_at: at(index * 30),
        ingested_at: at(index * 30),
        source_time_quality: None,
        adapter: None,
        payload: EventPayload::Effect(Effect {
            action: Action::EmailSend,
            targets: vec![format!("person-{}@example.test", index % 7)],
            targets_complete: true,
            calendar_start: None,
            origin: Origin::Live,
            external_change: false,
            email: None,
            calendar: None,
        }),
    };
    let mut store = Store::memory()?;
    let history: Vec<_> = (0..5_000).map(send).collect();
    let end = at(5_000 * 30);
    store.commit(
        Batch {
            events: history,
            ..Batch::default()
        },
        end,
    )?;
    assert_eq!(
        store.reconcile(end)?.assessed,
        0,
        "an idle tick assesses nothing"
    );
    let summary = store.append_assessed(&send(5_000), end)?.1;
    assert!(summary.assessed <= 2, "assessed {}", summary.assessed);
    let later = end + Duration::hours(1);
    assert_eq!(
        store.reconcile(later)?.assessed,
        1,
        "only the passed deadline"
    );
    Ok(())
}

#[test]
fn retention_prunes_findings_about_removed_events_and_old_revisions() -> Result<()> {
    use picket_core::event::CoverageState;
    use picket_core::event::{Action, Effect, Event, EventKey, EventPayload, Origin, Source};
    let mut store = Store::memory()?;
    let send = |index: i64| Event {
        schema_version: 1,
        key: EventKey {
            account: "acct-1".into(),
            source: Source::Gmail,
            resource: format!("message-{index}"),
            revision: "1".into(),
        },
        occurred_at: at(index * 60),
        ingested_at: at(index * 60),
        source_time_quality: None,
        adapter: None,
        payload: EventPayload::Effect(Effect {
            action: Action::EmailSend,
            targets: vec!["a@example.test".into()],
            targets_complete: true,
            calendar_start: None,
            origin: Origin::Live,
            external_change: false,
            email: None,
            calendar: None,
        }),
    };
    for index in 0..4 {
        store.append_assessed(&send(index), at(index * 60))?;
        for (offset, state) in [(1, CoverageState::Delayed), (2, CoverageState::Healthy)] {
            store.commit(
                Batch {
                    coverage: vec![coverage_report(state)],
                    ..Batch::default()
                },
                at(index * 60 + offset),
            )?;
        }
    }
    store.reconcile(at(400))?;
    let effects = |store: &Store| -> Result<usize> {
        Ok(stored(store)?
            .values()
            .filter(|finding| finding.id.starts_with("effect/"))
            .count())
    };
    assert_eq!(effects(&store)?, 4);
    store.retain_older_than(at(150))?;
    assert_eq!(
        effects(&store)?,
        1,
        "findings about pruned effects are removed"
    );
    let coverage = stored(&store)?
        .into_keys()
        .find(|id| id.starts_with("coverage/"))
        .ok_or("missing coverage finding")?;
    let history = store.finding_history(&coverage)?;
    // Eight transitions; the five assessed before the minute-122 cutoff are superseded.
    assert_eq!(history.len(), 3);
    store.reconcile(at(400))?;
    assert_eq!(
        effects(&store)?,
        1,
        "retention does not recreate or revise findings"
    );
    Ok(())
}

#[test]
fn later_request_capture_settles_no_observed_request_for_earlier_effects() -> Result<()> {
    use picket_core::{
        engine::{FindingDetail, RequestFit, request_capability},
        event::{
            Action, Coverage, CoverageState, Effect, Event, EventKey, EventPayload, Origin, Source,
            ToolOutcome, ToolResult,
        },
    };
    let event = |source: Source, resource: &str, minute: i64, payload: EventPayload| Event {
        schema_version: 1,
        key: EventKey {
            account: "acct-1".into(),
            source,
            resource: resource.into(),
            revision: "1".into(),
        },
        occurred_at: at(minute),
        ingested_at: at(minute),
        source_time_quality: None,
        adapter: None,
        payload,
    };
    let send = event(
        Source::Gmail,
        "m1",
        10,
        EventPayload::Effect(Effect {
            action: Action::EmailSend,
            targets: vec!["a@example.test".into()],
            targets_complete: true,
            calendar_start: None,
            origin: Origin::Live,
            external_change: false,
            email: None,
            calendar: None,
        }),
    );
    let tool = event(
        Source::OpenClaw,
        "call-1",
        10,
        EventPayload::ToolResult(ToolResult {
            agent: "agent-1".into(),
            task: None,
            action: Action::EmailSend,
            targets: vec!["a@example.test".into()],
            provider_resource: Some("m1".into()),
            provider_revision: None,
            outcome: ToolOutcome::Succeeded,
        }),
    );
    // Before the effect's review deadline (minute 15), so only the dependency rule
    // can reassess it.
    let captured = event(
        Source::Messages,
        "capture-1",
        13,
        EventPayload::Coverage(Coverage {
            capability: request_capability("agent-1"),
            state: CoverageState::Healthy,
            cursor: None,
            gap_since: None,
            reason: None,
            partial_pages: None,
        }),
    );
    let fit = |store: &Store| -> Result<Option<RequestFit>> {
        Ok(stored(store)?
            .into_values()
            .find_map(|finding| match finding.detail {
                FindingDetail::Effect(value) => Some(value.request_fit),
                _ => None,
            }))
    };
    let mut store = Store::memory()?;
    store.append_assessed(&send, at(11))?;
    store.append_assessed(&tool, at(11))?;
    assert_eq!(fit(&store)?, Some(RequestFit::RequestNotCaptured));
    store.append_assessed(&captured, at(13))?;
    assert_eq!(fit(&store)?, Some(RequestFit::NoObservedRequest));
    assert_eq!(stored(&store)?, replayed(&store, at(13))?);
    Ok(())
}
