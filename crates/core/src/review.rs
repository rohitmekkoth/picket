//! A privacy-safe timeline for live validation: what was captured, extracted, reported
//! and observed, and how the engine assessed it. Lines carry kinds, actions, counts and
//! short hashes of source identities, never message text, subjects or addresses.

use std::collections::BTreeMap;

use chrono::{DateTime, SecondsFormat, Utc};
use sha2::{Digest, Sha256};

use crate::{
    engine::{self, Assessment, ClaimCheck},
    event::{EventKey, EventPayload, Field, Source},
};

/// One timeline line per retained input since `since`, assessed as of `at`.
pub fn timeline(
    events: &[crate::event::Event],
    since: DateTime<Utc>,
    at: DateTime<Utc>,
) -> Vec<String> {
    let assessments: BTreeMap<_, _> = engine::assess(events, at)
        .into_iter()
        .map(|item| (identity(&item.effect), item))
        .collect();
    let claims: BTreeMap<_, _> = engine::assess_claims(events, at)
        .into_iter()
        .map(|item| (identity(&item.claim), item))
        .collect();
    let mut rows: Vec<_> = events
        .iter()
        .filter(|event| event.occurred_at >= since)
        .filter_map(|event| {
            let detail = match &event.payload {
                EventPayload::Message(message) => format!(
                    "message    {} {:?} {:?}",
                    message.agent, message.direction, message.text
                ),
                EventPayload::Extraction(record) => format!(
                    "extraction {} {:?} prompt v{}",
                    record.agent, record.outcome, record.prompt_version
                ),
                EventPayload::Request(request) => format!(
                    "request    task {} actions {} prohibited {:?} targets {} named {} start {} max {} approval {} approved {}",
                    short(&request.task),
                    field(&request.actions, |value| format!("{value:?}")),
                    request.prohibited_actions,
                    field(&request.targets, |value| value.len().to_string()),
                    request.target_names.len(),
                    field(&request.calendar_start, |_| "known".into()),
                    field(&request.max_actions, ToString::to_string),
                    field(&request.requires_approval, |value| {
                        if *value { "required" } else { "not required" }.into()
                    }),
                    request.approved
                ),
                EventPayload::Claim(claim) => format!(
                    "claim      {:?} channel {:?} task {} targets {} named {} resource {} -> {}",
                    claim.action,
                    claim.channel,
                    claim.task.as_deref().map_or("none".into(), short),
                    claim.targets.len(),
                    claim.target_names.len(),
                    claim.provider_resource.is_some(),
                    claims
                        .get(&identity(&event.key))
                        .map_or("unassessed".into(), claim_result)
                ),
                EventPayload::ToolResult(tool) => format!(
                    "tool       {} {:?} {:?} targets {} resource {} revision {}",
                    tool.agent,
                    tool.action,
                    tool.outcome,
                    tool.targets.len(),
                    tool.provider_resource.is_some(),
                    tool.provider_revision.is_some()
                ),
                EventPayload::Effect(effect) => format!(
                    "effect     {:?} {:?} targets {} -> {}",
                    effect.action,
                    event.key.source,
                    effect.targets.len(),
                    assessments
                        .get(&identity(&event.key))
                        .map_or("unassessed".into(), assessment)
                ),
                EventPayload::Coverage(coverage)
                    if matches!(event.key.source, Source::Messages | Source::OpenClaw)
                        || coverage.capability == "effects" =>
                {
                    format!(
                        "coverage   {:?} {} {:?} {:?}",
                        event.key.source, coverage.capability, coverage.state, coverage.reason
                    )
                }
                _ => return None,
            };
            Some((
                event.occurred_at,
                format!(
                    "{} {} {}",
                    event.occurred_at.to_rfc3339_opts(SecondsFormat::Secs, true),
                    short(event.key.resource.split('#').next().unwrap_or_default()),
                    detail
                ),
            ))
        })
        .collect();
    rows.sort_by_key(|(time, _)| *time);
    rows.into_iter().map(|(_, line)| line).collect()
}

/// The retained inputs with what extraction derived from messages since `since` left
/// out, so the current extractor can be replayed on the same conversation in a scratch
/// store. Checkpoints describe the original chain and are dropped too.
pub fn without_extractions_since(
    events: Vec<crate::event::Event>,
    since: DateTime<Utc>,
) -> Vec<crate::event::Event> {
    events
        .into_iter()
        .filter(|event| match &event.payload {
            EventPayload::Request(_) | EventPayload::Claim(_) | EventPayload::Extraction(_) => {
                event.key.source != Source::Messages || event.occurred_at < since
            }
            EventPayload::Checkpoint { .. } => false,
            _ => true,
        })
        .collect()
}

