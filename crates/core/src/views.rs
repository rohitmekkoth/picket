//! Read models for the desktop UI. Each incident is built from the same evidence that
//! produced its findings, so a screen cannot show a request, report or recipient the
//! engine did not use. Values are codes and data; wording belongs to the UI.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::{
    engine::{
        ActorEvidence, Assessment, ClaimStatus, ConflictReason, Evidence, Finding, FindingDetail,
        RequestFit, Route,
    },
    event::{
        Action, AgentRegistration, Boundary, BoundaryKind, ClaimChannel, CoverageReason,
        CoverageState, Event, EventKey, EventPayload, Field, Level, MessageObserved, NoticeKind,
        Source,
    },
    rules,
};

/// How far back the rule editor looks when it shows what a boundary would flag.
pub const PREVIEW_DAYS: i64 = 30;
const PREVIEW_HITS: usize = 5;

/// How loudly an incident asks for attention, from its findings' routes.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Review,
    Urgent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Open,
    /// The user said it was expected.
    Fine,
    /// The user said it was not expected; it stays visible until resolved.
    NotOk,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Email,
    Calendar,
    Report,
    AccountWarning,
    Visibility,
}

/// Why an incident was raised, most specific first. The UI words each one.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    AfterRevocation,
    Boundary,
    ProhibitedAction,
    ApprovalMissing,
    ExtraTarget,
    DifferentCalendarTime,
    TooManyActions,
    DifferentAction,
    OutsideWindow,
    ReportNotObserved,
    ReportContradicted,
    Forwarding,
    AppAccess,
    Otp,
    Purchase,
    SignInNeeded,
    SourceGap,
    SourceDelayed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncidentSummary {
    pub id: String,
    pub severity: Severity,
    pub status: Status,
    pub kind: Kind,
    pub reasons: Vec<Reason>,
    pub agent: Option<String>,
    pub occurred_at: DateTime<Utc>,
    pub flagged_at: DateTime<Utc>,
    /// An email subject or event title, already bounded by the collectors.
    pub subject: Option<String>,
    pub people: Vec<Person>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub address: String,
    pub name: Option<String>,
    /// Not covered by the request the effect was judged against.
    pub unrequested: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncidentDetail {
    #[serde(flatten)]
    pub summary: IncidentSummary,
    /// The findings a decision applies to.
    pub findings: Vec<String>,
    pub request: Option<RequestView>,
    pub reports: Vec<ReportView>,
    pub effect: Option<EffectView>,
    pub actor: ActorView,
    pub request_fit: Option<FitView>,
    pub report_status: Option<ClaimStatus>,
    pub notice: Option<NoticeKind>,
    pub visibility: Option<VisibilityView>,
    /// The user's boundaries the effect crossed, as recorded in the current rules.
    pub boundaries: Vec<Boundary>,
    pub decision: Option<DecisionView>,
    pub history: Vec<HistoryItem>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestView {
    pub at: DateTime<Utc>,
    /// The bounded message snippet the design allows, when one was stored.
    pub message: Option<String>,
    pub actions: Option<Vec<Action>>,
    pub any_action: bool,
    pub targets: Vec<String>,
    pub target_names: Vec<String>,
    pub calendar_start: Option<DateTime<Utc>>,
    pub prohibited_actions: Vec<Action>,
    pub requires_approval: Option<bool>,
    pub approved: bool,
    pub max_actions: Option<u32>,
    pub valid_until: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportView {
    pub at: DateTime<Utc>,
    pub message: Option<String>,
    pub action: Action,
    pub targets: Vec<String>,
    pub target_names: Vec<String>,
    pub channel: ClaimChannel,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectView {
    pub at: DateTime<Utc>,
    pub source: Source,
    pub action: Action,
    pub subject: Option<String>,
    pub calendar_start: Option<DateTime<Utc>>,
    pub people: Vec<Person>,
    pub targets_complete: bool,
    /// The Gmail thread, for opening it in Gmail.
    pub thread: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorView {
    pub basis: ActorBasis,
    pub agents: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorBasis {
    Unknown,
    Reported,
    Runtime,
    Confirmed,
    Conflicting,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FitView {
    pub verdict: FitVerdict,
    pub conflicts: Vec<ConflictReason>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FitVerdict {
    NotAssessed,
    NoRequest,
    NotCaptured,
    Ambiguous,
    Insufficient,
    Consistent,
    Conflicts,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibilityView {
    pub source: Source,
    pub capability: String,
    pub state: CoverageState,
    pub reason: Option<CoverageReason>,
    pub since: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionView {
    pub expected: bool,
    pub at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    pub at: DateTime<Utc>,
    pub kind: HistoryKind,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryKind {
    Requested,
    Happened,
    Reported,
    Flagged,
    Revised,
    MarkedFine,
    MarkedNotOk,
}

/// One finding with its revisions, oldest first, as `(assessed_at, finding)`.
pub struct FindingRecord {
    pub revisions: Vec<(DateTime<Utc>, Finding)>,
}

impl FindingRecord {
    fn current(&self) -> Option<&Finding> {
        self.revisions.last().map(|(_, finding)| finding)
    }

    fn first_review(&self) -> Option<DateTime<Utc>> {
        self.revisions
            .iter()
            .find(|(_, finding)| finding.route.needs_review())
            .map(|(at, _)| *at)
    }
}

/// Message snippets by message key, for quoting requests and reports.
pub trait Messages {
    fn message(&self, key: &EventKey) -> Option<MessageObserved>;
}

/// Groups that were ever routed to review, newest first.
pub fn incidents(
    evidence: &Evidence,
    findings: &BTreeMap<String, Vec<FindingRecord>>,
    agents: &[AgentRegistration],
) -> Vec<IncidentSummary> {
    let mut result: Vec<_> = findings
        .values()
        .filter_map(|group| summary(evidence, group, agents))
        .collect();
    result.sort_by(|left, right| {
        right
            .flagged_at
            .cmp(&left.flagged_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    result
}

pub fn incident(
    evidence: &Evidence,
    group: &[FindingRecord],
    agents: &[AgentRegistration],
    messages: &impl Messages,
) -> Option<IncidentDetail> {
    let summary = summary(evidence, group, agents)?;
    let current: Vec<_> = group.iter().filter_map(FindingRecord::current).collect();
    let effect_assessment = current.iter().find_map(|finding| match &finding.detail {
        FindingDetail::Effect(assessment) => Some(assessment),
        _ => None,
    });
    let basis = effect_assessment.and_then(|assessment| evidence.effect_basis(assessment));
    let quote = |event: &Event| {
        let resource = event.key.resource.split('#').next()?;
        messages
            .message(&EventKey {
                resource: resource.to_owned(),
                ..event.key.clone()
            })
            .map(|message| message.snippet)
            .filter(|snippet| !snippet.is_empty())
    };
    // Quote the message that started the task, not a follow-up such as "Yes": a task's
    // id is the id of the message that created it.
    let request = basis
        .as_ref()
        .and_then(|basis| basis.request)
        .and_then(|event| {
            let original = match &event.payload {
                EventPayload::Request(request) => messages
                    .message(&EventKey {
                        resource: request.task.clone(),
                        ..event.key.clone()
                    })
                    .map(|message| message.snippet)
                    .filter(|snippet| !snippet.is_empty()),
                _ => None,
            };
            request_view(event, original.or_else(|| quote(event)))
        });
    let mut reports: Vec<_> = basis
        .as_ref()
        .map(|basis| basis.reports.clone())
        .unwrap_or_default();
    let mut report_status = None;
    let mut observed = None;
    for finding in &current {
        if let FindingDetail::Claim(check) = &finding.detail {
            report_status = Some(check.status);
            if let Some(event) = evidence.event(&check.claim, "claim")
                && !reports.iter().any(|known| known.key == event.key)
            {
                reports.push(event);
            }
            observed = check.observed_effect.clone();
        }
    }
    reports.sort_by_key(|event| event.occurred_at);
    let effect_event = basis
        .as_ref()
        .map(|basis| basis.effect)
        .or_else(|| observed.and_then(|key| evidence.effect_event(&key)));
    let extra = basis
        .as_ref()
        .map(|basis| basis.extra_targets.clone())
        .unwrap_or_default();
    let effect = effect_event.and_then(|event| effect_view(event, &extra));
    let notice = current.iter().find_map(|finding| match &finding.detail {
        FindingDetail::Notice(notice) => Some(notice.kind.clone()),
        _ => None,
    });
    let visibility = current.iter().find_map(|finding| match &finding.detail {
        FindingDetail::Coverage(coverage) => Some(VisibilityView {
            source: coverage.evidence.source.clone(),
            capability: coverage.evidence.capability.clone(),
            state: coverage.evidence.state.clone(),
            reason: coverage.evidence.reason.clone(),
            since: coverage.evidence.observed_at,
        }),
        _ => None,
    });
    let decision = decision(evidence, &current);
    let mut history = Vec::new();
    if let Some(request) = &request {
        history.push(HistoryItem {
            at: request.at,
            kind: HistoryKind::Requested,
        });
    }
    if let Some(effect) = &effect {
        history.push(HistoryItem {
            at: effect.at,
            kind: HistoryKind::Happened,
        });
    }
    for report in &reports {
        history.push(HistoryItem {
            at: report.occurred_at,
            kind: HistoryKind::Reported,
        });
    }
    history.push(HistoryItem {
        at: summary.flagged_at,
        kind: HistoryKind::Flagged,
    });
    for record in group {
        let mut previous: Option<&Finding> = None;
        for (at, finding) in &record.revisions {
            if let Some(before) = previous
                && *at > summary.flagged_at
                && before.route.needs_review()
                && finding.route.needs_review()
                && before.detail != finding.detail
            {
                history.push(HistoryItem {
                    at: *at,
                    kind: HistoryKind::Revised,
                });
            }
            previous = Some(finding);
        }
    }
    if let Some(decision) = &decision {
        history.push(HistoryItem {
            at: decision.at,
            kind: if decision.expected {
                HistoryKind::MarkedFine
            } else {
                HistoryKind::MarkedNotOk
            },
        });
    }
    history.sort_by(|left, right| left.at.cmp(&right.at).then(left.kind.cmp(&right.kind)));
    history.dedup();
    Some(IncidentDetail {
        findings: current.iter().map(|finding| finding.id.clone()).collect(),
        request,
        reports: reports
            .into_iter()
            .filter_map(|event| report_view(event, quote(event)))
            .collect(),
        effect,
        actor: actor_view(
            effect_assessment.map(|assessment| &assessment.actor),
            agents,
        ),
        request_fit: effect_assessment.map(|assessment| fit_view(&assessment.request_fit)),
        report_status,
        notice,
        visibility,
        boundaries: effect_assessment
            .map(|assessment| {
                evidence
                    .rules()
                    .map(|rules| {
                        rules
                            .boundaries
                            .iter()
                            .filter(|boundary| assessment.boundaries.contains(&boundary.id))
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .unwrap_or_default(),
        decision,
        history,
        summary,
    })
}

fn summary(
    evidence: &Evidence,
    group: &[FindingRecord],
    agents: &[AgentRegistration],
) -> Option<IncidentSummary> {
    let flagged_at = group.iter().filter_map(FindingRecord::first_review).min()?;
    let current: Vec<_> = group.iter().filter_map(FindingRecord::current).collect();
    let first = current.first()?;
    let severity = current
        .iter()
        .map(|finding| match finding.route {
            Route::HighReview => Severity::Urgent,
            _ => Severity::Review,
        })
        .max()
        .unwrap_or(Severity::Review);
    let open = current.iter().any(|finding| finding.route.needs_review());
    let status = match decision(evidence, &current) {
        Some(DecisionView {
            expected: false, ..
        }) => Status::NotOk,
        _ if open => Status::Open,
        _ => Status::Fine,
    };
    let mut reasons: Vec<_> = current
        .iter()
        .flat_map(|finding| reasons(finding))
        .collect();
    reasons.sort();
    reasons.dedup();
    let effect = current.iter().find_map(|finding| match &finding.detail {
        FindingDetail::Effect(assessment) => Some(assessment),
        _ => None,
    });
    let effect_event = effect.and_then(|assessment| evidence.effect_event(&assessment.effect));
    let extra = effect
        .and_then(|assessment| evidence.effect_basis(assessment))
        .map(|basis| basis.extra_targets)
        .unwrap_or_default();
    let view = effect_event.and_then(|event| effect_view(event, &extra));
    let kind = match &first.detail {
        _ if view
            .as_ref()
            .is_some_and(|view| view.source == Source::Calendar) =>
        {
            Kind::Calendar
        }
        _ if view.is_some() => Kind::Email,
        FindingDetail::Claim(_) => Kind::Report,
        FindingDetail::Notice(_) => Kind::AccountWarning,
        FindingDetail::Coverage(_) => Kind::Visibility,
        FindingDetail::Effect(_) => Kind::Email,
    };
    let occurred_at = view
        .as_ref()
        .map(|view| view.at)
        .or_else(|| subject_time(evidence, first))
        .unwrap_or(flagged_at);
    let agent = effect
        .and_then(|assessment| actor_agents(&assessment.actor).into_iter().next())
        .or_else(|| {
            current.iter().find_map(|finding| match &finding.detail {
                FindingDetail::Claim(check) => {
                    evidence
                        .event(&check.claim, "claim")
                        .and_then(|event| match &event.payload {
                            EventPayload::Claim(claim) => Some(claim.agent.clone()),
                            _ => None,
                        })
                }
                _ => None,
            })
        })
        .map(|id| agent_name(&id, agents));
    Some(IncidentSummary {
        id: first.group.clone(),
        severity,
        status,
        kind,
        reasons,
        agent,
        occurred_at,
        flagged_at,
        subject: view.as_ref().and_then(|view| view.subject.clone()),
        people: view.map(|view| view.people).unwrap_or_default(),
    })
}

fn decision(evidence: &Evidence, current: &[&Finding]) -> Option<DecisionView> {
    current
        .iter()
        .filter_map(|finding| {
            let account = finding_account(finding)?;
            evidence.decision(account, &finding.id)
        })
        .max_by_key(|(_, at)| *at)
        .and_then(|(expected, at)| expected.map(|expected| DecisionView { expected, at }))
}

/// The account a finding's subject belongs to; decisions are recorded under it.
pub fn finding_account(finding: &Finding) -> Option<&str> {
    Some(match &finding.detail {
        FindingDetail::Effect(assessment) => &assessment.effect.account,
        FindingDetail::Claim(check) => &check.claim.account,
        FindingDetail::Notice(notice) => &notice.notice.account,
        FindingDetail::Coverage(coverage) => &coverage.account,
    })
}

fn subject_time(evidence: &Evidence, finding: &Finding) -> Option<DateTime<Utc>> {
    match &finding.detail {
        FindingDetail::Claim(check) => evidence
            .event(&check.claim, "claim")
            .map(|event| event.occurred_at),
        FindingDetail::Notice(notice) => evidence
            .event(&notice.notice, "notice")
            .map(|event| event.occurred_at),
        FindingDetail::Coverage(coverage) => Some(coverage.evidence.observed_at),
        FindingDetail::Effect(_) => None,
    }
}

fn reasons(finding: &Finding) -> Vec<Reason> {
    if !finding.route.needs_review() {
        return Vec::new();
    }
    match &finding.detail {
        FindingDetail::Effect(assessment) => effect_reasons(assessment),
        FindingDetail::Claim(check) => match check.status {
            ClaimStatus::NotObservedInHealthySource => vec![Reason::ReportNotObserved],
            ClaimStatus::ContradictedByObservedEffect => vec![Reason::ReportContradicted],
            _ => Vec::new(),
        },
        FindingDetail::Notice(notice) => vec![match notice.kind {
            NoticeKind::Forwarding => Reason::Forwarding,
            NoticeKind::AppAccess => Reason::AppAccess,
            NoticeKind::Otp => Reason::Otp,
            NoticeKind::Purchase => Reason::Purchase,
        }],
        FindingDetail::Coverage(coverage) => {
            vec![
                match (&coverage.evidence.state, &coverage.evidence.reason) {
                    (_, Some(CoverageReason::AuthUnavailable)) => Reason::SignInNeeded,
                    (CoverageState::Delayed, _) => Reason::SourceDelayed,
                    _ => Reason::SourceGap,
                },
            ]
        }
    }
}

fn effect_reasons(assessment: &Assessment) -> Vec<Reason> {
    let mut reasons = Vec::new();
    if assessment.after_revocation {
        reasons.push(Reason::AfterRevocation);
    }
    if !assessment.policy_ids.is_empty() {
        reasons.push(Reason::Boundary);
    }
    if let RequestFit::Conflicts(conflicts) = &assessment.request_fit {
        reasons.extend(conflicts.iter().map(|conflict| match conflict {
            ConflictReason::ProhibitedAction => Reason::ProhibitedAction,
            ConflictReason::ApprovalMissing => Reason::ApprovalMissing,
            ConflictReason::ExtraTarget => Reason::ExtraTarget,
            ConflictReason::DifferentCalendarTime => Reason::DifferentCalendarTime,
            ConflictReason::TooManyActions => Reason::TooManyActions,
            ConflictReason::DifferentAction => Reason::DifferentAction,
            ConflictReason::OutsideWindow => Reason::OutsideWindow,
        }));
    }
    reasons
}

fn request_view(event: &Event, message: Option<String>) -> Option<RequestView> {
    let EventPayload::Request(request) = &event.payload else {
        return None;
    };
    Some(RequestView {
        at: event.occurred_at,
        message,
        actions: match &request.actions {
            Field::Known(actions) => Some(actions.clone()),
            _ => None,
        },
        any_action: request.actions == Field::ExplicitAny,
        targets: match &request.targets {
            Field::Known(targets) => targets.clone(),
            _ => Vec::new(),
        },
        target_names: request.target_names.clone(),
        calendar_start: match request.calendar_start {
            Field::Known(start) => Some(start),
            _ => None,
        },
        prohibited_actions: request.prohibited_actions.clone(),
        requires_approval: match request.requires_approval {
            Field::Known(value) => Some(value),
            _ => None,
        },
        approved: request.approved,
        max_actions: match request.max_actions {
            Field::Known(value) => Some(value),
            _ => None,
        },
        valid_until: request.valid_until,
    })
}

fn report_view(event: &Event, message: Option<String>) -> Option<ReportView> {
    let EventPayload::Claim(claim) = &event.payload else {
        return None;
    };
    Some(ReportView {
        at: event.occurred_at,
        message,
        action: claim.action.clone(),
        targets: claim.targets.clone(),
        target_names: claim.target_names.clone(),
        channel: claim.channel.clone(),
    })
}

fn effect_view(event: &Event, extra: &[String]) -> Option<EffectView> {
    let EventPayload::Effect(effect) = &event.payload else {
        return None;
    };
    Some(EffectView {
        at: event.occurred_at,
        source: event.key.source.clone(),
        action: effect.action.clone(),
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
        calendar_start: effect.calendar_start,
        people: effect
            .targets
            .iter()
            .map(|address| Person {
                address: address.clone(),
                name: effect.target_names.get(address).cloned(),
                unrequested: extra.contains(address),
            })
            .collect(),
        targets_complete: effect.targets_complete,
        thread: effect.email.as_ref().map(|email| email.thread_id.clone()),
    })
}

fn actor_agents(actor: &ActorEvidence) -> Vec<String> {
    match actor {
        ActorEvidence::Unknown => Vec::new(),
        ActorEvidence::Reported(agent)
        | ActorEvidence::RuntimeLinked(agent)
        | ActorEvidence::UserConfirmed(agent) => vec![agent.clone()],
        ActorEvidence::Conflicting(agents) => agents.clone(),
    }
}

fn actor_view(actor: Option<&ActorEvidence>, agents: &[AgentRegistration]) -> ActorView {
    let basis = match actor {
        None | Some(ActorEvidence::Unknown) => ActorBasis::Unknown,
        Some(ActorEvidence::Reported(_)) => ActorBasis::Reported,
        Some(ActorEvidence::RuntimeLinked(_)) => ActorBasis::Runtime,
        Some(ActorEvidence::UserConfirmed(_)) => ActorBasis::Confirmed,
        Some(ActorEvidence::Conflicting(_)) => ActorBasis::Conflicting,
    };
    ActorView {
        basis,
        agents: actor
            .map(actor_agents)
            .unwrap_or_default()
            .iter()
            .map(|id| agent_name(id, agents))
            .collect(),
    }
}

fn fit_view(fit: &RequestFit) -> FitView {
    let (verdict, conflicts) = match fit {
        RequestFit::NotAssessed => (FitVerdict::NotAssessed, Vec::new()),
        RequestFit::NoObservedRequest => (FitVerdict::NoRequest, Vec::new()),
        RequestFit::RequestNotCaptured => (FitVerdict::NotCaptured, Vec::new()),
        RequestFit::AmbiguousTask => (FitVerdict::Ambiguous, Vec::new()),
        RequestFit::InsufficientEvidence => (FitVerdict::Insufficient, Vec::new()),
        RequestFit::ConsistentWithVisibleRequest => (FitVerdict::Consistent, Vec::new()),
        RequestFit::Conflicts(conflicts) => (FitVerdict::Conflicts, conflicts.clone()),
    };
    FitView { verdict, conflicts }
}

fn agent_name(id: &str, agents: &[AgentRegistration]) -> String {
    agents
        .iter()
        .find(|agent| agent.id == id)
        .map_or_else(|| id.to_owned(), |agent| agent.name.clone())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RulesView {
    pub levels: Vec<RuleLevelView>,
    pub boundaries: Vec<BoundaryView>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleLevelView {
    pub id: &'static str,
    pub level: Level,
    pub default: Level,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundaryView {
    #[serde(flatten)]
    pub boundary: Boundary,
    /// Effects in the last `PREVIEW_DAYS` it would have flagged, had it been in force.
    pub recent: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewView {
    pub count: usize,
    pub hits: Vec<PreviewHit>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewHit {
    pub at: DateTime<Utc>,
    pub action: Action,
    pub subject: Option<String>,
    pub people: Vec<Person>,
    pub agent: Option<String>,
}

pub fn rules_view(evidence: &Evidence, now: DateTime<Utc>) -> RulesView {
    let current = evidence.rules();
    let from = now - chrono::Duration::days(PREVIEW_DAYS);
    RulesView {
        levels: rules::BUILT_IN
            .iter()
            .map(|(id, default)| RuleLevelView {
                id,
                level: rules::level(current, id),
                default: *default,
            })
            .collect(),
        boundaries: current
            .map(|rules| {
                rules
                    .boundaries
                    .iter()
                    .map(|boundary| BoundaryView {
                        recent: evidence.boundary_hits(&boundary.kind, from, now).len(),
                        boundary: boundary.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

pub fn preview(
    evidence: &Evidence,
    kind: &BoundaryKind,
    now: DateTime<Utc>,
    agents: &[AgentRegistration],
) -> PreviewView {
    let hits = evidence.boundary_hits(kind, now - chrono::Duration::days(PREVIEW_DAYS), now);
    PreviewView {
        count: hits.len(),
        hits: hits
            .into_iter()
            .take(PREVIEW_HITS)
            .filter_map(|event| {
                let view = effect_view(event, &[])?;
                let agent = actor_agents(&evidence.effect_actor(event))
                    .into_iter()
                    .next()
                    .map(|id| agent_name(&id, agents));
                Some(PreviewHit {
                    at: view.at,
                    action: view.action,
                    subject: view.subject,
                    people: view.people,
                    agent,
                })
            })
            .collect(),
    }
}
