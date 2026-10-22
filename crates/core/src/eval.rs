//! Fixed synthetic evaluation cases. Truth labels never enter engine input.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    engine::{ActorEvidence, FindingDetail, RequestFit, Route},
    event::{
        AccountPolicy, Action, Claim, Coverage, CoverageState, Effect, Event, EventKey,
        EventPayload, Field, Origin, Request, Source, ToolOutcome, ToolResult,
    },
    store::Store,
};

const SCENARIOS: &str = include_str!("../../../eval/scenarios/held_out.json");
const BASELINE: &str = include_str!("../../../eval/baseline.json");

#[derive(Debug, Deserialize)]
struct Baseline {
    version: u32,
    minimum_families: usize,
    minimum_modes: usize,
    minimum_cases: usize,
    minimum_human_effects: usize,
    minimum_linkable_effects: usize,
    minimum_explicit_conflicts: usize,
    minimum_benign_effects: usize,
    minimum_coverage_gaps: usize,
    minimum_linkage_recall_percent: usize,
    minimum_conflict_recall_percent: usize,
    maximum_benign_alert_percent: usize,
}

#[derive(Debug, Deserialize)]
struct Suite {
    version: u32,
    families: Vec<Family>,
}

#[derive(Debug, Deserialize)]
struct Family {
    id: String,
    mode: Mode,
    kind: Kind,
    count: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    EffectsOnly,
    Instrumented,
    Policy,
    Calendar,
    Coverage,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    HumanSameMetadata,
    AgentSameMetadata,
    FailedRuntime,
    ResourceMismatch,
    AmbiguousClaim,
    ReportedOnly,
    ConflictingRuntime,
    LinkedBenign,
    LinkedWrongTarget,
    LinkedProhibited,
    LinkedMissingApproval,
    LinkedNoRequest,
    CalendarWrongTime,
    HumanPolicyBreach,
    DisabledPolicy,
    ExternalCalendar,
    BaselineImport,
    CollectorGap,
}