fn identity(key: &EventKey) -> (String, &'static str, String, String) {
    (
        key.account.clone(),
        key.source.as_str(),
        key.resource.clone(),
        key.revision.clone(),
    )
}

fn assessment(item: &Assessment) -> String {
    let actor = match &item.actor {
        engine::ActorEvidence::Unknown => "actor unknown".to_owned(),
        engine::ActorEvidence::Reported(agent) => format!("reported by {agent}"),
        engine::ActorEvidence::RuntimeLinked(agent) => format!("runtime linked to {agent}"),
        engine::ActorEvidence::UserConfirmed(agent) => format!("user confirmed {agent}"),
        engine::ActorEvidence::Conflicting(agents) => format!("conflicting {agents:?}"),
    };
    format!("{actor}, {:?}, {:?}", item.request_fit, item.route)
}

fn claim_result(item: &ClaimCheck) -> String {
    format!("{:?}", item.status)
}

fn field<T>(value: &Field<T>, show: impl Fn(&T) -> String) -> String {
    match value {
        Field::Known(value) => show(value),
        Field::ExplicitAny => "any".into(),
        Field::Unknown => "unknown".into(),
        Field::Ambiguous => "ambiguous".into(),
    }
}

/// A stable eight-character tag that correlates lines without revealing the identity.
fn short(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .take(4)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::event::{
        Action, Effect, Event, MessageDirection, MessageObserved, MessageText, Origin, Request,
    };

    fn at(minutes: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 2, 9, 0, 0)
            .single()
            .unwrap_or_else(|| panic!("fixed time"))
            + chrono::Duration::minutes(minutes)
    }

    fn event(source: Source, resource: &str, minutes: i64, payload: EventPayload) -> Event {
        Event {
            schema_version: 1,
            key: EventKey {
                account: "account-1".into(),
                source,
                resource: resource.into(),
                revision: "0".into(),
            },
            occurred_at: at(minutes),
            ingested_at: at(minutes),
            source_time_quality: None,
            adapter: None,
            payload,
        }
    }

    #[test]
    fn lines_summarize_without_text_subjects_or_addresses() {
        let events = [
            event(
                Source::Messages,
                "guid-1",
                0,
                EventPayload::Message(MessageObserved {
                    agent: "instinct".into(),
                    direction: MessageDirection::FromUser,
                    service: "iMessage".into(),
                    text: MessageText::Decoded,
                    snippet: "Email Sam the secret plan".into(),
                    origin: Origin::Live,
                }),
            ),
            event(
                Source::Messages,
                "guid-1#request",
                0,
                EventPayload::Request(Request {
                    agent: "instinct".into(),
                    task: "guid-1".into(),
                    actions: Field::Known(vec![Action::EmailSend]),
                    targets: Field::Known(vec!["sam@example.test".into()]),
                    calendar_start: Field::Unknown,
                    prohibited_actions: Vec::new(),
                    max_actions: Field::Unknown,
                    valid_until: None,
                    requires_approval: Field::Known(false),
                    approved: false,
                    target_names: vec!["Sam".into()],
                }),
            ),
            event(
                Source::Gmail,
                "message-1",
                1,
                EventPayload::Effect(Effect {
                    action: Action::EmailSend,
                    targets: vec!["sam@example.test".into()],
                    targets_complete: true,
                    calendar_start: None,
                    origin: Origin::Live,
                    external_change: false,
                    target_names: BTreeMap::new(),
                    email: None,
                    calendar: None,
                }),
            ),
        ];
        let lines = timeline(&events, at(0), at(5));
        assert_eq!(lines.len(), 3);
        assert!(
            lines
                .iter()
                .all(|line| !line.contains("sam@") && !line.contains("secret"))
        );
        assert!(
            lines
                .iter()
                .all(|line| !line.contains("guid-1") && !line.contains("Sam\""))
        );
        let tag = short("guid-1");
        assert!(
            lines[0].contains(&tag) && lines[1].contains(&tag),
            "derived records share the tag"
        );
        assert!(lines[1].contains("actions [EmailSend]") && lines[1].contains("targets 1 named 1"));
        assert!(lines[2].contains("effect     EmailSend Gmail targets 1 -> actor unknown"));
        assert!(
            timeline(&events, at(1), at(5)).len() == 1,
            "older inputs are left out"
        );
    }
}
