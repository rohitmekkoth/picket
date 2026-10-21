//! Deterministic, conservative assessments of independently observed effects.
//! A report from an agent is evidence of a report, not proof of authorship.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};

use crate::event::{
    AccountPolicy, Action, Effect, Event, EventKey, EventPayload, Feedback, Field, Origin, Request,
    SnapshotValue, Source, ToolOutcome,
};

const MAX_CANDIDATES: usize = 128;
const CLAIM_WINDOW: Duration = Duration::minutes(10);
const GRACE: Duration = Duration::minutes(5);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActorEvidence {
    Unknown,
    Reported(String),
    RuntimeLinked(String),
    UserConfirmed(String),
    Conflicting(Vec<String>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RequestFit {
    NotAssessed,
    NoObservedRequest,
    AmbiguousTask,
    InsufficientEvidence,
    ConsistentWithVisibleRequest,
    Conflicts(Vec<ConflictReason>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictReason {
    ProhibitedAction,
    DifferentAction,
    ExtraTarget,
    ApprovalMissing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Route {
    Timeline,
    MediumReview,
    HighReview,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Assessment {
    pub finding: String,
    pub effect: EventKey,
    pub actor: ActorEvidence,
    pub request_fit: RequestFit,
    pub policy_ids: Vec<String>,
    pub pending: bool,
    pub route: Route,
    pub expected_feedback: Option<bool>,
}

/// Replaying the same retained input at the same clock time gives the same result.
/// Retention may remove earlier requests or policy history, so callers must show its horizon.
pub fn assess(events: &[Event], now: DateTime<Utc>) -> Vec<Assessment> {
    let mut effects: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::Effect(effect) if effect.origin != Origin::Baseline => {
                Some((event, effect))
            }
            _ => None,
        })
        .collect();
    effects.sort_by(|(a, _), (b, _)| {
        a.occurred_at
            .cmp(&b.occurred_at)
            .then_with(|| a.key.resource.cmp(&b.key.resource))
    });
    effects
        .into_iter()
        .map(|(event, effect)| assess_one(events, event, effect, now))
        .collect()
}

fn assess_one(events: &[Event], event: &Event, effect: &Effect, now: DateTime<Utc>) -> Assessment {
    let finding = finding_id(&event.key);
    let actor = actor_for(events, event, effect, &finding);
    let policy_hits = policy_violations(events, event, effect);
    let policy_ids = policy_hits.iter().map(|(id, _)| id.clone()).collect();
    let request_fit = request_fit(events, event, effect, &actor);
    let wait_from = if event.occurred_at > event.ingested_at + GRACE {
        event.ingested_at // Source clock is suspiciously ahead; do not wait indefinitely.
    } else {
        event.ingested_at.max(event.occurred_at)
    };
    let pending = now < wait_from + GRACE;
    let expected_feedback = events
        .iter()
        .filter_map(|candidate| {
            if candidate.key.account == event.key.account
                && let Some((feedback, occurred_at)) = feedback(candidate)
                && occurred_at >= event.occurred_at
                && feedback.finding == finding
            {
                return Some((candidate.ingested_at, feedback.expected));
            }
            None
        })
        .max_by_key(|(at, _)| *at)
        .and_then(|(_, value)| value);
    let route = if expected_feedback == Some(true) {
        Route::Timeline
    } else if policy_hits.iter().any(|(_, high)| *high) {
        Route::HighReview
    } else if !policy_hits.is_empty()
        || (!pending
            && matches!(request_fit, RequestFit::Conflicts(_))
            && matches!(
                actor,
                ActorEvidence::Reported(_)
                    | ActorEvidence::RuntimeLinked(_)
                    | ActorEvidence::UserConfirmed(_)
            ))
    {
        Route::MediumReview
    } else {
        Route::Timeline
    };
    Assessment {
        finding,
        effect: event.key.clone(),
        actor,
        request_fit,
        policy_ids,
        pending,
        route,
        expected_feedback,
    }
}

fn actor_for(events: &[Event], event: &Event, effect: &Effect, finding: &str) -> ActorEvidence {
    if effect.external_change {
        return ActorEvidence::Unknown;
    }
    let mut runtime = BTreeSet::new();
    let mut reports = BTreeSet::new();
    let mut confirmed = None;
    for candidate in events
        .iter()
        .filter(|item| item.key.account == event.key.account)
    {
        match &candidate.payload {
            EventPayload::ToolResult(tool)
                if candidate.key.source == Source::OpenClaw
                    && tool.outcome == ToolOutcome::Succeeded
                    && tool.provider_resource.as_deref() == Some(event.key.resource.as_str())
                    && tool.action == effect.action
                    && effect.targets_complete
                    && same_targets(&tool.targets, &effect.targets)
                    && (effect.action != Action::CalendarUpdate
                        || tool.provider_revision.as_deref()
                            == Some(event.key.revision.as_str())) =>
            {
                runtime.insert(tool.agent.clone());
            }
            EventPayload::Claim(claim)
                if candidate.key.source == Source::Messages
                    && claim_matches(events, candidate, claim, event, effect) =>
            {
                reports.insert(claim.agent.clone());
            }
            _ => {}
        }
        if let Some((feedback, _)) = feedback(candidate)
            && feedback.finding == finding
            && candidate.ingested_at >= event.ingested_at
            && let Some(actor) = &feedback.actor
            && confirmed
                .as_ref()
                .is_none_or(|(at, _)| candidate.ingested_at > *at)
        {
            confirmed = Some((candidate.ingested_at, actor.clone()));
        }
    }
    if let Some((_, actor)) = confirmed {
        return ActorEvidence::UserConfirmed(actor);
    }
    if runtime.len() > 1 {
        return ActorEvidence::Conflicting(runtime.into_iter().collect());
    }
    if let Some(agent) = runtime.into_iter().next() {
        return ActorEvidence::RuntimeLinked(agent);
    }
    if reports.len() > 1 {
        return ActorEvidence::Conflicting(reports.into_iter().collect());
    }
    reports
        .into_iter()
        .next()
        .map_or(ActorEvidence::Unknown, ActorEvidence::Reported)
}

fn claim_matches(
    events: &[Event],
    report: &Event,
    claim: &crate::event::Claim,
    event: &Event,
    effect: &Effect,
) -> bool {
    if claim.action != effect.action
        || !effect.targets_complete
        || !same_targets(&claim.targets, &effect.targets)
    {
        return false;
    }
    if let Some(resource) = &claim.provider_resource {
        return resource == &event.key.resource;
    }
    if (report.occurred_at - event.occurred_at).num_seconds().abs() > CLAIM_WINDOW.num_seconds() {
        return false;
    }
    // A report without a provider ID cannot identify one of two similar effects.
    events
        .iter()
        .filter(|candidate| {
            candidate.key.account == event.key.account
                && (report.occurred_at - candidate.occurred_at)
                    .num_seconds()
                    .abs()
                    <= CLAIM_WINDOW.num_seconds()
                && matches!(&candidate.payload, EventPayload::Effect(other)
                    if other.origin != Origin::Baseline && other.action == claim.action
                        && other.targets_complete && same_targets(&other.targets, &claim.targets))
        })
        .take(2)
        .count()
        == 1
}

fn policy_violations(events: &[Event], event: &Event, effect: &Effect) -> Vec<(String, bool)> {
    let mut latest = BTreeMap::new();
    for candidate in events {
        if candidate.key.account != event.key.account {
            continue;
        }
        if let Some((policy, occurred_at)) = policy(candidate) {
            if occurred_at > event.occurred_at {
                continue;
            }
            let entry = latest.entry(&policy.id).or_insert((occurred_at, policy));
            if occurred_at > entry.0 {
                *entry = (occurred_at, policy);
            }
        }
    }
    latest
        .into_iter()
        .filter(|(_, (_, policy))| {
            policy.enabled
                && (policy.forbidden_actions.contains(&effect.action)
                    || effect
                        .targets
                        .iter()
                        .any(|target| policy.forbidden_targets.contains(target)))
        })
        .map(|(id, (_, policy))| (id.clone(), policy.high_consequence))
        .collect()
}

fn feedback(event: &Event) -> Option<(&Feedback, DateTime<Utc>)> {
    match &event.payload {
        EventPayload::Feedback(value) if event.key.source == Source::Picket => {
            Some((value, event.occurred_at))
        }
        EventPayload::Snapshot(snapshot)
            if event.key.source == Source::Picket && snapshot.original_source == Source::Picket =>
        {
            if let SnapshotValue::Feedback(value) = &snapshot.value {
                Some((value, snapshot.original_occurred_at))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn policy(event: &Event) -> Option<(&AccountPolicy, DateTime<Utc>)> {
    match &event.payload {
        EventPayload::Policy(value) if event.key.source == Source::Picket => {
            Some((value, event.occurred_at))
        }
        EventPayload::Snapshot(snapshot)
            if event.key.source == Source::Picket && snapshot.original_source == Source::Picket =>
        {
            if let SnapshotValue::Policy(value) = &snapshot.value {
                Some((value, snapshot.original_occurred_at))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn request_fit(
    events: &[Event],
    event: &Event,
    effect: &Effect,
    actor: &ActorEvidence,
) -> RequestFit {
    let agent = match actor {
        ActorEvidence::Reported(agent) | ActorEvidence::RuntimeLinked(agent) => agent,
        _ => return RequestFit::NotAssessed,
    };
    let task_hints: BTreeSet<_> = events
        .iter()
        .filter_map(|candidate| {
            if candidate.key.account != event.key.account {
                return None;
            }
            match (&actor, &candidate.payload) {
                (ActorEvidence::RuntimeLinked(_), EventPayload::ToolResult(tool))
                    if candidate.key.source == Source::OpenClaw
                        && tool.agent == *agent
                        && tool.outcome == ToolOutcome::Succeeded
                        && tool.provider_resource.as_deref() == Some(&event.key.resource)
                        && tool.action == effect.action
                        && effect.targets_complete
                        && same_targets(&tool.targets, &effect.targets)
                        && (effect.action != Action::CalendarUpdate
                            || tool.provider_revision.as_deref() == Some(&event.key.revision)) =>
                {
                    tool.task.as_ref()
                }
                (ActorEvidence::Reported(_), EventPayload::Claim(claim))
                    if candidate.key.source == Source::Messages
                        && claim.agent == *agent
                        && claim.provider_resource.as_deref() == Some(&event.key.resource)
                        && claim.action == effect.action
                        && same_targets(&claim.targets, &effect.targets) =>
                {
                    claim.task.as_ref()
                }
                _ => None,
            }
        })
        .collect();
    if task_hints.len() > 1 {
        return RequestFit::AmbiguousTask;
    }
    let task_hint = task_hints.into_iter().next();
    let candidates: Vec<_> = events
        .iter()
        .filter_map(|candidate| {
            if candidate.key.account != event.key.account
                || candidate.key.source != Source::Messages
                || candidate.occurred_at > event.occurred_at
            {
                return None;
            }
            match &candidate.payload {
                EventPayload::Request(request)
                    if request.agent == *agent
                        && request
                            .valid_until
                            .is_none_or(|until| event.occurred_at <= until)
                        && task_hint.is_none_or(|task| request.task == *task) =>
                {
                    Some((candidate.occurred_at, request))
                }
                _ => None,
            }
        })
        .collect();
    if candidates.len() > MAX_CANDIDATES {
        return RequestFit::InsufficientEvidence;
    }
    let mut latest = BTreeMap::new();
    for (at, request) in candidates {
        let entry = latest.entry(&request.task).or_insert((at, request));
        if at > entry.0 {
            *entry = (at, request);
        }
    }
    match latest.len() {
        0 => RequestFit::NoObservedRequest,
        1 => match latest.into_values().next() {
            Some((_, request)) => compare_request(request, effect),
            None => RequestFit::NoObservedRequest,
        },
        _ => RequestFit::AmbiguousTask,
    }
}

fn compare_request(request: &Request, effect: &Effect) -> RequestFit {
    let mut conflicts = Vec::new();
    if request.prohibited_actions.contains(&effect.action) {
        conflicts.push(ConflictReason::ProhibitedAction);
    }
    if let Field::Known(actions) = &request.actions
        && !actions.contains(&effect.action)
    {
        conflicts.push(ConflictReason::DifferentAction);
    }
    if let Field::Known(targets) = &request.targets
        && effect
            .targets
            .iter()
            .any(|target| !targets.contains(target))
    {
        conflicts.push(ConflictReason::ExtraTarget);
    }
    if request.requires_approval == Field::Known(true) && !request.approved {
        conflicts.push(ConflictReason::ApprovalMissing);
    }
    // Account-wide counts include human actions. Quantity remains unassessed
    // until effects can be explicitly linked to task slots.
    if !conflicts.is_empty() {
        return RequestFit::Conflicts(conflicts);
    }
    if !effect.targets_complete
        || !matches!(request.actions, Field::Known(_) | Field::ExplicitAny)
        || !matches!(request.targets, Field::Known(_) | Field::ExplicitAny)
        || !matches!(request.requires_approval, Field::Known(_))
        || !matches!(request.max_actions, Field::ExplicitAny)
    {
        return RequestFit::InsufficientEvidence;
    }
    RequestFit::ConsistentWithVisibleRequest
}

fn same_targets(left: &[String], right: &[String]) -> bool {
    !left.is_empty()
        && left.len() == right.len()
        && left.iter().collect::<BTreeSet<_>>() == right.iter().collect::<BTreeSet<_>>()
}

fn finding_id(key: &EventKey) -> String {
    // EventKey is a fixed shape of strings and an enum; serialization is infallible.
    let mut hash = Sha256::new();
    for part in [
        key.account.as_str(),
        key.source.as_str(),
        key.resource.as_str(),
        key.revision.as_str(),
    ] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    format!("effect/{:x}", hash.finalize())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::{
        event::{AccountPolicy, Claim, Feedback, ToolResult},
        store::Store,
    };

    fn time(minute: i64) -> DateTime<Utc> {
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
            occurred_at: time(minute),
            ingested_at: time(minute),
            payload,
        }
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
                origin: Origin::Live,
                external_change: false,
            }),
        )
    }

    fn tool() -> Event {
        event(
            Source::OpenClaw,
            "call-1",
            10,
            EventPayload::ToolResult(ToolResult {
                agent: "agent-1".into(),
                task: Some("task-1".into()),
                action: Action::EmailSend,
                targets: vec!["a@example.test".into()],
                provider_resource: Some("message-1".into()),
                provider_revision: None,
                outcome: ToolOutcome::Succeeded,
            }),
        )
    }

    fn request() -> Event {
        event(
            Source::Messages,
            "user-request",
            0,
            EventPayload::Request(Request {
                agent: "agent-1".into(),
                task: "task-1".into(),
                actions: Field::Known(vec![Action::EmailSend]),
                targets: Field::Known(vec!["a@example.test".into()]),
                prohibited_actions: vec![],
                max_actions: Field::ExplicitAny,
                valid_until: None,
                requires_approval: Field::Known(false),
                approved: false,
            }),
        )
    }

    fn one(events: &[Event]) -> Assessment {
        assess(events, time(20)).remove(0)
    }

    #[test]
    fn effects_only_and_human_phone_activity_remain_unknown() {
        let result = one(&[request(), effect()]);
        assert_eq!(result.actor, ActorEvidence::Unknown);
        assert_eq!(result.request_fit, RequestFit::NotAssessed);
        assert_eq!(result.route, Route::Timeline);
    }

    #[test]
    fn matching_runtime_result_is_qualified_and_replay_is_stable() -> crate::store::Result<()> {
        let inputs = [request(), effect(), tool()];
        let result = one(&inputs);
        assert_eq!(result.actor, ActorEvidence::RuntimeLinked("agent-1".into()));
        assert_eq!(result.request_fit, RequestFit::ConsistentWithVisibleRequest);
        assert_eq!(assess(&inputs, time(20)), assess(&inputs, time(20)));
        let mut store = Store::memory()?;
        for input in &inputs {
            store.append(input)?;
        }
        let replay: Vec<_> = store.events()?.into_iter().map(|item| item.event).collect();
        assert_eq!(assess(&replay, time(20)), assess(&inputs, time(20)));
        Ok(())
    }

    #[test]
    fn failed_or_unlinked_tool_result_cannot_name_agent() {
        let mut failed = tool();
        if let EventPayload::ToolResult(value) = &mut failed.payload {
            value.outcome = ToolOutcome::Failed;
        }
        assert_eq!(one(&[effect(), failed]).actor, ActorEvidence::Unknown);
        let mut wrong = tool();
        if let EventPayload::ToolResult(value) = &mut wrong.payload {
            value.provider_resource = Some("other".into());
        }
        assert_eq!(one(&[effect(), wrong]).actor, ActorEvidence::Unknown);
        let mut incomplete = effect();
        if let EventPayload::Effect(value) = &mut incomplete.payload {
            value.targets_complete = false;
        }
        assert_eq!(one(&[incomplete, tool()]).actor, ActorEvidence::Unknown);
    }

    #[test]
    fn copied_claim_is_only_reported_and_does_not_override_runtime_link() {
        let claim = event(
            Source::Messages,
            "claim-1",
            11,
            EventPayload::Claim(Claim {
                agent: "agent-2".into(),
                task: None,
                action: Action::EmailSend,
                targets: vec!["a@example.test".into()],
                provider_resource: Some("message-1".into()),
            }),
        );
        assert_eq!(
            one(&[effect(), claim.clone()]).actor,
            ActorEvidence::Reported("agent-2".into())
        );
        assert_eq!(
            one(&[effect(), claim, tool()]).actor,
            ActorEvidence::RuntimeLinked("agent-1".into())
        );
    }

    #[test]
    fn body_only_claim_does_not_select_between_human_and_agent_sends() {
        let claim = event(
            Source::Messages,
            "claim-1",
            11,
            EventPayload::Claim(Claim {
                agent: "agent-1".into(),
                task: None,
                action: Action::EmailSend,
                targets: vec!["a@example.test".into()],
                provider_resource: None,
            }),
        );
        assert_eq!(
            one(&[effect(), claim.clone()]).actor,
            ActorEvidence::Reported("agent-1".into())
        );
        let mut second = effect();
        second.key.resource = "message-2".into();
        let assessments = assess(&[effect(), second, claim], time(20));
        assert!(
            assessments
                .iter()
                .all(|item| item.actor == ActorEvidence::Unknown)
        );
    }

    #[test]
    fn contradictory_runtime_links_remain_conflicting() {
        let mut second = tool();
        second.key.resource = "call-2".into();
        if let EventPayload::ToolResult(value) = &mut second.payload {
            value.agent = "agent-2".into();
        }
        assert_eq!(
            one(&[effect(), tool(), second]).actor,
            ActorEvidence::Conflicting(vec!["agent-1".into(), "agent-2".into()])
        );
    }

    #[test]
    fn explicit_policy_breach_alerts_without_actor_or_ai() {
        let policy = event(
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
        );
        let result = one(&[policy, effect()]);
        assert_eq!(result.actor, ActorEvidence::Unknown);
        assert_eq!(result.policy_ids, vec!["no-send"]);
        assert_eq!(result.route, Route::HighReview);
    }

    #[test]
    fn prohibitions_extra_targets_and_missing_approval_cannot_be_outscored() {
        let mut sent = effect();
        if let EventPayload::Effect(value) = &mut sent.payload {
            value.targets.push("b@example.test".into());
        }
        let mut linked = tool();
        if let EventPayload::ToolResult(value) = &mut linked.payload {
            value.targets.push("b@example.test".into());
        }
        let mut asked = request();
        if let EventPayload::Request(value) = &mut asked.payload {
            value.prohibited_actions.push(Action::EmailSend);
            value.requires_approval = Field::Known(true);
        }
        assert_eq!(
            one(&[asked, sent, linked]).request_fit,
            RequestFit::Conflicts(vec![
                ConflictReason::ProhibitedAction,
                ConflictReason::ExtraTarget,
                ConflictReason::ApprovalMissing,
            ])
        );
    }

    #[test]
    fn late_request_does_not_authorize_and_missing_targets_abstain() {
        let mut late = request();
        late.occurred_at = time(11);
        assert_eq!(
            one(&[late, effect(), tool()]).request_fit,
            RequestFit::NoObservedRequest
        );
        let mut incomplete = effect();
        if let EventPayload::Effect(value) = &mut incomplete.payload {
            value.targets_complete = false;
        }
        assert_eq!(
            one(&[request(), incomplete, tool()]).request_fit,
            RequestFit::NotAssessed
        );
    }

    #[test]
    fn external_calendar_update_cannot_be_claimed_by_agent() {
        let mut changed = effect();
        if let EventPayload::Effect(value) = &mut changed.payload {
            value.external_change = true;
        }
        assert_eq!(one(&[changed, tool()]).actor, ActorEvidence::Unknown);
    }

    #[test]
    fn feedback_resolves_only_its_finding_and_does_not_name_actor() {
        let result = one(&[effect()]);
        let feedback = event(
            Source::Picket,
            "feedback-1",
            15,
            EventPayload::Feedback(Feedback {
                finding: result.finding,
                expected: Some(true),
                actor: None,
            }),
        );
        let result = one(&[effect(), feedback]);
        assert_eq!(result.route, Route::Timeline);
        assert_eq!(result.actor, ActorEvidence::Unknown);
        assert_eq!(result.expected_feedback, Some(true));
    }

    #[test]
    fn future_source_clock_cannot_hold_review_open_forever() {
        let mut sent = effect();
        sent.occurred_at = time(1000);
        sent.ingested_at = time(10);
        let result = assess(&[sent], time(20)).remove(0);
        assert!(!result.pending);
    }

    #[test]
    fn later_task_revision_changes_only_later_assessments() {
        let mut updated = request();
        updated.key.revision = "2".into();
        updated.occurred_at = time(11);
        if let EventPayload::Request(value) = &mut updated.payload {
            value.prohibited_actions.push(Action::EmailSend);
        }
        assert_eq!(
            one(&[request(), updated, effect(), tool()]).request_fit,
            RequestFit::ConsistentWithVisibleRequest
        );
    }

    #[test]
    fn multiple_relevant_tasks_abstain_instead_of_choosing_permissive_one() {
        let mut second = request();
        second.key.resource = "other-request".into();
        if let EventPayload::Request(value) = &mut second.payload {
            value.task = "task-2".into();
            value.prohibited_actions.push(Action::EmailSend);
        }
        let mut unhinted = tool();
        if let EventPayload::ToolResult(value) = &mut unhinted.payload {
            value.task = None;
        }
        assert_eq!(
            one(&[request(), second, effect(), unhinted]).request_fit,
            RequestFit::AmbiguousTask
        );
    }

    #[test]
    fn calendar_update_requires_exact_revision() {
        let mut changed = effect();
        changed.key.source = Source::Calendar;
        if let EventPayload::Effect(value) = &mut changed.payload {
            value.action = Action::CalendarUpdate;
        }
        let mut linked = tool();
        if let EventPayload::ToolResult(value) = &mut linked.payload {
            value.action = Action::CalendarUpdate;
        }
        assert_eq!(
            one(&[changed.clone(), linked.clone()]).actor,
            ActorEvidence::Unknown
        );
        if let EventPayload::ToolResult(value) = &mut linked.payload {
            value.provider_revision = Some("1".into());
        }
        assert_eq!(
            one(&[changed, linked]).actor,
            ActorEvidence::RuntimeLinked("agent-1".into())
        );
    }

    #[test]
    fn policy_revision_applies_only_when_effective_and_high_is_explicit() {
        let mut policy = event(
            Source::Picket,
            "policy-1",
            0,
            EventPayload::Policy(AccountPolicy {
                id: "no-send".into(),
                forbidden_actions: vec![Action::EmailSend],
                forbidden_targets: vec![],
                high_consequence: false,
                enabled: true,
            }),
        );
        assert_eq!(one(&[policy.clone(), effect()]).route, Route::MediumReview);
        policy.occurred_at = time(11);
        assert_eq!(one(&[policy, effect()]).route, Route::Timeline);
    }
}
