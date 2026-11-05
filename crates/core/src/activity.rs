//! The Overview counts and the Activity timeline. Verdicts come from assessing each
//! input against the same evidence the findings use, so the lists agree with Review.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::{
    engine::{ActorEvidence, ClaimStatus, Evidence, RequestFit, effect_finding_id},
    event::{
        Action, AgentRegistration, ClaimChannel, CoverageReason, CoverageState, Event,
        EventPayload, Source,
    },
    views::{self, Person},
};

const MAX_PAGE: usize = 200;

/// What a list row shows about how an input was judged.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Linked to an agent and consistent with the visible request.
    AsAsked,
    Review,
    /// Linked to an agent, but the request or report could not settle it.
    Unclear,
    /// Not linked to any agent.
    NotLinked,
    /// Requests and notes carry no verdict.
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Email,
    Calendar,
    Request,
    Report,
    Gap,
    Recovered,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityItem {
    /// Stable within a log, for paging.
    pub id: String,
    pub at: DateTime<Utc>,
    pub kind: ItemKind,
    pub action: Option<Action>,
    pub subject: Option<String>,
    pub people: Vec<Person>,
    pub message: Option<String>,
    pub agent: Option<String>,
    pub verdict: Verdict,
    /// The incident to open, when the item is under review.
    pub incident: Option<String>,
    /// A report about the agent's own mailbox, which Picket cannot see.
    pub outside_scope: bool,
    pub source: Option<Source>,
    pub gap_reason: Option<CoverageReason>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityPage {
    pub items: Vec<ActivityItem>,
    /// Pass back as `before` for the next page.
    pub next: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    /// Effects linked to an agent.
    pub agent_actions: usize,
    pub as_asked: usize,
    pub review: usize,
    pub unclear: usize,
    pub not_linked: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    pub source: Source,
    /// `effects` for Gmail and Calendar, the agent id for request capture.
    pub scope: String,
    pub state: CoverageState,
    pub reason: Option<CoverageReason>,
    pub since: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverviewView {
    pub counts: Counts,
    pub recent: Vec<ActivityItem>,
    pub sources: Vec<SourceView>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Who {
    Everyone,
    Agents,
    NotLinked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Filter {
    Any,
    Review,
    AsAsked,
    Unclear,
}

pub struct Query<'a> {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub who: Who,
    pub result: Filter,
    pub before: Option<&'a str>,
    pub limit: usize,
}

pub fn overview(
    evidence: &Evidence,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    now: DateTime<Utc>,
    agents: &[AgentRegistration],
    messages: &impl views::Messages,
) -> OverviewView {
    let items = items(evidence, from, to, now, agents, messages);
    let mut counts = Counts::default();
    for item in items
        .iter()
        .filter(|item| matches!(item.kind, ItemKind::Email | ItemKind::Calendar))
    {
        match item.verdict {
            Verdict::NotLinked => counts.not_linked += 1,
            Verdict::None => {}
            verdict => {
                counts.agent_actions += 1;
                match verdict {
                    Verdict::AsAsked => counts.as_asked += 1,
                    Verdict::Review => counts.review += 1,
                    _ => counts.unclear += 1,
                }
            }
        }
    }
    let recent = items
        .into_iter()
        .filter(|item| item.agent.is_some() && item.kind != ItemKind::Request)
        .take(5)
        .collect();
    let mut latest: BTreeMap<(&'static str, String), SourceView> = BTreeMap::new();
    for (_, source, coverage, at) in evidence.coverage_history() {
        let scope = coverage
            .capability
            .strip_prefix("requests_")
            .unwrap_or(&coverage.capability)
            .to_owned();
        latest.insert(
            (source.as_str(), scope.clone()),
            SourceView {
                source,
                scope,
                state: coverage.state.clone(),
                reason: coverage.reason.clone(),
                since: at,
            },
        );
    }
    OverviewView {
        counts,
        recent,
        sources: latest.into_values().collect(),
    }
}

pub fn activity(
    evidence: &Evidence,
    query: &Query<'_>,
    now: DateTime<Utc>,
    agents: &[AgentRegistration],
    messages: &impl views::Messages,
) -> ActivityPage {
    let limit = query.limit.clamp(1, MAX_PAGE);
    let cursor = query.before.and_then(parse_cursor);
    let mut page: Vec<_> = items(evidence, query.from, query.to, now, agents, messages)
        .into_iter()
        .filter(|item| {
            cursor
                .as_ref()
                .is_none_or(|(at, id)| (item.at, item.id.as_str()) < (*at, id.as_str()))
        })
        .filter(|item| match query.who {
            Who::Everyone => true,
            Who::Agents => item.agent.is_some(),
            Who::NotLinked => item.verdict == Verdict::NotLinked,
        })
        .filter(|item| match query.result {
            Filter::Any => true,
            Filter::Review => item.verdict == Verdict::Review,
            Filter::AsAsked => item.verdict == Verdict::AsAsked,
            Filter::Unclear => item.verdict == Verdict::Unclear,
        })
        .take(limit + 1)
        .collect();
    let next = (page.len() > limit).then(|| {
        page.truncate(limit);
        page.last()
            .map(|item| format!("{}|{}", item.at.to_rfc3339(), item.id))
    });
    ActivityPage {
        items: page,
        next: next.flatten(),
    }
}

fn parse_cursor(value: &str) -> Option<(DateTime<Utc>, String)> {
    let (at, id) = value.split_once('|')?;
    let at = DateTime::parse_from_rfc3339(at).ok()?.with_timezone(&Utc);
    Some((at, id.to_owned()))
}

/// Every listed input in a period, newest first.
fn items(
    evidence: &Evidence,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    now: DateTime<Utc>,
    agents: &[AgentRegistration],
    messages: &impl views::Messages,
) -> Vec<ActivityItem> {
    let name = |id: &str| {
        agents
            .iter()
            .find(|agent| agent.id == id)
            .map_or_else(|| id.to_owned(), |agent| agent.name.clone())
    };
    let quote = |event: &Event| {
        let resource = event.key.resource.split('#').next()?;
        messages
            .message(&crate::event::EventKey {
                resource: resource.to_owned(),
                ..event.key.clone()
            })
            .map(|message| message.snippet)
            .filter(|snippet| !snippet.is_empty())
    };
    let mut items = Vec::new();
    for event in evidence.effects_between(from, to) {
        let EventPayload::Effect(effect) = &event.payload else {
            continue;
        };
        let Some(assessment) = evidence.effect_assessment(event, now) else {
            continue;
        };
        let agent = match &assessment.actor {
            ActorEvidence::Reported(agent)
            | ActorEvidence::RuntimeLinked(agent)
            | ActorEvidence::UserConfirmed(agent) => Some(name(agent)),
            ActorEvidence::Conflicting(_) | ActorEvidence::Unknown => None,
        };
        let verdict = if assessment.route.needs_review() {
            Verdict::Review
        } else if agent.is_none() {
            Verdict::NotLinked
        } else if assessment.request_fit == RequestFit::ConsistentWithVisibleRequest {
            Verdict::AsAsked
        } else {
            Verdict::Unclear
        };
        let extra = evidence
            .effect_basis(&assessment)
            .map(|basis| basis.extra_targets)
            .unwrap_or_default();
        items.push(ActivityItem {
            id: effect_finding_id(&event.key),
            at: event.occurred_at,
            kind: if event.key.source == Source::Calendar {
                ItemKind::Calendar
            } else {
                ItemKind::Email
            },
            action: Some(effect.action.clone()),
            subject: effect
                .email
                .as_ref()
                .map(|email| email.subject_snippet.clone())
                .or_else(|| {
                    effect
                        .calendar
                        .as_ref()
                        .and_then(|calendar| calendar.summary.clone())
                })
                .filter(|subject| !subject.is_empty()),
            people: effect
                .targets
                .iter()
                .map(|address| Person {
                    address: address.clone(),
                    name: effect.target_names.get(address).cloned(),
                    unrequested: extra.contains(address),
                })
                .collect(),
            message: None,
            agent,
            verdict,
            incident: (verdict == Verdict::Review).then(|| incident_id(&event.key)),
            outside_scope: false,
            source: Some(event.key.source.clone()),
            gap_reason: None,
        });
    }
    for event in evidence.claims_between(from, to) {
        let EventPayload::Claim(claim) = &event.payload else {
            continue;
        };
        let check = evidence.report_check(event, now);
        let status = check.as_ref().map(|check| check.status);
        // A matched report is shown on the effect it matched; repeating it adds nothing.
        if status == Some(ClaimStatus::Matched) {
            continue;
        }
        let outside = claim.channel == ClaimChannel::AgentAccount
            || status == Some(ClaimStatus::OutsideObservedScope);
        let review = matches!(
            status,
            Some(
                ClaimStatus::NotObservedInHealthySource | ClaimStatus::ContradictedByObservedEffect
            )
        ) && check
            .as_ref()
            .is_some_and(|check| evidence.claim_route(check).needs_review());
        items.push(ActivityItem {
            id: format!("report/{}/{}", event.key.resource, event.key.revision),
            at: event.occurred_at,
            kind: ItemKind::Report,
            action: Some(claim.action.clone()),
            subject: None,
            people: claim
                .targets
                .iter()
                .map(|address| Person {
                    address: address.clone(),
                    name: None,
                    unrequested: false,
                })
                .chain(claim.target_names.iter().map(|name| Person {
                    address: String::new(),
                    name: Some(name.clone()),
                    unrequested: false,
                }))
                .collect(),
            message: quote(event),
            agent: Some(name(&claim.agent)),
            verdict: if review {
                Verdict::Review
            } else {
                Verdict::Unclear
            },
            incident: review.then(|| {
                let subject = check
                    .as_ref()
                    .and_then(|check| check.observed_effect.clone())
                    .unwrap_or_else(|| event.key.clone());
                incident_id(&subject)
            }),
            outside_scope: outside,
            source: None,
            gap_reason: None,
        });
    }
    for event in evidence.requests_between(from, to) {
        let EventPayload::Request(request) = &event.payload else {
            continue;
        };
        items.push(ActivityItem {
            id: format!("request/{}/{}", event.key.resource, event.key.revision),
            at: event.occurred_at,
            kind: ItemKind::Request,
            action: None,
            subject: None,
            people: Vec::new(),
            message: quote(event),
            agent: Some(name(&request.agent)),
            verdict: Verdict::None,
            incident: None,
            outside_scope: false,
            source: None,
            gap_reason: None,
        });
    }
    let mut previous: BTreeMap<(String, &'static str, String), CoverageState> = BTreeMap::new();
    for (account, source, coverage, at) in evidence.coverage_history() {
        let key = (
            account.to_owned(),
            source.as_str(),
            coverage.capability.clone(),
        );
        let before = previous.insert(key, coverage.state.clone());
        let kind = match (&before, &coverage.state) {
            (Some(CoverageState::Healthy) | None, CoverageState::Healthy) => continue,
            (_, CoverageState::Healthy) => ItemKind::Recovered,
            (Some(state), current) if state == current => continue,
            _ => ItemKind::Gap,
        };
        if at < from || at > to {
            continue;
        }
        items.push(ActivityItem {
            id: format!(
                "coverage/{account}/{}/{}/{}",
                source.as_str(),
                coverage.capability,
                at.timestamp()
            ),
            at,
            kind,
            action: None,
            subject: None,
            people: Vec::new(),
            message: None,
            agent: None,
            verdict: Verdict::None,
            incident: None,
            outside_scope: false,
            source: Some(source),
            gap_reason: coverage.reason.clone(),
        });
    }
    items.sort_by(|left, right| (right.at, &right.id).cmp(&(left.at, &left.id)));
    items
}

/// The incident group a subject belongs to, as the engine names it.
fn incident_id(key: &crate::event::EventKey) -> String {
    crate::engine::incident_group(key)
}