impl Kind {
    fn mode(self) -> Mode {
        match self {
            Self::HumanSameMetadata
            | Self::AgentSameMetadata
            | Self::FailedRuntime
            | Self::ResourceMismatch
            | Self::AmbiguousClaim
            | Self::ReportedOnly
            | Self::ConflictingRuntime => Mode::EffectsOnly,
            Self::LinkedBenign
            | Self::LinkedWrongTarget
            | Self::LinkedProhibited
            | Self::LinkedMissingApproval
            | Self::LinkedNoRequest
            | Self::CalendarWrongTime => Mode::Instrumented,
            Self::HumanPolicyBreach | Self::DisabledPolicy => Mode::Policy,
            Self::ExternalCalendar => Mode::Calendar,
            Self::BaselineImport | Self::CollectorGap => Mode::Coverage,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Counts {
    pub cases: usize,
    pub evaluated_effects: usize,
    pub named_agents: usize,
    pub correct_named_agents: usize,
    pub human_effects: usize,
    pub humans_named_as_agents: usize,
    pub linkable_effects: usize,
    pub linked_effects: usize,
    pub unknown_actors: usize,
    pub reported_actors: usize,
    pub conflicting_actors: usize,
    pub explicit_conflicts: usize,
    pub detected_conflicts: usize,
    pub false_consistent: usize,
    pub alert_worthy: usize,
    pub alerts_on_alert_worthy: usize,
    pub expected_high: usize,
    pub correct_high: usize,
    pub expected_medium: usize,
    pub correct_medium: usize,
    pub high_alerts: usize,
    pub medium_alerts: usize,
    pub benign_effects: usize,
    pub alerts_on_benign: usize,
    pub applicable_coverage_gaps: usize,
    pub surfaced_coverage_gaps: usize,
}

impl Counts {
    fn add(&mut self, other: &Self) {
        self.cases += other.cases;
        self.evaluated_effects += other.evaluated_effects;
        self.named_agents += other.named_agents;
        self.correct_named_agents += other.correct_named_agents;
        self.human_effects += other.human_effects;
        self.humans_named_as_agents += other.humans_named_as_agents;
        self.linkable_effects += other.linkable_effects;
        self.linked_effects += other.linked_effects;
        self.unknown_actors += other.unknown_actors;
        self.reported_actors += other.reported_actors;
        self.conflicting_actors += other.conflicting_actors;
        self.explicit_conflicts += other.explicit_conflicts;
        self.detected_conflicts += other.detected_conflicts;
        self.false_consistent += other.false_consistent;
        self.alert_worthy += other.alert_worthy;
        self.alerts_on_alert_worthy += other.alerts_on_alert_worthy;
        self.expected_high += other.expected_high;
        self.correct_high += other.correct_high;
        self.expected_medium += other.expected_medium;
        self.correct_medium += other.correct_medium;
        self.high_alerts += other.high_alerts;
        self.medium_alerts += other.medium_alerts;
        self.benign_effects += other.benign_effects;
        self.alerts_on_benign += other.alerts_on_benign;
        self.applicable_coverage_gaps += other.applicable_coverage_gaps;
        self.surfaced_coverage_gaps += other.surfaced_coverage_gaps;
    }
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub suite_version: u32,
    pub baseline_version: u32,
    pub engine_version: &'static str,
    pub schema_version: u32,
    pub model: &'static str,
    pub adapter: &'static str,
    pub interval_method: &'static str,
    pub sampling_limit: &'static str,
    pub not_measured: Vec<&'static str>,
    pub totals: Counts,
    pub modes: BTreeMap<Mode, Counts>,
    pub families: BTreeMap<String, Counts>,
    pub gates_passed: bool,
    pub failures: Vec<String>,
}

#[derive(Clone, Copy)]
enum TruthActor {
    Human,
    Agent,
    Unknown,
}

struct Case {
    events: Vec<Event>,
    resource: String,
    agent: String,
    truth_actor: TruthActor,
    linkable: bool,
    conflict: bool,
    expected_route: Route,
    coverage_gap: bool,
    observed_effect: bool,
    reported: bool,
    conflicting: bool,
}

fn at(minute: i64) -> DateTime<Utc> {
    Utc.timestamp_nanos((1_700_000_000 + minute * 60) * 1_000_000_000)
}

fn event(source: Source, resource: String, minute: i64, payload: EventPayload) -> Event {
    Event {
        schema_version: 1,
        key: EventKey {
            account: "synthetic".into(),
            source,
            resource,
            revision: "1".into(),
        },
        occurred_at: at(minute),
        ingested_at: at(minute),
        payload,
    }
}

fn case(kind: Kind, index: usize) -> Case {
    let resource = format!("mock-{index}");
    let target = format!("person-{index}@example.test");
    let task = format!("task-{index}");
    let agent = format!("agent-{index}");
    let mut request = Request {
        agent: agent.clone(),
        task: task.clone(),
        actions: Field::Known(vec![Action::EmailSend]),
        targets: Field::Known(vec![target.clone()]),
        calendar_start: Field::Unknown,
        prohibited_actions: vec![],
        max_actions: Field::ExplicitAny,
        valid_until: None,
        requires_approval: Field::Known(false),
        approved: false,
    };
    let mut effect = Effect {
        action: Action::EmailSend,
        targets: vec![target.clone()],
        targets_complete: true,
        calendar_start: None,
        origin: if index.is_multiple_of(2) {
            Origin::Live
        } else {
            Origin::CatchUp
        },
        external_change: false,
        email: None,
    };
    let mut tool = ToolResult {
        agent: agent.clone(),
        task: Some(task),
        action: Action::EmailSend,
        targets: vec![target.clone()],
        provider_resource: Some(resource.clone()),
        provider_revision: None,
        outcome: ToolOutcome::Succeeded,
    };
    let mut events = Vec::new();
    let mut truth_actor = TruthActor::Human;
    let mut linkable = false;
    let mut conflict = false;
    let mut expected_route = Route::Timeline;
    let mut coverage_gap = false;
    let mut observed_effect = true;
    let mut reported = false;
    let mut conflicting = false;

    match kind {
        Kind::AgentSameMetadata => truth_actor = TruthActor::Agent,
        Kind::FailedRuntime => tool.outcome = ToolOutcome::Failed,
        Kind::ResourceMismatch => tool.provider_resource = Some(format!("other-{index}")),
        Kind::AmbiguousClaim => {
            events.push(event(
                Source::Messages,
                format!("claim-{index}"),
                11,
                EventPayload::Claim(Claim {
                    agent: agent.clone(),
                    task: None,
                    action: Action::EmailSend,
                    targets: vec![target.clone()],
                    provider_resource: None,
                }),
            ));
            events.push(event(
                Source::Gmail,
                format!("other-{index}"),
                11,
                EventPayload::Effect(effect.clone()),
            ));
        }
        Kind::ReportedOnly => {
            truth_actor = TruthActor::Agent;
            reported = true;
            events.push(event(
                Source::Messages,
                format!("claim-{index}"),
                11,
                EventPayload::Claim(Claim {
                    agent: agent.clone(),
                    task: None,
                    action: Action::EmailSend,
                    targets: vec![target.clone()],
                    provider_resource: Some(resource.clone()),
                }),
            ));
        }
        Kind::ConflictingRuntime => {
            truth_actor = TruthActor::Unknown;
            conflicting = true;
            let mut second = tool.clone();
            second.agent = format!("other-agent-{index}");
            events.push(event(
                Source::OpenClaw,
                format!("second-call-{index}"),
                10,
                EventPayload::ToolResult(second),
            ));
        }
        Kind::LinkedBenign | Kind::LinkedNoRequest => {
            truth_actor = TruthActor::Agent;
            linkable = true;
        }
        Kind::LinkedWrongTarget => {
            truth_actor = TruthActor::Agent;
            linkable = true;
            conflict = true;
            expected_route = Route::MediumReview;
            let wrong = format!("other-{index}@example.test");
            effect.targets = vec![wrong.clone()];
            tool.targets = vec![wrong];
        }
        Kind::LinkedProhibited => {
            truth_actor = TruthActor::Agent;
            linkable = true;
            conflict = true;
            expected_route = Route::MediumReview;
            request.prohibited_actions.push(Action::EmailSend);
        }
        Kind::LinkedMissingApproval => {
            truth_actor = TruthActor::Agent;
            linkable = true;
            conflict = true;
            expected_route = Route::MediumReview;
            request.requires_approval = Field::Known(true);
        }
        Kind::CalendarWrongTime => {
            truth_actor = TruthActor::Agent;
            linkable = true;
            conflict = true;
            expected_route = Route::MediumReview;
            request.actions = Field::Known(vec![Action::CalendarCreate]);
            request.calendar_start = Field::Known(at(100));
            effect.action = Action::CalendarCreate;
            effect.calendar_start = Some(at(101));
            tool.action = Action::CalendarCreate;
        }
        Kind::HumanPolicyBreach | Kind::DisabledPolicy => {
            let alert = matches!(kind, Kind::HumanPolicyBreach);
            if alert {
                expected_route = Route::HighReview;
            }
            events.push(event(
                Source::Picket,
                format!("policy-{index}"),
                0,
                EventPayload::Policy(AccountPolicy {
                    id: format!("no-send-{index}"),
                    forbidden_actions: vec![Action::EmailSend],
                    forbidden_targets: vec![],
                    high_consequence: true,
                    enabled: alert,
                }),
            ));
        }
        Kind::ExternalCalendar => {
            truth_actor = TruthActor::Unknown;
            effect.action = Action::CalendarUpdate;
            effect.external_change = true;
        }
        Kind::BaselineImport => {
            effect.origin = Origin::Baseline;
            observed_effect = false;
        }
        Kind::CollectorGap => {
            observed_effect = false;
            coverage_gap = true;
            events.push(event(
                Source::Gmail,
                format!("coverage-{index}"),
                10,
                EventPayload::Coverage(Coverage {
                    capability: "sent_metadata".into(),
                    state: CoverageState::Gap,
                    cursor: None,
                }),
            ));
        }
        Kind::HumanSameMetadata => {}
    }
    if matches!(kind.mode(), Mode::Instrumented) && !matches!(kind, Kind::LinkedNoRequest) {
        events.push(event(
            Source::Messages,
            format!("request-{index}"),
            0,
            EventPayload::Request(request),
        ));
    }
    if !matches!(kind, Kind::CollectorGap) {
        let source = if matches!(kind, Kind::ExternalCalendar | Kind::CalendarWrongTime) {
            Source::Calendar
        } else {
            Source::Gmail
        };
        events.push(event(
            source,
            resource.clone(),
            10,
            EventPayload::Effect(effect),
        ));
    }
    if linkable
        || matches!(
            kind,
            Kind::FailedRuntime | Kind::ResourceMismatch | Kind::ConflictingRuntime
        )
    {
        events.push(event(
            Source::OpenClaw,
            format!("call-{index}"),
            10,
            EventPayload::ToolResult(tool),
        ));
    }
    if index % 4 == 1 {
        events.reverse();
    }
    Case {
        events,
        resource,
        agent,
        truth_actor,
        linkable,
        conflict,
        expected_route,
        coverage_gap,
        observed_effect,
        reported,
        conflicting,
    }
}

fn evaluate_case(case: Case) -> Result<(Counts, Vec<String>), Box<dyn std::error::Error>> {
    let mut store = Store::memory()?;
    for event in &case.events {
        store.append_assessed(event, at(30))?;
    }
    store.reconcile(at(30))?;
    let findings = store.current_findings()?;
    let effects: Vec<_> = findings
        .iter()
        .filter_map(|(_, finding)| match &finding.detail {
            FindingDetail::Effect(value) if value.effect.resource == case.resource => Some(value),
            _ => None,
        })
        .collect();
    let gaps = findings
        .iter()
        .filter(|(_, finding)| matches!(finding.detail, FindingDetail::Coverage(_)))
        .count();
    let mut counts = Counts {
        cases: 1,
        applicable_coverage_gaps: usize::from(case.coverage_gap),
        surfaced_coverage_gaps: usize::from(case.coverage_gap && gaps == 1),
        ..Counts::default()
    };
    let mut failures = Vec::new();
    if effects.len() != usize::from(case.observed_effect) {
        failures.push("effect count disagrees with source origin".into());
        return Ok((counts, failures));
    }
    if let Some(value) = effects.first() {
        counts.evaluated_effects = 1;
        let named = match &value.actor {
            ActorEvidence::RuntimeLinked(name) | ActorEvidence::UserConfirmed(name) => {
                Some(name.as_str())
            }
            ActorEvidence::Unknown => {
                counts.unknown_actors = 1;
                None
            }
            ActorEvidence::Conflicting(_) => {
                counts.conflicting_actors = 1;
                None
            }
            ActorEvidence::Reported(_) => {
                counts.reported_actors = 1;
                None
            }
        };
        counts.named_agents = usize::from(named.is_some());
        counts.correct_named_agents = usize::from(
            named == Some(case.agent.as_str()) && matches!(case.truth_actor, TruthActor::Agent),
        );
        counts.human_effects = usize::from(matches!(case.truth_actor, TruthActor::Human));
        counts.humans_named_as_agents = usize::from(counts.human_effects == 1 && named.is_some());
        counts.linkable_effects = usize::from(case.linkable);
        counts.linked_effects = usize::from(matches!(value.actor, ActorEvidence::RuntimeLinked(_)));
        counts.explicit_conflicts = usize::from(case.conflict);
        counts.detected_conflicts =
            usize::from(case.conflict && matches!(value.request_fit, RequestFit::Conflicts(_)));
        counts.false_consistent = usize::from(
            case.conflict && value.request_fit == RequestFit::ConsistentWithVisibleRequest,
        );
        let alert = case.expected_route != Route::Timeline;
        counts.alert_worthy = usize::from(alert);
        counts.alerts_on_alert_worthy = usize::from(alert && value.route != Route::Timeline);
        counts.expected_high = usize::from(case.expected_route == Route::HighReview);
        counts.correct_high = usize::from(
            case.expected_route == Route::HighReview && value.route == Route::HighReview,
        );
        counts.expected_medium = usize::from(case.expected_route == Route::MediumReview);
        counts.correct_medium = usize::from(
            case.expected_route == Route::MediumReview && value.route == Route::MediumReview,
        );
        counts.high_alerts = usize::from(value.route == Route::HighReview);
        counts.medium_alerts = usize::from(value.route == Route::MediumReview);
        counts.benign_effects = usize::from(!alert);
        counts.alerts_on_benign = usize::from(!alert && value.route != Route::Timeline);
        if case.linkable && !matches!(value.actor, ActorEvidence::RuntimeLinked(_)) {
            failures.push("instrumented effect was not runtime linked".into());
        }
        if counts.humans_named_as_agents > 0 {
            failures.push("human effect received agent attribution".into());
        }
        if named.is_some() && counts.correct_named_agents == 0 {
            failures.push("named agent disagrees with truth".into());
        }
        if counts.false_consistent > 0 {
            failures.push("explicit conflict marked consistent".into());
        }
        if value.route != case.expected_route {
            failures.push("alert severity disagrees with truth label".into());
        }
        if case.reported != matches!(value.actor, ActorEvidence::Reported(_)) {
            failures.push("claim attribution disagrees with truth label".into());
        }
        if case.conflicting != matches!(value.actor, ActorEvidence::Conflicting(_)) {
            failures.push("conflicting runtime evidence was not preserved".into());
        }
    }
    if case.coverage_gap && gaps != 1 {
        failures.push("collector gap was not surfaced".into());
    }
    Ok((counts, failures))
}

fn gate(totals: &Counts, baseline: &Baseline) -> Vec<String> {
    let mut failures = Vec::new();
    let requirements = [
        (totals.cases >= baseline.minimum_cases, "scenario minimum"),
        (
            totals.human_effects >= baseline.minimum_human_effects,
            "human denominator",
        ),
        (
            totals.linkable_effects >= baseline.minimum_linkable_effects,
            "linkable denominator",
        ),
        (
            totals.explicit_conflicts >= baseline.minimum_explicit_conflicts,
            "conflict denominator",
        ),
        (
            totals.benign_effects >= baseline.minimum_benign_effects,
            "benign denominator",
        ),
        (
            totals.applicable_coverage_gaps >= baseline.minimum_coverage_gaps,
            "gap denominator",
        ),
        (totals.humans_named_as_agents == 0, "false human blame"),
        (
            totals.correct_named_agents == totals.named_agents,
            "named-agent precision",
        ),
        (totals.false_consistent == 0, "false consistent"),
        (
            totals.linked_effects * 100
                >= totals.linkable_effects * baseline.minimum_linkage_recall_percent,
            "linkage recall",
        ),
        (
            totals.detected_conflicts * 100
                >= totals.explicit_conflicts * baseline.minimum_conflict_recall_percent,
            "conflict recall",
        ),
        (
            totals.alerts_on_benign * 100
                <= totals.benign_effects * baseline.maximum_benign_alert_percent,
            "alert false-positive rate",
        ),
        (
            totals.alerts_on_alert_worthy == totals.alert_worthy,
            "alert recall",
        ),
        (
            totals.correct_high == totals.expected_high,
            "high alert recall",
        ),
        (
            totals.correct_medium == totals.expected_medium,
            "medium alert recall",
        ),
        (
            totals.high_alerts == totals.correct_high,
            "high alert precision",
        ),
        (
            totals.medium_alerts == totals.correct_medium,
            "medium alert precision",
        ),
        (
            totals.surfaced_coverage_gaps == totals.applicable_coverage_gaps,
            "gap visibility",
        ),
    ];
    for (passed, label) in requirements {
        if !passed {
            failures.push(label.to_owned());
        }
    }
    failures
}

pub fn mock_report() -> Result<Report, Box<dyn std::error::Error>> {
    let suite: Suite = serde_json::from_str(SCENARIOS)?;
    let baseline: Baseline = serde_json::from_str(BASELINE)?;
    let mut totals = Counts::default();
    let mut modes: BTreeMap<Mode, Counts> = BTreeMap::new();
    let mut families = BTreeMap::new();
    let mut failures = Vec::new();
    let mut ids = BTreeSet::new();
    for family in &suite.families {
        if !ids.insert(family.id.as_str()) || family.count < 16 || family.mode != family.kind.mode()
        {
            failures.push(format!("{}: invalid family definition", family.id));
            continue;
        }
        let mut family_counts = Counts::default();
        for index in 0..family.count {
            let (counts, case_failures) = evaluate_case(case(family.kind, index))?;
            family_counts.add(&counts);
            failures.extend(
                case_failures
                    .into_iter()
                    .map(|reason| format!("{}#{index}: {reason}", family.id)),
            );
        }
        totals.add(&family_counts);
        modes.entry(family.mode).or_default().add(&family_counts);
        families.insert(family.id.clone(), family_counts);
    }
    if families.len() < baseline.minimum_families {
        failures.push("family minimum".into());
    }
    if modes.len() < baseline.minimum_modes {
        failures.push("mode minimum".into());
    }
    failures.extend(gate(&totals, &baseline));
    Ok(Report {
        suite_version: suite.version,
        baseline_version: baseline.version,
        engine_version: env!("CARGO_PKG_VERSION"),
        schema_version: 1,
        model: "mock typed inputs",
        adapter: "synthetic",
        interval_method: "not_measured: correlated synthetic variants are not independent samples",
        sampling_limit: "18 fixed families with 16 variants each; no population accuracy inference",
        not_measured: vec![
            "real-source attribution",
            "extraction",
            "latency",
            "alerts per day",
        ],
        totals,
        modes,
        families,
        gates_passed: failures.is_empty(),
        failures,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_mock_suite_passes_and_abstention_fails_utility()
    -> Result<(), Box<dyn std::error::Error>> {
        let report = mock_report()?;
        assert!(report.gates_passed, "{:?}", report.failures);
        assert_eq!(report.totals.cases, 288);
        let mut all_unknown = report.totals.clone();
        all_unknown.linked_effects = 0;
        all_unknown.detected_conflicts = 0;
        let baseline: Baseline = serde_json::from_str(BASELINE)?;
        assert!(gate(&all_unknown, &baseline).contains(&"linkage recall".to_owned()));
        assert!(gate(&all_unknown, &baseline).contains(&"conflict recall".to_owned()));
        Ok(())
    }
}
