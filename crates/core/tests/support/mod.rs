//! Deterministic synthetic event mixes for equivalence tests. No real account data.

use chrono::{DateTime, Duration, TimeZone, Utc};
use picket_core::event::{
    AccountPolicy, Action, Claim, Coverage, CoverageState, Effect, Event, EventKey, EventPayload,
    Feedback, Field, NoticeKind, Origin, Request, Source, ToolOutcome, ToolResult,
};

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(
            seed.wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407),
        )
    }

    pub fn below(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % bound
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len() as u64) as usize]
    }
}

pub fn at(minute: i64) -> DateTime<Utc> {
    Utc.timestamp_nanos((1_700_000_000 + minute * 60) * 1_000_000_000)
}

const RESOURCES: &[&str] = &["m1", "m2", "m3", "e1", "e2"];
const TARGETS: &[&str] = &["a@example.test", "b@example.test"];
const AGENTS: &[&str] = &["agent-1", "agent-2"];
const TASKS: &[&str] = &["task-1", "task-2"];
const ACCOUNTS: &[&str] = &["acct-1", "acct-2"];

fn action(rng: &mut Rng) -> Action {
    [
        Action::EmailSend,
        Action::CalendarCreate,
        Action::CalendarUpdate,
    ][rng.below(3) as usize]
        .clone()
}

/// One synthetic event near `minute`, ingested between zero and twenty minutes later.
pub fn random_event(rng: &mut Rng, index: usize, minute: i64) -> Event {
    let account = rng.pick(ACCOUNTS).to_owned();
    let resource = rng.pick(RESOURCES).to_owned();
    let targets = vec![rng.pick(TARGETS).to_owned()];
    let agent = rng.pick(AGENTS).to_owned();
    let task = rng.pick(TASKS).to_owned();
    let occurred = at(minute) + Duration::seconds(rng.below(1_800) as i64 - 900);
    let ingested = occurred + Duration::seconds(rng.below(1_200) as i64);
    let (source, payload) = match rng.below(11) {
        0..=2 => {
            let action = action(rng);
            let source = if action == Action::EmailSend {
                Source::Gmail
            } else {
                Source::Calendar
            };
            (
                source,
                EventPayload::Effect(Effect {
                    action,
                    targets,
                    targets_complete: rng.below(5) != 0,
                    calendar_start: None,
                    origin: [Origin::Live, Origin::CatchUp, Origin::Baseline]
                        [rng.below(3) as usize]
                        .clone(),
                    external_change: rng.below(6) == 0,
                    email: None,
                    calendar: None,
                }),
            )
        }
        3 => (
            Source::Messages,
            EventPayload::Claim(Claim {
                agent,
                task: (rng.below(2) == 0).then_some(task),
                action: action(rng),
                targets,
                provider_resource: (rng.below(2) == 0).then_some(resource.clone()),
            }),
        ),
        4 => (
            Source::OpenClaw,
            EventPayload::ToolResult(ToolResult {
                agent,
                task: (rng.below(4) != 0).then_some(task),
                action: action(rng),
                targets,
                provider_resource: (rng.below(5) != 0).then_some(resource.clone()),
                provider_revision: (rng.below(2) == 0).then(|| format!("r{}", rng.below(3))),
                outcome: [
                    ToolOutcome::Succeeded,
                    ToolOutcome::Failed,
                    ToolOutcome::Unknown,
                ][rng.below(3) as usize]
                    .clone(),
            }),
        ),
        5 => (
            Source::Messages,
            EventPayload::Request(Request {
                agent,
                task,
                actions: if rng.below(2) == 0 {
                    Field::Known(vec![action(rng)])
                } else {
                    Field::ExplicitAny
                },
                targets: Field::Known(targets),
                calendar_start: Field::ExplicitAny,
                prohibited_actions: vec![],
                max_actions: [Field::ExplicitAny, Field::Known(1), Field::Known(2)]
                    [rng.below(3) as usize]
                    .clone(),
                valid_until: (rng.below(3) == 0).then(|| occurred + Duration::minutes(20)),
                requires_approval: Field::Known(false),
                approved: false,
            }),
        ),
        6 => (
            Source::Picket,
            EventPayload::Policy(AccountPolicy {
                id: format!("policy-{}", rng.below(2)),
                forbidden_actions: vec![action(rng)],
                forbidden_targets: vec![],
                high_consequence: rng.below(2) == 0,
                enabled: rng.below(4) != 0,
            }),
        ),
        7 => (
            Source::Picket,
            EventPayload::Feedback(Feedback {
                finding: format!("unknown-{}", rng.below(2)),
                expected: Some(rng.below(2) == 0),
                actor: (rng.below(2) == 0).then_some(agent),
            }),
        ),
        8 | 9 => (
            [Source::Gmail, Source::Calendar][rng.below(2) as usize].clone(),
            EventPayload::Coverage(Coverage {
                capability: "effects".into(),
                state: [CoverageState::Healthy, CoverageState::Gap][rng.below(2) as usize].clone(),
                cursor: None,
                gap_since: None,
                reason: None,
                partial_pages: None,
            }),
        ),
        _ => (
            Source::Gmail,
            EventPayload::Notice {
                kind: [NoticeKind::Forwarding, NoticeKind::Otp][rng.below(2) as usize].clone(),
                origin: Origin::Live,
            },
        ),
    };
    let revision = if matches!(payload, EventPayload::Effect(_)) {
        format!("r{}", rng.below(3))
    } else {
        index.to_string()
    };
    let resource = if matches!(payload, EventPayload::Effect(_)) {
        resource
    } else {
        format!("input-{index}")
    };
    Event {
        schema_version: 1,
        key: EventKey {
            account,
            source,
            resource,
            revision,
        },
        occurred_at: occurred,
        ingested_at: ingested,
        source_time_quality: None,
        adapter: None,
        payload,
    }
}

/// A random history in which feedback names findings that actually exist.
pub fn random_history(seed: u64, count: usize) -> Vec<Event> {
    let mut rng = Rng::new(seed);
    let mut events: Vec<Event> = Vec::new();
    for index in 0..count {
        let mut event = random_event(&mut rng, index, (index as i64) * 3);
        let target = events
            .iter()
            .filter(|item| matches!(item.payload, EventPayload::Effect(_)))
            .nth(rng.below(4) as usize)
            .cloned();
        // Align some inputs with an earlier effect so links, counts and feedback occur.
        if let Some(target) = target.filter(|_| rng.below(3) != 0)
            && let EventPayload::Effect(effect) = &target.payload
        {
            match &mut event.payload {
                EventPayload::Feedback(feedback) => {
                    feedback.finding = picket_core::engine::effect_finding_id(&target.key);
                }
                EventPayload::ToolResult(tool) => {
                    tool.action = effect.action.clone();
                    tool.targets = effect.targets.clone();
                    tool.provider_resource = Some(target.key.resource.clone());
                    tool.provider_revision = Some(target.key.revision.clone());
                }
                EventPayload::Claim(claim) => {
                    claim.action = effect.action.clone();
                    claim.targets = effect.targets.clone();
                }
                EventPayload::Request(request) => {
                    request.targets = Field::Known(effect.targets.clone());
                    event.occurred_at = target.occurred_at - Duration::minutes(5);
                }
                _ => {}
            }
            event.key.account = target.key.account.clone();
        }
        let duplicate = events
            .iter()
            .any(|item| item.key == event.key && item.payload.kind() == event.payload.kind());
        if !duplicate {
            events.push(event);
        }
    }
    events
}
