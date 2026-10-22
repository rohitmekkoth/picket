//! Hand-labeled M3 smoke scenarios. Truth is kept outside visible engine input.

use chrono::{DateTime, TimeZone, Utc};
use picket_core::{
    engine::{ActorEvidence, ConflictReason, FindingDetail, RequestFit, Route},
    event::{
        AccountPolicy, Action, Claim, Effect, Event, EventKey, EventPayload, Field, Origin,
        Request, Source, ToolOutcome, ToolResult,
    },
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

/// Deterministic extraction stand-in: synthetic prompt labels map to typed fields.
/// Prompt text never enters an Event or the SQLite store.
struct MockModel;

impl MockModel {
    fn request(&self, prompt: &str) -> Request {
        let mut value = Request {
            agent: "agent-1".into(),
            task: "task-1".into(),
            actions: Field::Known(vec![Action::EmailSend]),
            targets: Field::Known(vec!["a@example.test".into()]),
            calendar_start: Field::Unknown,
            prohibited_actions: vec![],
            max_actions: Field::ExplicitAny,
            valid_until: None,
            requires_approval: Field::Known(false),
            approved: false,
        };
        match prompt {
            "send the email to A" => {}
            "draft only; do not send" => {
                value.actions = Field::Known(vec![]);
                value.prohibited_actions.push(Action::EmailSend);
            }
            "ask before sending" => value.requires_approval = Field::Known(true),
            "recipient unclear" => value.targets = Field::Ambiguous,
            "meet at 100" => {
                value.actions = Field::Known(vec![Action::CalendarCreate]);
                value.calendar_start = Field::Known(at(100));
            }
            "send before 5" => value.valid_until = Some(at(5)),
            "one email only" => value.max_actions = Field::Known(1),
            _ => panic!("unknown synthetic prompt"),
        }
        value
    }

    fn claim(&self, prompt: &str, agent: &str, resource: &str) -> Claim {
        match prompt {
            "I sent the email" => Claim {
                agent: agent.into(),
                task: Some("task-1".into()),
                action: Action::EmailSend,
                targets: vec!["a@example.test".into()],
                provider_resource: Some(resource.into()),
            },
            _ => panic!("unknown synthetic claim"),
        }
    }
}

fn asked(prompt: &str) -> Event {
    event(
        Source::Messages,
        "request-1",
        0,
        EventPayload::Request(MockModel.request(prompt)),
    )
}

fn sent(resource: &str, minute: i64) -> Event {
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
}

fn runtime(resource: &str, agent: &str, task: Option<&str>) -> Event {
    event(
        Source::OpenClaw,
        &format!("call-{resource}-{agent}"),
        10,
        EventPayload::ToolResult(ToolResult {
            agent: agent.into(),
            task: task.map(str::to_owned),
            action: Action::EmailSend,
            targets: vec!["a@example.test".into()],
            provider_resource: Some(resource.into()),
            provider_revision: None,
            outcome: ToolOutcome::Succeeded,
        }),
    )
}

fn report(resource: &str, agent: &str) -> Event {
    event(
        Source::Messages,
        &format!("claim-{agent}"),
        11,
        EventPayload::Claim(MockModel.claim("I sent the email", agent, resource)),
    )
}

fn policy(enabled: bool) -> Event {
    event(
        Source::Picket,
        "policy-1",
        0,
        EventPayload::Policy(AccountPolicy {
            id: "no-send".into(),
            forbidden_actions: vec![Action::EmailSend],
            forbidden_targets: vec![],
            high_consequence: true,
            enabled,
        }),
    )
}

struct Scenario {
    label: &'static str,
    truth_actor: &'static str,
    visible: Vec<Event>,
    resource: &'static str,
    actor: ActorEvidence,
    fit: RequestFit,
    route: Route,
}

fn case(
    label: &'static str,
    truth_actor: &'static str,
    visible: Vec<Event>,
    resource: &'static str,
    actor: ActorEvidence,
    fit: RequestFit,
    route: Route,
) -> Scenario {
    Scenario {
        label,
        truth_actor,
        visible,
        resource,
        actor,
        fit,
        route,
    }
}

#[test]
fn twenty_four_labeled_scenarios_replay_through_store() -> Result<(), Box<dyn std::error::Error>> {
    use ActorEvidence::{Conflicting, Reported, RuntimeLinked, Unknown};
    use RequestFit::{
        AmbiguousTask, Conflicts, ConsistentWithVisibleRequest, InsufficientEvidence,
        NoObservedRequest, NotAssessed,
    };
    use Route::{HighReview, MediumReview, Timeline};
    let base_request = asked("send the email to A");
    let base_send = sent("message-1", 10);
    let base_tool = runtime("message-1", "agent-1", Some("task-1"));
    let mut cases = vec![
        case(
            "human phone while agent registered",
            "human",
            vec![base_request.clone(), base_send.clone()],
            "message-1",
            Unknown,
            NotAssessed,
            Timeline,
        ),
        case(
            "identical evidence but actual agent",
            "agent-1",
            vec![base_request.clone(), base_send.clone()],
            "message-1",
            Unknown,
            NotAssessed,
            Timeline,
        ),
        case(
            "linked send within visible request",
            "agent-1",
            vec![base_request.clone(), base_send.clone(), base_tool.clone()],
            "message-1",
            RuntimeLinked("agent-1".into()),
            ConsistentWithVisibleRequest,
            Timeline,
        ),
        case(
            "reported send only",
            "agent-1",
            vec![
                base_request.clone(),
                base_send.clone(),
                report("message-1", "agent-1"),
            ],
            "message-1",
            Reported("agent-1".into()),
            ConsistentWithVisibleRequest,
            Timeline,
        ),
        case(
            "unknown actor breaches account policy",
            "human",
            vec![policy(true), base_send.clone()],
            "message-1",
            Unknown,
            NotAssessed,
            HighReview,
        ),
        case(
            "disabled policy has no effect",
            "human",
            vec![policy(false), base_send.clone()],
            "message-1",
            Unknown,
            NotAssessed,
            Timeline,
        ),
    ];
    assert_eq!(cases[0].visible, cases[1].visible);
    let mut wrong = base_send.clone();
    if let EventPayload::Effect(value) = &mut wrong.payload {
        value.targets = vec!["b@example.test".into()];
    }
    let mut wrong_tool = base_tool.clone();
    if let EventPayload::ToolResult(value) = &mut wrong_tool.payload {
        value.targets = vec!["b@example.test".into()];
    }
    cases.push(case(
        "wrong recipient",
        "agent-1",
        vec![base_request.clone(), wrong, wrong_tool],
        "message-1",
        RuntimeLinked("agent-1".into()),
        Conflicts(vec![ConflictReason::ExtraTarget]),
        MediumReview,
    ));
    let mut extra = base_send.clone();
    if let EventPayload::Effect(value) = &mut extra.payload {
        value.targets.push("cc@example.test".into());
    }
    let mut extra_tool = base_tool.clone();
    if let EventPayload::ToolResult(value) = &mut extra_tool.payload {
        value.targets.push("cc@example.test".into());
    }
    cases.push(case(
        "extra CC recipient",
        "agent-1",
        vec![base_request.clone(), extra, extra_tool],
        "message-1",
        RuntimeLinked("agent-1".into()),
        Conflicts(vec![ConflictReason::ExtraTarget]),
        MediumReview,
    ));
    cases.push(case(
        "draft only but agent sent",
        "agent-1",
        vec![
            asked("draft only; do not send"),
            base_send.clone(),
            base_tool.clone(),
        ],
        "message-1",
        RuntimeLinked("agent-1".into()),
        Conflicts(vec![
            ConflictReason::ProhibitedAction,
            ConflictReason::DifferentAction,
        ]),
        MediumReview,
    ));
    cases.push(case(
        "approval was never captured",
        "agent-1",
        vec![
            asked("ask before sending"),
            base_send.clone(),
            base_tool.clone(),
        ],
        "message-1",
        RuntimeLinked("agent-1".into()),
        Conflicts(vec![ConflictReason::ApprovalMissing]),
        MediumReview,
    ));
    cases.push(case(
        "ambiguous recipient extraction",
        "agent-1",
        vec![
            asked("recipient unclear"),
            base_send.clone(),
            base_tool.clone(),
        ],
        "message-1",
        RuntimeLinked("agent-1".into()),
        InsufficientEvidence,
        Timeline,
    ));
    let mut failed = base_tool.clone();
    if let EventPayload::ToolResult(value) = &mut failed.payload {
        value.outcome = ToolOutcome::Failed;
    }
    cases.push(case(
        "failed tool call",
        "human",
        vec![base_request.clone(), base_send.clone(), failed],
        "message-1",
        Unknown,
        NotAssessed,
        Timeline,
    ));
    let mut fabricated = base_tool.clone();
    if let EventPayload::ToolResult(value) = &mut fabricated.payload {
        value.provider_resource = Some("fabricated-id".into());
    }
    cases.push(case(
        "fabricated provider ID",
        "human",
        vec![base_request.clone(), base_send.clone(), fabricated],
        "message-1",
        Unknown,
        NotAssessed,
        Timeline,
    ));
    let mut second_agent = runtime("message-1", "agent-2", Some("task-2"));
    second_agent.key.resource = "call-other-agent".into();
    cases.push(case(
        "two runtime agents claim same effect",
        "unknown",
        vec![base_send.clone(), base_tool.clone(), second_agent],
        "message-1",
        Conflicting(vec!["agent-1".into(), "agent-2".into()]),
        NotAssessed,
        Timeline,
    ));
    let mut external = base_send.clone();
    external.key.source = Source::Calendar;
    if let EventPayload::Effect(value) = &mut external.payload {
        value.action = Action::CalendarUpdate;
        value.external_change = true;
    }
    cases.push(case(
        "external calendar organizer update",
        "external",
        vec![external, base_tool.clone()],
        "message-1",
        Unknown,
        NotAssessed,
        Timeline,
    ));
    let mut calendar = base_send.clone();
    calendar.key.source = Source::Calendar;
    if let EventPayload::Effect(value) = &mut calendar.payload {
        value.action = Action::CalendarCreate;
        value.calendar_start = Some(at(101));
    }
    let mut calendar_tool = base_tool.clone();
    if let EventPayload::ToolResult(value) = &mut calendar_tool.payload {
        value.action = Action::CalendarCreate;
    }
    cases.push(case(
        "wrong calendar start time",
        "agent-1",
        vec![asked("meet at 100"), calendar, calendar_tool],
        "message-1",
        RuntimeLinked("agent-1".into()),
        Conflicts(vec![ConflictReason::DifferentCalendarTime]),
        MediumReview,
    ));
    cases.push(case(
        "action after explicit deadline",
        "agent-1",
        vec![asked("send before 5"), base_send.clone(), base_tool.clone()],
        "message-1",
        RuntimeLinked("agent-1".into()),
        Conflicts(vec![ConflictReason::OutsideWindow]),
        MediumReview,
    ));
    let mut later_request = base_request.clone();
    later_request.key.resource = "request-later".into();
    later_request.occurred_at = at(11);
    later_request.ingested_at = at(11);
    cases.push(case(
        "late request cannot authorize earlier send",
        "agent-1",
        vec![later_request, base_send.clone(), base_tool.clone()],
        "message-1",
        RuntimeLinked("agent-1".into()),
        NoObservedRequest,
        Timeline,
    ));
    let mut second_task = base_request.clone();
    second_task.key.resource = "request-2".into();
    if let EventPayload::Request(value) = &mut second_task.payload {
        value.task = "task-2".into();
    }
    let unhinted = runtime("message-1", "agent-1", None);
    cases.push(case(
        "overlapping tasks without task identity",
        "agent-1",
        vec![
            base_request.clone(),
            second_task,
            base_send.clone(),
            unhinted,
        ],
        "message-1",
        RuntimeLinked("agent-1".into()),
        AmbiguousTask,
        Timeline,
    ));
    let mut incomplete = base_send.clone();
    if let EventPayload::Effect(value) = &mut incomplete.payload {
        value.targets_complete = false;
    }
    cases.push(case(
        "missing recipient visibility",
        "unknown",
        vec![base_request.clone(), incomplete, base_tool.clone()],
        "message-1",
        Unknown,
        NotAssessed,
        Timeline,
    ));
    let mut no_resource = base_tool.clone();
    if let EventPayload::ToolResult(value) = &mut no_resource.payload {
        value.provider_resource = None;
    }
    cases.push(case(
        "successful tool with no provider ID",
        "unknown",
        vec![base_request.clone(), base_send.clone(), no_resource],
        "message-1",
        Unknown,
        NotAssessed,
        Timeline,
    ));
    let mut body_claim = report("message-1", "agent-1");
    if let EventPayload::Claim(value) = &mut body_claim.payload {
        value.provider_resource = None;
    }
    let other_send = sent("message-2", 11);
    cases.push(case(
        "ambiguous body-only claim near human send",
        "human",
        vec![
            base_request.clone(),
            base_send.clone(),
            other_send,
            body_claim,
        ],
        "message-1",
        Unknown,
        NotAssessed,
        Timeline,
    ));
    let mut baseline = base_send.clone();
    if let EventPayload::Effect(value) = &mut baseline.payload {
        value.origin = Origin::Baseline;
    }
    cases.push(case(
        "baseline import is not live activity",
        "human",
        vec![baseline, sent("message-2", 12)],
        "message-2",
        Unknown,
        NotAssessed,
        Timeline,
    ));
    let mut count_request = asked("one email only");
    let mut second_send = sent("message-2", 12);
    second_send.ingested_at = at(12);
    let mut second_tool = runtime("message-2", "agent-1", Some("task-1"));
    second_tool.ingested_at = at(12);
    count_request.key.resource = "request-count".into();
    cases.push(case(
        "second task-linked send exceeds action slot",
        "agent-1",
        vec![
            count_request,
            base_send.clone(),
            base_tool.clone(),
            second_send,
            second_tool,
        ],
        "message-2",
        RuntimeLinked("agent-1".into()),
        Conflicts(vec![ConflictReason::TooManyActions]),
        MediumReview,
    ));
    assert_eq!(cases.len(), 24);

    for scenario in cases {
        let mut store = Store::memory()?;
        for visible in &scenario.visible {
            store.append_assessed(visible, at(30))?;
        }
        store.reconcile(at(30))?;
        let actual = store
            .current_findings()?
            .into_iter()
            .find_map(|(_, finding)| {
                if let FindingDetail::Effect(value) = finding.detail {
                    (value.effect.resource == scenario.resource).then_some(value)
                } else {
                    None
                }
            })
            .ok_or_else(|| format!("{}: effect missing", scenario.label))?;
        assert_eq!(
            actual.actor, scenario.actor,
            "{} actor; truth={}",
            scenario.label, scenario.truth_actor
        );
        assert_eq!(actual.request_fit, scenario.fit, "{} fit", scenario.label);
        assert_eq!(actual.route, scenario.route, "{} route", scenario.label);
        assert!(store.verify()?.retained_events > 0);
    }
    Ok(())
}
