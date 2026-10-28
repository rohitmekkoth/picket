//! Deterministic, conservative assessments of independently observed effects.
//! A report from an agent is evidence of a report, not proof of authorship.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::event::{
    AccountPolicy, Action, ClaimChannel, Coverage, CoverageReason, CoverageState, Effect, Event,
    EventKey, EventPayload, Feedback, Field, NoticeKind, Origin, Request, SnapshotValue, Source,
    ToolOutcome, ToolResult,
};

const MAX_CANDIDATES: usize = 128;
const CLAIM_WINDOW: Duration = Duration::minutes(10);
const GRACE: Duration = Duration::minutes(5);
/// Coverage is indexed per account, source and capability, ordered by observation time.
type CoverageKey = (String, &'static str, String);
type CoverageHistory = Vec<(DateTime<Utc>, i64)>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ActorEvidence {
    Unknown,
    Reported(String),
    RuntimeLinked(String),
    UserConfirmed(String),
    Conflicting(Vec<String>),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RequestFit {
    NotAssessed,
    /// Request capture for this agent was healthy and found no request.
    NoObservedRequest,
    /// The agent's request channel was not captured or not yet extracted, so the
    /// absence of a request means nothing.
    RequestNotCaptured,
    AmbiguousTask,
    InsufficientEvidence,
    ConsistentWithVisibleRequest,
    Conflicts(Vec<ConflictReason>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ConflictReason {
    ProhibitedAction,
    DifferentAction,
    ExtraTarget,
    ApprovalMissing,
    DifferentCalendarTime,
    TooManyActions,
    OutsideWindow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Route {
    Timeline,
    MediumReview,
    HighReview,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CoverageEvidence {
    pub source: Source,
    pub capability: String,
    pub state: CoverageState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<CoverageReason>,
    pub observed_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ClaimStatus {
    Pending,
    Matched,
    ContradictedByObservedEffect,
    NotObservedInHealthySource,
    CoverageIncomplete,
    /// The claimed email was not, or not known to be, sent from the observed account,
    /// for example from the agent's own mailbox. Its absence proves nothing.
    OutsideObservedScope,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClaimCheck {
    pub claim: EventKey,
    pub status: ClaimStatus,
    pub observed_effect: Option<EventKey>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NoticeAssessment {
    pub notice: EventKey,
    pub kind: NoticeKind,
    pub route: Route,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CoverageAssessment {
    pub account: String,
    pub evidence: CoverageEvidence,
    pub route: Route,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum FindingDetail {
    Effect(Assessment),
    Claim(ClaimCheck),
    Notice(NoticeAssessment),
    Coverage(CoverageAssessment),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    #[serde(default)]
    pub group: String,
    pub route: Route,
    pub detail: FindingDetail,
}

/// Engine-relevant visible events, indexed so an assessment reads only its candidates.
/// Each index key is a necessary condition of the predicate that consumes it, so a
/// lookup returns a superset of the matches a full scan would find. Candidates are
/// visited in log order, which keeps tie-breaking identical to a full scan.
#[derive(Clone, Debug, Default)]
pub struct Evidence {
    events: BTreeMap<i64, Event>,
    effects: BTreeSet<(String, DateTime<Utc>, i64)>,
    claims: BTreeSet<(String, DateTime<Utc>, i64)>,
    resources: BTreeMap<(String, String), Vec<i64>>,
    requests: BTreeMap<(String, String), Vec<i64>>,
    tasks: BTreeMap<(String, String, String), Vec<i64>>,
    feedback: BTreeMap<String, Vec<i64>>,
    policies: BTreeMap<String, Vec<i64>>,
    coverage: BTreeMap<CoverageKey, CoverageHistory>,
    notices: Vec<i64>,
    effect_findings: BTreeMap<String, i64>,
    deadlines: BTreeSet<(DateTime<Utc>, i64)>,
}

impl Evidence {
    /// Indexes the events visible at `now`, using slice position as log order.
    pub fn visible(events: &[Event], now: DateTime<Utc>) -> Self {
        let mut evidence = Self::default();
        for (index, event) in events.iter().enumerate() {
            if event.ingested_at <= now {
                evidence.insert(index as i64, event.clone());
            }
        }
        evidence
    }

    /// Adds one visible event. Events the engine never reads are not retained.
    pub fn insert(&mut self, seq: i64, event: Event) {
        if self.events.contains_key(&seq) {
            return;
        }
        let account = event.key.account.clone();
        match &event.payload {
            EventPayload::Effect(effect)
                if matches!(effect.origin, Origin::Live | Origin::CatchUp) =>
            {
                self.effects
                    .insert((account.clone(), event.occurred_at, seq));
                self.effect_findings
                    .insert(effect_finding_id(&event.key), seq);
                self.deadlines.insert((wait_from(&event) + GRACE, seq));
                push_sorted(
                    self.resources
                        .entry((account, event.key.resource.clone()))
                        .or_default(),
                    seq,
                );
            }
            EventPayload::Claim(claim) if event.key.source == Source::Messages => {
                self.claims
                    .insert((account.clone(), event.occurred_at, seq));
                self.deadlines.insert((wait_from(&event) + GRACE, seq));
                if let Some(resource) = &claim.provider_resource {
                    push_sorted(
                        self.resources
                            .entry((account, resource.clone()))
                            .or_default(),
                        seq,
                    );
                }
            }
            EventPayload::ToolResult(tool) if event.key.source == Source::OpenClaw => {
                if let Some(task) = &tool.task {
                    push_sorted(
                        self.tasks
                            .entry((account.clone(), tool.agent.clone(), task.clone()))
                            .or_default(),
                        seq,
                    );
                }
                if let Some(resource) = &tool.provider_resource {
                    push_sorted(
                        self.resources
                            .entry((account, resource.clone()))
                            .or_default(),
                        seq,
                    );
                }
            }
            EventPayload::Request(request) if event.key.source == Source::Messages => {
                push_sorted(
                    self.requests
                        .entry((account, request.agent.clone()))
                        .or_default(),
                    seq,
                );
            }
            EventPayload::Notice { origin, .. }
                if event.key.source == Source::Gmail
                    && matches!(origin, Origin::Live | Origin::CatchUp) =>
            {
                push_sorted(&mut self.notices, seq);
            }
            _ => {
                if let Some((value, _)) = feedback(&event) {
                    push_sorted(self.feedback.entry(value.finding.clone()).or_default(), seq);
                } else if policy(&event).is_some() {
                    push_sorted(self.policies.entry(account).or_default(), seq);
                } else if let Some((source, value, observed_at)) = coverage_value(&event) {
                    let list = self
                        .coverage
                        .entry((account, source.as_str(), value.capability.clone()))
                        .or_default();
                    let position = list.partition_point(|item| *item < (observed_at, seq));
                    list.insert(position, (observed_at, seq));
                } else {
                    return;
                }
            }
        }
        self.events.insert(seq, event);
    }

    fn get(&self, seq: i64) -> Option<&Event> {
        self.events.get(&seq)
    }

    fn list<'a>(&'a self, seqs: Option<&'a Vec<i64>>) -> impl Iterator<Item = &'a Event> + 'a {
        seqs.into_iter()
            .flatten()
            .filter_map(|seq| self.events.get(seq))
    }

    fn by_resource(&self, account: &str, resource: &str) -> Vec<(i64, &Event)> {
        self.resources
            .get(&(account.to_owned(), resource.to_owned()))
            .into_iter()
            .flatten()
            .filter_map(|seq| self.get(*seq).map(|event| (*seq, event)))
            .collect()
    }

    /// Effects near a time. The extra second covers predicates that compare whole seconds.
    fn effects_near(&self, account: &str, at: DateTime<Utc>) -> Vec<(i64, &Event)> {
        self.near(&self.effects, account, at)
    }

    fn claims_near(&self, account: &str, at: DateTime<Utc>) -> Vec<(i64, &Event)> {
        self.near(&self.claims, account, at)
    }

    fn near(
        &self,
        index: &BTreeSet<(String, DateTime<Utc>, i64)>,
        account: &str,
        at: DateTime<Utc>,
    ) -> Vec<(i64, &Event)> {
        let window = CLAIM_WINDOW + Duration::seconds(1);
        time_range(index, account, at - window, at + window)
            .filter_map(|seq| self.get(seq).map(|event| (seq, event)))
            .collect()
    }

    pub fn findings(&self, now: DateTime<Utc>) -> Vec<Finding> {
        let mut result: Vec<_> = self.assess(now).into_iter().map(effect_finding).collect();
        result.extend(self.assess_claims(now).into_iter().map(claim_finding));
        result.extend(self.assess_notices().into_iter().map(notice_finding));
        let accounts: BTreeSet<_> = self.coverage.keys().map(|key| key.0.as_str()).collect();
        for account in accounts {
            result.extend(self.coverage_findings(account));
        }
        result.sort_by(|left, right| left.id.cmp(&right.id));
        result
    }

    pub fn assess(&self, now: DateTime<Utc>) -> Vec<Assessment> {
        let mut effects: Vec<_> = self
            .effects
            .iter()
            .filter_map(|(_, _, seq)| self.get(*seq).map(|event| (*seq, event)))
            .collect();
        effects.sort_by(|(a_seq, a), (b_seq, b)| {
            a.occurred_at
                .cmp(&b.occurred_at)
                .then_with(|| a.key.resource.cmp(&b.key.resource))
                .then_with(|| a_seq.cmp(b_seq))
        });
        effects
            .into_iter()
            .filter_map(|(_, event)| self.assess_effect(event, now))
            .collect()
    }

    fn assess_effect(&self, event: &Event, now: DateTime<Utc>) -> Option<Assessment> {
        let EventPayload::Effect(effect) = &event.payload else {
            return None;
        };
        let finding = effect_finding_id(&event.key);
        let actor = self.actor_for(event, effect, &finding);
        let policy_hits = self.policy_violations(event, effect);
        let policy_ids = policy_hits.iter().map(|(id, _)| id.clone()).collect();
        let request_fit = self.request_fit(event, effect, &actor);
        let pending = now < wait_from(event) + GRACE;
        let expected_feedback = self
            .list(self.feedback.get(&finding))
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
        Some(Assessment {
            finding,
            effect: event.key.clone(),
            actor,
            request_fit,
            policy_ids,
            pending,
            route,
            expected_feedback,
        })
    }

    /// A metadata notice prompts review; it does not prove a setting or payment.
    pub fn assess_notices(&self) -> Vec<NoticeAssessment> {
        self.list(Some(&self.notices))
            .filter_map(notice_assessment)
            .collect()
    }

    /// Claim checks concern observed consistency, not agent honesty.
    pub fn assess_claims(&self, now: DateTime<Utc>) -> Vec<ClaimCheck> {
        let mut claims: Vec<_> = self.claims.iter().map(|(_, _, seq)| *seq).collect();
        claims.sort_unstable();
        claims
            .into_iter()
            .filter_map(|seq| {
                self.get(seq)
                    .and_then(|report| self.claim_check(report, now))
            })
            .collect()
    }

    fn claim_check(&self, report: &Event, now: DateTime<Utc>) -> Option<ClaimCheck> {
        let EventPayload::Claim(claim) = &report.payload else {
            return None;
        };
        let source = claim_source(claim);
        let account = &report.key.account;
        let mut candidates: BTreeMap<i64, &Event> = self
            .effects_near(account, report.occurred_at)
            .into_iter()
            .collect();
        let by_resource = claim
            .provider_resource
            .as_ref()
            .map(|resource| self.by_resource(account, resource))
            .unwrap_or_default();
        candidates.extend(by_resource.iter().copied());
        let matched = candidates.values().find(|event| {
            effect_on(event, &source)
                .is_some_and(|effect| self.claim_matches(report, claim, event, effect))
        });
        let contradicted = claim.provider_resource.as_ref().and_then(|resource| {
            by_resource.iter().map(|(_, event)| event).find(|event| {
                effect_on(event, &source).is_some() && event.key.resource == *resource
            })
        });
        let (status, observed_effect) = if let Some(event) = matched {
            (ClaimStatus::Matched, Some(event.key.clone()))
        } else if let Some(event) = contradicted {
            (
                ClaimStatus::ContradictedByObservedEffect,
                Some(event.key.clone()),
            )
        } else if claim.action == Action::EmailSend && claim.channel != ClaimChannel::UserAccount {
            (ClaimStatus::OutsideObservedScope, None)
        } else if now < wait_from(report) + GRACE {
            (ClaimStatus::Pending, None)
        } else if self.observed_without_gap(report, &source) {
            (ClaimStatus::NotObservedInHealthySource, None)
        } else {
            (ClaimStatus::CoverageIncomplete, None)
        };
        Some(ClaimCheck {
            claim: report.key.clone(),
            status,
            observed_effect,
        })
    }

    /// True when the source recorded no lossy coverage from the claim until its first
    /// healthy confirmation after the deadline. Later coverage cannot change the answer.
    fn observed_without_gap(&self, report: &Event, source: &Source) -> bool {
        self.covered(
            &report.key.account,
            source,
            "effects",
            report.occurred_at,
            wait_from(report) + GRACE,
        )
    }

    /// Whether request capture for `agent` covered `at`: no lossy coverage from the
    /// last entry before `at` until a healthy confirmation at or after it.
    fn request_captured(&self, account: &str, agent: &str, at: DateTime<Utc>) -> bool {
        self.covered(
            account,
            &Source::Messages,
            &request_capability(agent),
            at,
            at,
        )
    }

    /// Walks one capability's coverage from the entry in force at `from` until the
    /// first healthy entry at or after `deadline`. A delay is repaired by catch-up; a
    /// gap or disabled source is not.
    fn covered(
        &self,
        account: &str,
        source: &Source,
        capability: &str,
        from: DateTime<Utc>,
        deadline: DateTime<Utc>,
    ) -> bool {
        let Some(history) =
            self.coverage
                .get(&(account.to_owned(), source.as_str(), capability.to_owned()))
        else {
            return false;
        };
        let start = history
            .partition_point(|(observed_at, _)| *observed_at <= from)
            .saturating_sub(1);
        for (observed_at, seq) in &history[start..] {
            let Some((_, coverage, _)) = self.get(*seq).and_then(coverage_value) else {
                continue;
            };
            match coverage.state {
                CoverageState::Healthy if *observed_at >= deadline => return true,
                CoverageState::Healthy | CoverageState::Delayed => {}
                CoverageState::Gap | CoverageState::Disabled => return false,
            }
        }
        false
    }

    fn coverage_findings(&self, account: &str) -> Vec<Finding> {
        self.coverage_for(account)
            .into_iter()
            .map(|evidence| {
                let route = coverage_route(&evidence);
                Finding {
                    id: scoped_id(
                        "coverage",
                        &[account, evidence.source.as_str(), &evidence.capability],
                    ),
                    group: scoped_id(
                        "incident",
                        &[account, evidence.source.as_str(), &evidence.capability],
                    ),
                    route,
                    detail: FindingDetail::Coverage(CoverageAssessment {
                        account: account.to_owned(),
                        evidence,
                        route,
                    }),
                }
            })
            .collect()
    }

    fn coverage_keys<'a>(
        &'a self,
        account: &'a str,
    ) -> impl Iterator<Item = (&'a CoverageKey, &'a CoverageHistory)> + 'a {
        self.coverage
            .range((account.to_owned(), "", String::new())..)
            .take_while(move |(key, _)| key.0 == account)
    }

    fn coverage_for(&self, account: &str) -> Vec<CoverageEvidence> {
        self.coverage_keys(account)
            .filter_map(|(_, list)| list.last())
            .filter_map(|(_, seq)| self.get(*seq))
            .filter_map(coverage_value)
            .map(|(source, value, observed_at)| CoverageEvidence {
                source,
                capability: value.capability.clone(),
                state: value.state.clone(),
                reason: value.reason.clone(),
                observed_at,
            })
            .collect()
    }

    /// The most recently observed coverage for one source capability.
    pub fn coverage_latest(
        &self,
        account: &str,
        source: &Source,
        capability: &str,
    ) -> Option<(&Coverage, DateTime<Utc>)> {
        self.coverage
            .get(&(account.to_owned(), source.as_str(), capability.to_owned()))
            .and_then(|list| list.last())
            .and_then(|(_, seq)| self.get(*seq))
            .and_then(coverage_value)
            .map(|(_, value, observed_at)| (value, observed_at))
    }

    fn actor_for(&self, event: &Event, effect: &Effect, finding: &str) -> ActorEvidence {
        if effect.external_change {
            return ActorEvidence::Unknown;
        }
        let account = &event.key.account;
        let mut runtime = BTreeSet::new();
        let mut reports = BTreeSet::new();
        let mut candidates: BTreeMap<i64, &Event> = self
            .claims_near(account, event.occurred_at)
            .into_iter()
            .collect();
        candidates.extend(self.by_resource(account, &event.key.resource));
        for candidate in candidates.values() {
            match &candidate.payload {
                EventPayload::ToolResult(tool) if tool_matches(tool, event, effect) => {
                    runtime.insert(tool.agent.clone());
                }
                EventPayload::Claim(claim)
                    if self.claim_matches(candidate, claim, event, effect) =>
                {
                    reports.insert(claim.agent.clone());
                }
                _ => {}
            }
        }
        let mut confirmed = None;
        for candidate in self.list(self.feedback.get(finding)) {
            if candidate.key.account == *account
                && let Some((feedback, _)) = feedback(candidate)
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
        &self,
        report: &Event,
        claim: &crate::event::Claim,
        event: &Event,
        effect: &Effect,
    ) -> bool {
        if claim.action != effect.action
            || !effect.targets_complete
            || !claim_targets_match(claim, effect)
        {
            return false;
        }
        if let Some(resource) = &claim.provider_resource {
            return resource == &event.key.resource;
        }
        if (report.occurred_at - event.occurred_at).num_seconds().abs() > CLAIM_WINDOW.num_seconds()
        {
            return false;
        }
        // A report without a provider ID cannot identify one of two similar effects.
        self.effects_near(&event.key.account, report.occurred_at)
            .into_iter()
            .filter(|(_, candidate)| {
                (report.occurred_at - candidate.occurred_at)
                    .num_seconds()
                    .abs()
                    <= CLAIM_WINDOW.num_seconds()
                    && matches!(&candidate.payload, EventPayload::Effect(other)
                        if other.action == claim.action
                            && other.targets_complete && claim_targets_match(claim, other))
            })
            .take(2)
            .count()
            == 1
    }

    fn policy_violations(&self, event: &Event, effect: &Effect) -> Vec<(String, bool)> {
        let mut latest = BTreeMap::new();
        for candidate in self.list(self.policies.get(&event.key.account)) {
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

    fn request_fit(&self, event: &Event, effect: &Effect, actor: &ActorEvidence) -> RequestFit {
        let agent = match actor {
            ActorEvidence::Reported(agent) | ActorEvidence::RuntimeLinked(agent) => agent,
            _ => return RequestFit::NotAssessed,
        };
        let account = &event.key.account;
        // A task link needs a reason: a runtime result for this resource, or a claim that
        // independently matches this effect (by provider id, or uniquely in its window).
        let mut linked: BTreeMap<i64, &Event> = self
            .by_resource(account, &event.key.resource)
            .into_iter()
            .collect();
        linked.extend(self.claims_near(account, event.occurred_at));
        let task_hints: BTreeSet<_> = linked
            .values()
            .filter_map(|candidate| match (&actor, &candidate.payload) {
                (ActorEvidence::RuntimeLinked(_), EventPayload::ToolResult(tool))
                    if tool.agent == *agent && tool_matches(tool, event, effect) =>
                {
                    tool.task.as_ref()
                }
                (ActorEvidence::Reported(_), EventPayload::Claim(claim))
                    if claim.agent == *agent
                        && self.claim_matches(candidate, claim, event, effect) =>
                {
                    claim.task.as_ref()
                }
                _ => None,
            })
            .collect();
        if task_hints.len() > 1 {
            return RequestFit::AmbiguousTask;
        }
        let task_hint = task_hints.into_iter().next();
        let candidates: Vec<_> = self
            .list(self.requests.get(&(account.clone(), agent.clone())))
            .filter_map(|candidate| {
                if candidate.occurred_at > event.occurred_at {
                    return None;
                }
                match &candidate.payload {
                    EventPayload::Request(request)
                        if task_hint.is_none_or(|task| request.task == *task) =>
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
        if task_hint.is_none() && latest.len() == 1 {
            return RequestFit::InsufficientEvidence;
        }
        match latest.len() {
            0 if self.request_captured(account, agent, event.occurred_at) => {
                RequestFit::NoObservedRequest
            }
            0 => RequestFit::RequestNotCaptured,
            1 => match latest.into_values().next() {
                Some((_, request)) => {
                    let count = task_hint.and_then(|task| {
                        matches!(actor, ActorEvidence::RuntimeLinked(_))
                            .then(|| self.linked_action_count(event, agent, task))
                            .flatten()
                    });
                    compare_request(request, effect, count, event.occurred_at)
                }
                None => RequestFit::NoObservedRequest,
            },
            _ => RequestFit::AmbiguousTask,
        }
    }

    fn linked_action_count(&self, current: &Event, agent: &str, task: &str) -> Option<u32> {
        let account = &current.key.account;
        let first_request = self
            .list(self.requests.get(&(account.clone(), agent.to_owned())))
            .filter_map(|event| match &event.payload {
                EventPayload::Request(request)
                    if request.task == task && event.occurred_at <= current.occurred_at =>
                {
                    Some(event.occurred_at)
                }
                _ => None,
            })
            .min()?;
        let mut effects = BTreeSet::new();
        let tools = self.list(self.tasks.get(&(
            account.clone(),
            agent.to_owned(),
            task.to_owned(),
        )));
        for tool in tools {
            let EventPayload::ToolResult(tool) = &tool.payload else {
                continue;
            };
            let Some(resource) = &tool.provider_resource else {
                continue;
            };
            for (_, event) in self.by_resource(account, resource) {
                let EventPayload::Effect(effect) = &event.payload else {
                    continue;
                };
                if event.occurred_at < first_request
                    || event.occurred_at > current.occurred_at
                    || effect.external_change
                    || !tool_matches(tool, event, effect)
                {
                    continue;
                }
                effects.insert(effect_finding_id(&event.key));
                if effects.len() > MAX_CANDIDATES {
                    return None;
                }
            }
        }
        u32::try_from(effects.len()).ok()
    }
}

/// The input event an assessment is about.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Subject {
    Effect(i64),
    Claim(i64),
    Notice(i64),
    Coverage(String),
}

impl Evidence {
    /// Subjects whose assessment can change because `seq` became visible. Each rule
    /// mirrors a lookup in the assessment code; retention housekeeping changes nothing.
    fn affected_by(&self, seq: i64) -> BTreeSet<Subject> {
        let mut subjects = BTreeSet::new();
        let Some(event) = self.get(seq) else {
            return subjects;
        };
        let account = &event.key.account;
        let effects = |subjects: &mut BTreeSet<Subject>, events: Vec<(i64, &Event)>| {
            subjects.extend(
                events
                    .into_iter()
                    .filter(|(_, item)| matches!(item.payload, EventPayload::Effect(_)))
                    .map(|(seq, _)| Subject::Effect(seq)),
            );
        };
        match &event.payload {
            EventPayload::Effect(_) => {
                subjects.insert(Subject::Effect(seq));
                // A report without a provider ID matches only a unique nearby effect, so a
                // new effect can change the matches of reports and effects around it.
                let window = CLAIM_WINDOW * 2 + Duration::seconds(2);
                effects(
                    &mut subjects,
                    time_range(
                        &self.effects,
                        account,
                        event.occurred_at - window,
                        event.occurred_at + window,
                    )
                    .filter_map(|seq| self.get(seq).map(|item| (seq, item)))
                    .collect(),
                );
                subjects.extend(
                    self.claims_near(account, event.occurred_at)
                        .into_iter()
                        .map(|(seq, _)| Subject::Claim(seq)),
                );
                for (other, item) in self.by_resource(account, &event.key.resource) {
                    match &item.payload {
                        EventPayload::Claim(_) => {
                            subjects.insert(Subject::Claim(other));
                        }
                        EventPayload::ToolResult(tool) => {
                            self.task_effects(&mut subjects, account, tool);
                        }
                        _ => {}
                    }
                }
            }
            EventPayload::Claim(claim) => {
                subjects.insert(Subject::Claim(seq));
                effects(&mut subjects, self.effects_near(account, event.occurred_at));
                if let Some(resource) = &claim.provider_resource {
                    effects(&mut subjects, self.by_resource(account, resource));
                }
            }
            EventPayload::ToolResult(tool) => {
                if let Some(resource) = &tool.provider_resource {
                    effects(&mut subjects, self.by_resource(account, resource));
                }
                self.task_effects(&mut subjects, account, tool);
            }
            EventPayload::Request(_) | EventPayload::Policy(_) => {
                effects(
                    &mut subjects,
                    time_range(
                        &self.effects,
                        account,
                        event.occurred_at,
                        DateTime::<Utc>::MAX_UTC,
                    )
                    .filter_map(|seq| self.get(seq).map(|item| (seq, item)))
                    .collect(),
                );
            }
            EventPayload::Feedback(value) => {
                if let Some(effect) = self.effect_findings.get(&value.finding) {
                    subjects.insert(Subject::Effect(*effect));
                }
            }
            EventPayload::Coverage(_) => {
                subjects.insert(Subject::Coverage(account.clone()));
                self.awaiting_coverage(&mut subjects, event);
            }
            EventPayload::Notice { .. } => {
                subjects.insert(Subject::Notice(seq));
            }
            _ => {}
        }
        subjects
    }

    /// Effects linked to the tool result's task, whose linked-action counts may change.
    fn task_effects(&self, subjects: &mut BTreeSet<Subject>, account: &str, tool: &ToolResult) {
        let Some(task) = &tool.task else {
            return;
        };
        let key = (account.to_owned(), tool.agent.clone(), task.clone());
        for linked in self.list(self.tasks.get(&key)) {
            if let EventPayload::ToolResult(linked) = &linked.payload
                && let Some(resource) = &linked.provider_resource
            {
                subjects.extend(
                    self.by_resource(account, resource)
                        .into_iter()
                        .filter(|(_, item)| matches!(item.payload, EventPayload::Effect(_)))
                        .map(|(seq, _)| Subject::Effect(seq)),
                );
            }
        }
    }

    /// A coverage walk ends at the first healthy entry at or after its deadline. Claims
    /// and effects whose deadline and start precede the capability's latest earlier
    /// healthy entry already ended their walk, so a new entry cannot affect them.
    fn awaiting_coverage(&self, subjects: &mut BTreeSet<Subject>, event: &Event) {
        let Some((source, value, observed_at)) = coverage_value(event) else {
            return;
        };
        let account = &event.key.account;
        let history =
            self.coverage
                .get(&(account.clone(), source.as_str(), value.capability.clone()));
        let healthy_before = history
            .into_iter()
            .flatten()
            .rev()
            .filter(|(at, _)| *at < observed_at)
            .filter_map(|(at, seq)| {
                self.get(*seq)
                    .and_then(coverage_value)
                    .map(|item| (at, item))
            })
            .find(|(_, (_, coverage, _))| coverage.state == CoverageState::Healthy)
            .map_or(DateTime::<Utc>::MIN_UTC, |(at, _)| *at);
        let by_deadline = self
            .deadlines
            .range((healthy_before, i64::MAX)..)
            .map(|(_, seq)| *seq);
        let by_report = time_range(
            &self.claims,
            account,
            healthy_before,
            DateTime::<Utc>::MAX_UTC,
        );
        for seq in by_deadline.chain(by_report) {
            if let Some(report) = self.get(seq)
                && report.key.account == *account
                && let EventPayload::Claim(claim) = &report.payload
                && claim_source(claim) == source
            {
                subjects.insert(Subject::Claim(seq));
            }
        }
        // Request capture decides "no observed request" for effects after `healthy_before`.
        if source == Source::Messages && value.capability.starts_with(REQUEST_CAPTURE) {
            subjects.extend(
                time_range(
                    &self.effects,
                    account,
                    healthy_before,
                    DateTime::<Utc>::MAX_UTC,
                )
                .map(Subject::Effect),
            );
        }
    }

    /// Effects and claims whose review deadline passed in `(after, until]`.
    fn due(&self, after: DateTime<Utc>, until: DateTime<Utc>) -> BTreeSet<Subject> {
        if until <= after {
            return BTreeSet::new();
        }
        self.deadlines
            .range((after, i64::MAX)..=(until, i64::MAX))
            .filter_map(
                |(_, seq)| match self.get(*seq).map(|event| &event.payload) {
                    Some(EventPayload::Effect(_)) => Some(Subject::Effect(*seq)),
                    Some(EventPayload::Claim(_)) => Some(Subject::Claim(*seq)),
                    _ => None,
                },
            )
            .collect()
    }

    fn assess_subject(&self, subject: &Subject, now: DateTime<Utc>) -> Vec<Finding> {
        match subject {
            Subject::Effect(seq) => self
                .get(*seq)
                .and_then(|event| self.assess_effect(event, now))
                .map(effect_finding)
                .into_iter()
                .collect(),
            Subject::Claim(seq) => self
                .get(*seq)
                .and_then(|report| self.claim_check(report, now))
                .map(claim_finding)
                .into_iter()
                .collect(),
            Subject::Notice(seq) => self
                .get(*seq)
                .and_then(notice_assessment)
                .map(notice_finding)
                .into_iter()
                .collect(),
            Subject::Coverage(account) => self.coverage_findings(account),
        }
    }

    /// True when a pending decision needs a healthy confirmation recorded now: a claim
    /// whose deadline passed (`effects`), or an effect that request capture must cover
    /// (`requests_<agent>`). Otherwise an unchanged healthy state is not logged again.
    pub fn confirmation_due(
        &self,
        account: &str,
        source: &Source,
        capability: &str,
        now: DateTime<Utc>,
    ) -> bool {
        let confirmed = self
            .coverage
            .get(&(account.to_owned(), source.as_str(), capability.to_owned()))
            .into_iter()
            .flatten()
            .rev()
            .filter_map(|(at, seq)| {
                self.get(*seq)
                    .and_then(coverage_value)
                    .map(|item| (at, item))
            })
            .find(|(_, (_, coverage, _))| coverage.state == CoverageState::Healthy)
            .map_or(DateTime::<Utc>::MIN_UTC, |(at, _)| *at);
        if now <= confirmed {
            return false;
        }
        if *source == Source::Messages && capability.starts_with(REQUEST_CAPTURE) {
            return time_range(&self.effects, account, confirmed, now)
                .next()
                .is_some();
        }
        capability == "effects"
            && self
                .deadlines
                .range((confirmed, i64::MAX)..=(now, i64::MAX))
                .any(|(_, seq)| {
                    self.get(*seq).is_some_and(|report| {
                        report.key.account == account
                            && matches!(&report.payload, EventPayload::Claim(claim)
                                if claim_source(claim) == *source)
                    })
                })
    }
}

/// Incremental assessment over a growing log. Each `advance` reassesses only the
/// subjects that new visible events or passed deadlines can change; the result equals
/// a full replay of the same visible events at the same time.
#[derive(Clone, Debug, Default)]
pub struct Assessor {
    evidence: Evidence,
    waiting: BTreeMap<i64, Event>,
    arrivals: Vec<i64>,
    clock: Option<DateTime<Utc>>,
}

impl Assessor {
    /// Resumes from durable state: events through `assessed_through` were already
    /// assessed at `clock`. Without a clock, the next advance reassesses everything.
    pub fn resume(
        events: impl IntoIterator<Item = (i64, Event)>,
        assessed_through: i64,
        clock: Option<DateTime<Utc>>,
    ) -> Self {
        let mut assessor = Self {
            clock,
            ..Self::default()
        };
        for (seq, event) in events {
            if seq <= assessed_through && clock.is_some_and(|clock| event.ingested_at <= clock) {
                assessor.evidence.insert(seq, event);
            } else {
                assessor.waiting.insert(seq, event);
            }
        }
        assessor
    }

    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }

    /// Records a newly committed event; it is assessed once visible.
    pub fn observe(&mut self, seq: i64, event: Event, now: DateTime<Utc>) {
        if event.ingested_at <= now {
            self.evidence.insert(seq, event);
            self.arrivals.push(seq);
        } else {
            self.waiting.insert(seq, event);
        }
    }

    /// Returns the current findings for every subject that may have changed.
    pub fn advance(&mut self, now: DateTime<Utc>) -> Vec<Finding> {
        let visible: Vec<_> = self
            .waiting
            .iter()
            .filter(|(_, event)| event.ingested_at <= now)
            .map(|(seq, _)| *seq)
            .collect();
        for seq in visible {
            if let Some(event) = self.waiting.remove(&seq) {
                self.evidence.insert(seq, event);
                self.arrivals.push(seq);
            }
        }
        let arrivals = std::mem::take(&mut self.arrivals);
        let Some(clock) = self.clock else {
            self.clock = Some(now);
            return self.evidence.findings(now);
        };
        let mut subjects = self.evidence.due(clock, now);
        for seq in arrivals {
            subjects.extend(self.evidence.affected_by(seq));
        }
        self.clock = Some(clock.max(now));
        let mut result: Vec<_> = subjects
            .iter()
            .flat_map(|subject| self.evidence.assess_subject(subject, now))
            .collect();
        result.sort_by(|left, right| left.id.cmp(&right.id));
        result
    }
}

pub fn findings(events: &[Event], now: DateTime<Utc>) -> Vec<Finding> {
    Evidence::visible(events, now).findings(now)
}

/// Replaying the same retained input at the same clock time gives the same result.
/// Retention may remove earlier requests or policy history, so callers must show its horizon.
pub fn assess(events: &[Event], now: DateTime<Utc>) -> Vec<Assessment> {
    Evidence::visible(events, now).assess(now)
}

/// A metadata notice prompts review; it does not prove a setting or payment.
pub fn assess_notices(events: &[Event], now: DateTime<Utc>) -> Vec<NoticeAssessment> {
    Evidence::visible(events, now).assess_notices()
}

/// Claim checks concern observed consistency, not agent honesty.
pub fn assess_claims(events: &[Event], now: DateTime<Utc>) -> Vec<ClaimCheck> {
    Evidence::visible(events, now).assess_claims(now)
}

fn effect_finding(assessment: Assessment) -> Finding {
    Finding {
        id: assessment.finding.clone(),
        group: scoped_id(
            "incident",
            &[
                assessment.effect.account.as_str(),
                assessment.effect.source.as_str(),
                assessment.effect.resource.as_str(),
            ],
        ),
        route: assessment.route,
        detail: FindingDetail::Effect(assessment),
    }
}

fn claim_finding(claim: ClaimCheck) -> Finding {
    let subject = claim.observed_effect.as_ref().unwrap_or(&claim.claim);
    Finding {
        id: key_finding_id("claim", &claim.claim),
        group: scoped_id(
            "incident",
            &[
                subject.account.as_str(),
                subject.source.as_str(),
                subject.resource.as_str(),
            ],
        ),
        route: Route::Timeline,
        detail: FindingDetail::Claim(claim),
    }
}

fn notice_finding(notice: NoticeAssessment) -> Finding {
    Finding {
        id: key_finding_id("notice", &notice.notice),
        group: scoped_id(
            "incident",
            &[
                notice.notice.account.as_str(),
                notice.notice.source.as_str(),
                notice.notice.resource.as_str(),
            ],
        ),
        route: notice.route,
        detail: FindingDetail::Notice(notice),
    }
}

fn notice_assessment(event: &Event) -> Option<NoticeAssessment> {
    let EventPayload::Notice { kind, .. } = &event.payload else {
        return None;
    };
    Some(NoticeAssessment {
        notice: event.key.clone(),
        kind: kind.clone(),
        route: if *kind == NoticeKind::Otp {
            Route::Timeline
        } else {
            Route::MediumReview
        },
    })
}

/// Lost data and states that need the user alert; a delay that catch-up repairs does not.
fn coverage_route(evidence: &CoverageEvidence) -> Route {
    match (&evidence.state, &evidence.reason) {
        (CoverageState::Healthy, _) => Route::Timeline,
        (CoverageState::Delayed, Some(CoverageReason::AuthUnavailable)) => Route::MediumReview,
        (CoverageState::Delayed, _) => Route::Timeline,
        (CoverageState::Gap | CoverageState::Disabled, _) => Route::MediumReview,
    }
}

/// Coverage of extracted requests for one agent, recorded by the request extractor.
const REQUEST_CAPTURE: &str = "requests_";

pub fn request_capability(agent: &str) -> String {
    format!("{REQUEST_CAPTURE}{agent}")
}

/// The account source a claimed action would change.
fn claim_source(claim: &crate::event::Claim) -> Source {
    if claim.action == Action::EmailSend {
        Source::Gmail
    } else {
        Source::Calendar
    }
}

fn effect_on<'e>(event: &'e Event, source: &Source) -> Option<&'e Effect> {
    match &event.payload {
        EventPayload::Effect(effect) if event.key.source == *source => Some(effect),
        _ => None,
    }
}

fn push_sorted(list: &mut Vec<i64>, seq: i64) {
    let position = list.partition_point(|item| *item < seq);
    if list.get(position) != Some(&seq) {
        list.insert(position, seq);
    }
}

/// Sequences of one account's entries whose time falls in the inclusive range, in log order.
fn time_range(
    index: &BTreeSet<(String, DateTime<Utc>, i64)>,
    account: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> impl Iterator<Item = i64> {
    let mut seqs: Vec<_> = index
        .range((account.to_owned(), from, i64::MIN)..=(account.to_owned(), to, i64::MAX))
        .map(|(_, _, seq)| *seq)
        .collect();
    seqs.sort_unstable();
    seqs.into_iter()
}

fn wait_from(event: &Event) -> DateTime<Utc> {
    if event.occurred_at > event.ingested_at + GRACE {
        event.ingested_at
    } else {
        event.ingested_at.max(event.occurred_at)
    }
}

fn coverage_value(event: &Event) -> Option<(Source, &Coverage, DateTime<Utc>)> {
    match &event.payload {
        EventPayload::Coverage(value) => Some((event.key.source.clone(), value, event.occurred_at)),
        EventPayload::Snapshot(snapshot) if event.key.source == Source::Picket => {
            if let SnapshotValue::Coverage(value) = &snapshot.value {
                Some((
                    snapshot.original_source.clone(),
                    value,
                    snapshot.original_occurred_at,
                ))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn tool_matches(tool: &ToolResult, event: &Event, effect: &Effect) -> bool {
    tool.outcome == ToolOutcome::Succeeded
        && tool.provider_resource.as_deref() == Some(event.key.resource.as_str())
        && tool.action == effect.action
        && effect.targets_complete
        && !effect.external_change
        && same_targets(&tool.targets, &effect.targets)
        && (effect.action != Action::CalendarUpdate
            || tool.provider_revision.as_deref() == Some(event.key.revision.as_str()))
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

fn compare_request(
    request: &Request,
    effect: &Effect,
    linked_actions: Option<u32>,
    occurred_at: DateTime<Utc>,
) -> RequestFit {
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
        && effect.targets.iter().any(|target| {
            !targets.contains(target) && !named_in(&request.target_names, effect, target)
        })
    {
        conflicts.push(ConflictReason::ExtraTarget);
    }
    if request.requires_approval == Field::Known(true) && !request.approved {
        conflicts.push(ConflictReason::ApprovalMissing);
    }
    if request.valid_until.is_some_and(|until| occurred_at > until) {
        conflicts.push(ConflictReason::OutsideWindow);
    }
    if matches!(
        effect.action,
        Action::CalendarCreate | Action::CalendarUpdate | Action::CalendarRsvp
    ) && let (Field::Known(expected), Some(observed)) =
        (&request.calendar_start, effect.calendar_start)
        && *expected != observed
    {
        conflicts.push(ConflictReason::DifferentCalendarTime);
    }
    if let (Field::Known(max), Some(count)) = (&request.max_actions, linked_actions)
        && count > *max
    {
        conflicts.push(ConflictReason::TooManyActions);
    } else if request.max_actions == Field::Known(0) {
        conflicts.push(ConflictReason::TooManyActions);
    }
    if !conflicts.is_empty() {
        return RequestFit::Conflicts(conflicts);
    }
    if !effect.targets_complete
        || !matches!(request.actions, Field::Known(_) | Field::ExplicitAny)
        || !matches!(request.targets, Field::Known(_) | Field::ExplicitAny)
        || !matches!(request.requires_approval, Field::Known(_))
        || (matches!(
            effect.action,
            Action::CalendarCreate | Action::CalendarUpdate | Action::CalendarRsvp
        ) && (!matches!(request.calendar_start, Field::Known(_) | Field::ExplicitAny)
            || (matches!(request.calendar_start, Field::Known(_))
                && effect.calendar_start.is_none())))
        || !matches!(request.max_actions, Field::ExplicitAny)
            && !(matches!(request.max_actions, Field::Known(_)) && linked_actions.is_some())
    {
        return RequestFit::InsufficientEvidence;
    }
    RequestFit::ConsistentWithVisibleRequest
}

/// A claim's addresses and names describe exactly the effect's recipients: every
/// recipient is listed or named, every address is a recipient, and every name matches
/// a recipient's display name in the effect's own headers. A claim naming nobody is
/// compatible with any recipients.
fn claim_targets_match(claim: &crate::event::Claim, effect: &Effect) -> bool {
    // "Sent it!" names nobody. It can then match only through the uniqueness rule in
    // `claim_matches`: exactly one effect of that action in its window, or a provider id.
    if claim.targets.is_empty() && claim.target_names.is_empty() {
        return true;
    }
    if claim.target_names.is_empty() {
        return same_targets(&claim.targets, &effect.targets);
    }
    !effect.targets.is_empty()
        && claim
            .targets
            .iter()
            .all(|target| effect.targets.contains(target))
        && claim.target_names.iter().all(|name| {
            effect
                .targets
                .iter()
                .any(|target| named_in(std::slice::from_ref(name), effect, target))
        })
        && effect.targets.iter().all(|target| {
            claim.targets.contains(target) || named_in(&claim.target_names, effect, target)
        })
}

/// Whether one of `names` matches the display name the effect recorded for `target`.
/// Every word of the name must appear in the display name; an address is never
/// inferred from a name.
fn named_in(names: &[String], effect: &Effect, target: &str) -> bool {
    let Some(display) = effect.target_names.get(target) else {
        return false;
    };
    let display = words(display);
    names.iter().any(|name| {
        let name = words(name);
        !name.is_empty() && name.iter().all(|word| display.contains(word))
    })
}

fn words(text: &str) -> BTreeSet<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn same_targets(left: &[String], right: &[String]) -> bool {
    !left.is_empty()
        && left.len() == right.len()
        && left.iter().collect::<BTreeSet<_>>() == right.iter().collect::<BTreeSet<_>>()
}

/// The stable finding identity for an observed effect, used by feedback.
pub fn effect_finding_id(key: &EventKey) -> String {
    key_finding_id("effect", key)
}

fn key_finding_id(kind: &str, key: &EventKey) -> String {
    scoped_id(
        kind,
        &[
            key.account.as_str(),
            key.source.as_str(),
            key.resource.as_str(),
            key.revision.as_str(),
        ],
    )
}

fn scoped_id(kind: &str, parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    format!("{kind}/{:x}", hash.finalize())
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
            source_time_quality: None,
            adapter: None,
            payload,
        }
    }

    fn effect() -> Event {
        event(
            Source::Gmail,
            "message-1",
            10,
            EventPayload::Effect(Effect {
                target_names: Default::default(),
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
                target_names: Vec::new(),
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
            }),
        )
    }

    fn capture(minute: i64, state: CoverageState) -> Event {
        event(
            Source::Messages,
            &format!("capture-{minute}"),
            minute,
            EventPayload::Coverage(Coverage {
                capability: request_capability("agent-1"),
                state,
                cursor: None,
                gap_since: None,
                reason: None,
                partial_pages: None,
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
                target_names: Vec::new(),
                channel: crate::event::ClaimChannel::UserAccount,
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
                target_names: Vec::new(),
                channel: crate::event::ClaimChannel::UserAccount,
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
            one(&[late.clone(), effect(), tool()]).request_fit,
            RequestFit::RequestNotCaptured,
            "without request capture, absence means nothing"
        );
        assert_eq!(
            one(&[
                late.clone(),
                effect(),
                tool(),
                capture(15, CoverageState::Healthy)
            ])
            .request_fit,
            RequestFit::NoObservedRequest
        );
        assert_eq!(
            one(&[
                late,
                effect(),
                tool(),
                capture(5, CoverageState::Gap),
                capture(15, CoverageState::Healthy)
            ])
            .request_fit,
            RequestFit::RequestNotCaptured,
            "a gap before the effect leaves capture incomplete"
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
    fn an_unrelated_single_request_is_not_assumed_to_be_the_task() {
        let mut unhinted = tool();
        if let EventPayload::ToolResult(value) = &mut unhinted.payload {
            value.task = None;
        }
        assert_eq!(
            one(&[request(), effect(), unhinted]).request_fit,
            RequestFit::InsufficientEvidence
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

    #[test]
    fn action_limit_counts_only_task_linked_effects() {
        let mut asked = request();
        if let EventPayload::Request(value) = &mut asked.payload {
            value.max_actions = Field::Known(1);
        }
        let mut human = effect();
        human.key.resource = "human-message".into();
        human.occurred_at = time(11);
        let first = one(&[asked.clone(), effect(), tool(), human.clone()]);
        assert_eq!(first.request_fit, RequestFit::ConsistentWithVisibleRequest);
        let mut second = effect();
        second.key.resource = "message-2".into();
        second.occurred_at = time(12);
        let mut second_tool = tool();
        second_tool.key.resource = "call-2".into();
        if let EventPayload::ToolResult(value) = &mut second_tool.payload {
            value.provider_resource = Some("message-2".into());
        }
        let results = assess(
            &[asked, effect(), tool(), human, second, second_tool],
            time(20),
        );
        assert_eq!(
            results[2].request_fit,
            RequestFit::Conflicts(vec![ConflictReason::TooManyActions])
        );
    }

    #[test]
    fn calendar_time_is_a_hard_constraint() {
        let mut asked = request();
        if let EventPayload::Request(value) = &mut asked.payload {
            value.actions = Field::Known(vec![Action::CalendarCreate]);
            value.calendar_start = Field::Known(time(100));
        }
        let mut created = effect();
        created.key.source = Source::Calendar;
        if let EventPayload::Effect(value) = &mut created.payload {
            value.action = Action::CalendarCreate;
            value.calendar_start = Some(time(101));
        }
        let mut linked = tool();
        if let EventPayload::ToolResult(value) = &mut linked.payload {
            value.action = Action::CalendarCreate;
        }
        assert_eq!(
            one(&[asked, created, linked]).request_fit,
            RequestFit::Conflicts(vec![ConflictReason::DifferentCalendarTime])
        );
    }

    #[test]
    fn expired_request_and_zero_action_limit_are_explicit_conflicts() {
        let mut asked = request();
        if let EventPayload::Request(value) = &mut asked.payload {
            value.valid_until = Some(time(5));
            value.max_actions = Field::Known(0);
        }
        assert_eq!(
            one(&[asked, effect(), tool()]).request_fit,
            RequestFit::Conflicts(vec![
                ConflictReason::OutsideWindow,
                ConflictReason::TooManyActions
            ])
        );
    }

    #[test]
    fn claim_consistency_is_qualified_by_observation_and_coverage() {
        let claim = event(
            Source::Messages,
            "claim-1",
            11,
            EventPayload::Claim(Claim {
                target_names: Vec::new(),
                channel: crate::event::ClaimChannel::UserAccount,
                agent: "agent-1".into(),
                task: None,
                action: Action::EmailSend,
                targets: vec!["a@example.test".into()],
                provider_resource: Some("message-1".into()),
            }),
        );
        assert_eq!(
            assess_claims(std::slice::from_ref(&claim), time(12))[0].status,
            ClaimStatus::Pending
        );
        assert_eq!(
            assess_claims(std::slice::from_ref(&claim), time(20))[0].status,
            ClaimStatus::CoverageIncomplete
        );
        let healthy = event(
            Source::Gmail,
            "coverage-1",
            18,
            EventPayload::Coverage(Coverage {
                capability: "effects".into(),
                state: CoverageState::Healthy,
                cursor: Some("synthetic-cursor".into()),
                gap_since: None,
                reason: None,
                partial_pages: None,
            }),
        );
        assert_eq!(
            assess_claims(&[claim.clone(), healthy.clone()], time(20))[0].status,
            ClaimStatus::NotObservedInHealthySource
        );
        let mut stale = healthy.clone();
        stale.occurred_at = time(0);
        assert_eq!(
            assess_claims(&[claim.clone(), stale], time(20))[0].status,
            ClaimStatus::CoverageIncomplete
        );
        assert_eq!(
            assess_claims(&[claim.clone(), healthy.clone(), effect()], time(20))[0].status,
            ClaimStatus::Matched
        );
        let mut different = effect();
        if let EventPayload::Effect(value) = &mut different.payload {
            value.targets = vec!["b@example.test".into()];
        }
        assert_eq!(
            assess_claims(&[claim, healthy, different], time(20))[0].status,
            ClaimStatus::ContradictedByObservedEffect
        );
    }

    fn coverage(minute: i64, state: CoverageState, reason: Option<CoverageReason>) -> Event {
        event(
            Source::Gmail,
            &format!("coverage-{minute}"),
            minute,
            EventPayload::Coverage(Coverage {
                capability: "effects".into(),
                state,
                cursor: None,
                gap_since: None,
                reason,
                partial_pages: None,
            }),
        )
    }

    #[test]
    fn claim_decision_is_anchored_to_its_deadline_and_later_coverage_cannot_undo_it() {
        let claim = event(
            Source::Messages,
            "claim-1",
            11,
            EventPayload::Claim(Claim {
                target_names: Vec::new(),
                channel: crate::event::ClaimChannel::UserAccount,
                agent: "agent-1".into(),
                task: None,
                action: Action::EmailSend,
                targets: vec!["a@example.test".into()],
                provider_resource: None,
            }),
        );
        let status = |inputs: &[Event]| assess_claims(inputs, time(60))[0].status;
        let healthy = coverage(18, CoverageState::Healthy, None);
        let later_gap = coverage(30, CoverageState::Gap, None);
        assert_eq!(
            status(&[claim.clone(), healthy.clone(), later_gap]),
            ClaimStatus::NotObservedInHealthySource
        );
        let gap_between = coverage(13, CoverageState::Gap, None);
        assert_eq!(
            status(&[claim.clone(), gap_between, healthy.clone()]),
            ClaimStatus::CoverageIncomplete
        );
        let delay_between = coverage(13, CoverageState::Delayed, None);
        assert_eq!(
            status(&[claim.clone(), delay_between, healthy.clone()]),
            ClaimStatus::NotObservedInHealthySource
        );
        let gap_before = coverage(5, CoverageState::Gap, None);
        assert_eq!(
            status(&[gap_before, claim, healthy]),
            ClaimStatus::CoverageIncomplete
        );
    }

    fn named_send(recipients: &[(&str, Option<&str>)]) -> Event {
        let mut sent = effect();
        if let EventPayload::Effect(value) = &mut sent.payload {
            value.targets = recipients
                .iter()
                .map(|(address, _)| (*address).into())
                .collect();
            value.target_names = recipients
                .iter()
                .filter_map(|(address, name)| Some(((*address).into(), (*name)?.into())))
                .collect();
        }
        sent
    }

    #[test]
    fn named_people_match_only_through_the_effects_own_display_names() {
        let mut asked = request();
        if let EventPayload::Request(value) = &mut asked.payload {
            value.targets = Field::Known(vec![]);
            value.target_names = vec!["Sam".into()];
        }
        // The runtime reports the same recipients, so the actor is linked and the
        // request is compared.
        let fit = |sent: Event| {
            let mut linked = tool();
            if let (EventPayload::ToolResult(value), EventPayload::Effect(effect)) =
                (&mut linked.payload, &sent.payload)
            {
                value.targets = effect.targets.clone();
            }
            one(&[asked.clone(), sent, linked]).request_fit
        };
        assert_eq!(
            fit(named_send(&[("a@example.test", Some("Sam Lee"))])),
            RequestFit::ConsistentWithVisibleRequest
        );
        assert_eq!(
            fit(named_send(&[
                ("a@example.test", Some("Sam Lee")),
                ("b@example.test", Some("Boss Person"))
            ])),
            RequestFit::Conflicts(vec![ConflictReason::ExtraTarget])
        );
        assert_eq!(
            fit(named_send(&[("a@example.test", None)])),
            RequestFit::Conflicts(vec![ConflictReason::ExtraTarget]),
            "without a display name the recipient cannot be confirmed as Sam"
        );
    }

    #[test]
    fn named_claims_link_and_mail_outside_the_observed_account_is_out_of_scope() {
        let claim = |names: Vec<String>, channel: ClaimChannel| {
            event(
                Source::Messages,
                "claim-1",
                11,
                EventPayload::Claim(Claim {
                    agent: "agent-1".into(),
                    task: None,
                    action: Action::EmailSend,
                    targets: vec![],
                    provider_resource: None,
                    target_names: names,
                    channel,
                }),
            )
        };
        let sent = named_send(&[("a@example.test", Some("Sam Lee"))]);
        let named = claim(vec!["Sam".into()], ClaimChannel::UserAccount);
        assert_eq!(
            one(&[sent.clone(), named.clone()]).actor,
            ActorEvidence::Reported("agent-1".into())
        );
        let wrong = claim(vec!["Alex".into()], ClaimChannel::UserAccount);
        assert_eq!(one(&[sent, wrong]).actor, ActorEvidence::Unknown);
        for channel in [ClaimChannel::AgentAccount, ClaimChannel::Unknown] {
            assert_eq!(
                assess_claims(&[claim(vec!["Sam".into()], channel)], time(60))[0].status,
                ClaimStatus::OutsideObservedScope
            );
        }
        assert_eq!(
            assess_claims(&[named], time(60))[0].status,
            ClaimStatus::CoverageIncomplete
        );
    }

    #[test]
    fn a_claim_naming_nobody_links_only_a_unique_nearby_effect() {
        let claim = event(
            Source::Messages,
            "claim-1",
            11,
            EventPayload::Claim(Claim {
                agent: "agent-1".into(),
                task: None,
                action: Action::EmailSend,
                targets: vec![],
                provider_resource: None,
                target_names: vec![],
                channel: ClaimChannel::UserAccount,
            }),
        );
        assert_eq!(
            one(&[effect(), claim.clone()]).actor,
            ActorEvidence::Reported("agent-1".into())
        );
        assert_eq!(
            assess_claims(&[effect(), claim.clone()], time(60))[0].status,
            ClaimStatus::Matched
        );
        let mut second = effect();
        second.key.resource = "message-2".into();
        if let EventPayload::Effect(value) = &mut second.payload {
            value.targets = vec!["b@example.test".into()];
        }
        assert_eq!(
            one(&[effect(), second.clone(), claim]).actor,
            ActorEvidence::Unknown,
            "two candidate sends: abstain"
        );
    }

    #[test]
    fn coverage_changes_do_not_revise_effect_verdicts_and_delays_do_not_alert() {
        let effect_finding = |inputs: &[Event]| {
            findings(inputs, time(60))
                .into_iter()
                .find(|finding| matches!(finding.detail, FindingDetail::Effect(_)))
        };
        let quiet = effect_finding(&[effect()]);
        let busy = effect_finding(&[
            effect(),
            coverage(12, CoverageState::Healthy, None),
            coverage(14, CoverageState::Gap, None),
            coverage(16, CoverageState::Healthy, None),
        ]);
        assert!(quiet.is_some());
        assert_eq!(quiet, busy);
        let route = |state, reason| {
            findings(&[coverage(12, state, reason)], time(60))
                .into_iter()
                .map(|finding| finding.route)
                .next()
        };
        assert_eq!(
            route(CoverageState::Delayed, Some(CoverageReason::RequestFailed)),
            Some(Route::Timeline)
        );
        assert_eq!(
            route(
                CoverageState::Delayed,
                Some(CoverageReason::AuthUnavailable)
            ),
            Some(Route::MediumReview)
        );
        assert_eq!(route(CoverageState::Gap, None), Some(Route::MediumReview));
    }

    #[test]
    fn coverage_gap_survives_as_evidence_and_notice_is_only_a_review_prompt() {
        let gap = event(
            Source::Gmail,
            "coverage-1",
            18,
            EventPayload::Coverage(Coverage {
                capability: "effects".into(),
                state: CoverageState::Gap,
                cursor: None,
                gap_since: None,
                reason: None,
                partial_pages: None,
            }),
        );
        let inputs = [effect(), gap];
        let found = findings(&inputs, time(20));
        let coverage: Vec<_> = found
            .iter()
            .filter_map(|finding| match &finding.detail {
                FindingDetail::Coverage(value) => Some(value),
                _ => None,
            })
            .collect();
        assert_eq!(coverage.len(), 1);
        assert_eq!(coverage[0].evidence.state, CoverageState::Gap);
        assert_eq!(coverage[0].route, Route::MediumReview);
        assert_eq!(one(&inputs).route, Route::Timeline);
        let notice = event(
            Source::Gmail,
            "notice-1",
            19,
            EventPayload::Notice {
                kind: NoticeKind::Forwarding,
                origin: Origin::Live,
            },
        );
        let found = assess_notices(&[notice], time(20));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].route, Route::MediumReview);
        let baseline = event(
            Source::Gmail,
            "notice-old",
            0,
            EventPayload::Notice {
                kind: NoticeKind::Forwarding,
                origin: Origin::Baseline,
            },
        );
        assert!(assess_notices(&[baseline], time(20)).is_empty());
        let old_notice: Event = serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "key": {"account":"synthetic","source":"gmail","resource":"old-notice","revision":"1"},
            "occurred_at": time(0), "ingested_at": time(0),
            "payload": {"type":"notice","data":{"kind":"forwarding"}}
        }))
        .unwrap_or_else(|error| panic!("synthetic notice must deserialize: {error}"));
        assert!(assess_notices(&[old_notice], time(20)).is_empty());
        let mut unknown = effect();
        if let EventPayload::Effect(value) = &mut unknown.payload {
            value.origin = Origin::Unknown;
        }
        assert!(assess(&[unknown], time(20)).is_empty());
    }
}
