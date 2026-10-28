//! Local extraction of requests and claims from an agent conversation. A model proposes
//! structured fields; Rust decides what is kept. Direction separates trust: the user's
//! messages can only produce requests and approvals, the agent's only claims. Every kept
//! address, name, time and count must be grounded in the message text.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, FixedOffset, NaiveDateTime, TimeZone, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    engine::request_capability,
    event::{
        Action, AdapterProvenance, AgentMode, AgentRegistration, Claim, ClaimChannel, Coverage,
        CoverageReason, CoverageState, Event, EventKey, EventPayload, ExtractionOutcome,
        ExtractionRecord, Field, MessageDirection, MessageObserved, MessageText, Request, Source,
        SourceTimeQuality,
    },
    ledger::{Ledger, Progress},
    store::{Batch, CoverageReport},
};

pub const PROMPT_VERSION: u16 = 1;
const REQUEST_PROMPT: &str = include_str!("../prompts/request.txt");
const CLAIM_PROMPT: &str = include_str!("../prompts/claim.txt");
/// Model calls per pass; a backlog continues on the next pass.
const MESSAGES_PER_PASS: usize = 4;
const MAX_TEXT: usize = 4_000;
const MAX_TOKENS: u32 = 400;
const MAX_CLAIMS: usize = 4;
const MAX_OPEN_TASKS: usize = 8;
/// A message that could not be extracted may hold a request for this long.
const FAILURE_HORIZON: Duration = Duration::days(7);
/// Open tasks offered to the model for follow-ups.
const TASK_HORIZON: Duration = Duration::days(14);

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("no local model is available")]
    Unavailable,
    #[error("the model did not finish in time")]
    Timeout,
    #[error("the model output was not valid")]
    Invalid,
}

/// A local model that returns JSON constrained to a schema. Its output is a proposal.
pub trait LocalModel {
    fn id(&self) -> &str;
    fn complete_json(
        &mut self,
        system: &str,
        user: &str,
        schema: &Value,
        max_tokens: u32,
    ) -> Result<Value, ModelError>;
}

/// Extracts pending messages for every registered iMessage agent, then records
/// request-capture coverage: healthy once caught up, a gap while a recent message could
/// not be extracted, and delayed without a model, since texts stay in Messages and are
/// extracted once a model is available.
pub fn extract_pending(
    ledger: &mut impl Ledger,
    model: Option<&mut dyn LocalModel>,
    text: &mut dyn FnMut(&str) -> Option<String>,
    zone: &dyn Fn(DateTime<Utc>) -> FixedOffset,
    account: &str,
    now: DateTime<Utc>,
) -> Result<Progress, crate::store::StoreError> {
    let agents: Vec<_> = ledger
        .agents()?
        .into_iter()
        .filter(|agent| agent.enabled && agent.mode == AgentMode::Messages)
        .collect();
    if agents.is_empty() {
        return Ok(Progress::CaughtUp);
    }
    let Some(model) = model else {
        report(
            ledger,
            account,
            &agents,
            CoverageState::Delayed,
            Some(CoverageReason::ExtractionUnavailable),
            now,
        )?;
        return Ok(Progress::CaughtUp);
    };
    let pending = ledger.pending_extractions(account, MESSAGES_PER_PASS + 1)?;
    for message in pending.iter().take(MESSAGES_PER_PASS) {
        let EventPayload::Message(observed) = &message.payload else {
            continue;
        };
        let Some(agent) = agents.iter().find(|agent| agent.id == observed.agent) else {
            continue;
        };
        let content = match observed.text {
            MessageText::Decoded => text(&message.key.resource),
            _ => None,
        };
        let open = open_tasks(ledger, account, &agent.id, message.occurred_at)?;
        let events = match extract_one(model, message, observed, content.as_deref(), &open, zone) {
            Ok(events) => events,
            Err(ModelError::Unavailable) => {
                report(
                    ledger,
                    account,
                    &agents,
                    CoverageState::Delayed,
                    Some(CoverageReason::ExtractionUnavailable),
                    now,
                )?;
                return Ok(Progress::CaughtUp);
            }
            Err(_) => vec![record(
                message,
                observed,
                ExtractionOutcome::Failed,
                model.id(),
                now,
            )],
        };
        ledger.commit(
            Batch {
                events,
                ..Batch::default()
            },
            now,
        )?;
    }
    if pending.len() > MESSAGES_PER_PASS {
        return Ok(Progress::Behind);
    }
    for agent in &agents {
        let failed = ledger
            .extractions_since(account, &agent.id, now - FAILURE_HORIZON)?
            .iter()
            .any(|event| {
                matches!(&event.payload, EventPayload::Extraction(record)
                    if record.outcome == ExtractionOutcome::Failed)
            });
        let (state, reason) = if failed {
            (CoverageState::Gap, Some(CoverageReason::Undecodable))
        } else {
            (CoverageState::Healthy, None)
        };
        report(
            ledger,
            account,
            std::slice::from_ref(agent),
            state,
            reason,
            now,
        )?;
    }
    Ok(Progress::CaughtUp)
}

fn report(
    ledger: &mut impl Ledger,
    account: &str,
    agents: &[AgentRegistration],
    state: CoverageState,
    reason: Option<CoverageReason>,
    now: DateTime<Utc>,
) -> Result<(), crate::store::StoreError> {
    let coverage = agents
        .iter()
        .map(|agent| CoverageReport {
            account: account.into(),
            source: Source::Messages,
            coverage: Coverage {
                capability: request_capability(&agent.id),
                state: state.clone(),
                cursor: None,
                gap_since: None,
                reason: reason.clone(),
                partial_pages: None,
            },
        })
        .collect();
    ledger.commit(
        Batch {
            coverage,
            ..Batch::default()
        },
        now,
    )?;
    Ok(())
}

/// The latest revision of each task this agent was asked to do recently.
fn open_tasks(
    ledger: &impl Ledger,
    account: &str,
    agent: &str,
    at: DateTime<Utc>,
) -> Result<BTreeMap<String, Request>, crate::store::StoreError> {
    let mut tasks = BTreeMap::new();
    for event in ledger.extractions_since(account, agent, at - TASK_HORIZON)? {
        if let EventPayload::Request(request) = event.payload
            && event.occurred_at <= at
        {
            tasks.insert(request.task.clone(), request);
        }
    }
    tasks.retain(|_, request| request.valid_until.is_none_or(|until| until > at));
    while tasks.len() > MAX_OPEN_TASKS {
        tasks.pop_first();
    }
    Ok(tasks)
}

fn extract_one(
    model: &mut dyn LocalModel,
    message: &Event,
    observed: &MessageObserved,
    text: Option<&str>,
    open: &BTreeMap<String, Request>,
    zone: &dyn Fn(DateTime<Utc>) -> FixedOffset,
) -> Result<Vec<Event>, ModelError> {
    let now = message.ingested_at;
    let Some(text) = text.filter(|text| !text.trim().is_empty()) else {
        let outcome = match observed.text {
            MessageText::Undecodable => ExtractionOutcome::Failed,
            _ => ExtractionOutcome::NotATask,
        };
        return Ok(vec![record(message, observed, outcome, model.id(), now)]);
    };
    let input = Input {
        direction: observed.direction.clone(),
        text,
        at: message.occurred_at,
        offset: zone(message.occurred_at),
        agent: &observed.agent,
        id: &message.key.resource,
        open,
    };
    let model_id = model.id().to_owned();
    let proposal = propose(model, &input)?;
    let mut events: Vec<_> = proposal
        .request
        .map(|request| {
            derived(
                message,
                format!("{}#request", message.key.resource),
                EventPayload::Request(request),
                &model_id,
            )
        })
        .into_iter()
        .collect();
    events.extend(
        proposal
            .claims
            .into_iter()
            .enumerate()
            .map(|(index, claim)| {
                derived(
                    message,
                    format!("{}#claim{index}", message.key.resource),
                    EventPayload::Claim(claim),
                    &model_id,
                )
            }),
    );
    let outcome = if events.is_empty() {
        ExtractionOutcome::NotATask
    } else if observed.direction == MessageDirection::FromUser {
        ExtractionOutcome::Requests
    } else {
        ExtractionOutcome::Claims
    };
    events.push(record(message, observed, outcome, &model_id, now));
    Ok(events)
}

/// One message to extract, independent of storage.
pub struct Input<'a> {
    pub direction: MessageDirection,
    pub text: &'a str,
    pub at: DateTime<Utc>,
    pub offset: FixedOffset,
    pub agent: &'a str,
    /// The message identity: the task id of a new request and the data delimiter's seed.
    pub id: &'a str,
    pub open: &'a BTreeMap<String, Request>,
}

/// The validated extraction of one message: at most one request from the user, or
/// claims from the agent.
#[derive(Debug, Default, PartialEq)]
pub struct Proposal {
    pub request: Option<Request>,
    pub claims: Vec<Claim>,
}

/// Asks the model about one message and keeps only what validation accepts. The
/// pipeline and the evaluation both use this, so they measure the same behavior.
pub fn propose(model: &mut dyn LocalModel, input: &Input) -> Result<Proposal, ModelError> {
    let text: String = input.text.chars().take(MAX_TEXT).collect();
    let local = input.at.with_timezone(&input.offset);
    let tag = format!("message-{:x}", Sha256::digest(input.id))
        .chars()
        .take(16)
        .collect::<String>();
    let tasks = serde_json::to_string(
        &input
            .open
            .iter()
            .map(|(id, request)| task_summary(id, request))
            .collect::<Vec<_>>(),
    )
    .map_err(|_| ModelError::Invalid)?;
    let user = format!(
        "Message time: {} ({}), UTC offset {}\nOpen tasks: {}\n<{tag}>\n{}\n</{tag}>",
        local.format("%Y-%m-%d %H:%M"),
        local.format("%A"),
        input.offset,
        if input.open.is_empty() {
            "none".into()
        } else {
            tasks
        },
        text
    );
    match input.direction {
        MessageDirection::FromUser => {
            let raw = model.complete_json(REQUEST_PROMPT, &user, &request_schema(), MAX_TOKENS)?;
            let raw: RawRequest = serde_json::from_value(raw).map_err(|_| ModelError::Invalid)?;
            Ok(Proposal {
                request: request_from(raw, input, &text),
                claims: Vec::new(),
            })
        }
        MessageDirection::FromAgent => {
            let raw = model.complete_json(CLAIM_PROMPT, &user, &claim_schema(), MAX_TOKENS)?;
            let raw: RawClaims = serde_json::from_value(raw).map_err(|_| ModelError::Invalid)?;
            Ok(Proposal {
                request: None,
                claims: claims_from(raw, input, &text),
            })
        }
    }
}

/// What a user message asks for; the wire names match the prompt.
#[derive(Debug, Deserialize)]
enum RawKind {
    #[serde(rename = "new_task")]
    New,
    #[serde(rename = "revise_task")]
    Revise,
    #[serde(rename = "not_a_task")]
    None,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum RawState {
    Known,
    Any,
    Unknown,
    Ambiguous,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawApproval {
    Required,
    NotRequired,
    Unknown,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRequest {
    kind: RawKind,
    task: Option<String>,
    actions_state: RawState,
    actions: Vec<String>,
    recipients_state: RawState,
    addresses: Vec<String>,
    names: Vec<String>,
    start_state: RawState,
    start: Option<String>,
    prohibited: Vec<String>,
    max_actions_state: RawState,
    max_actions: Option<u32>,
    valid_until: Option<String>,
    approval: RawApproval,
    approves: bool,
    cancels: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawClaims {
    claims: Vec<RawClaim>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawClaim {
    action: String,
    addresses: Vec<String>,
    names: Vec<String>,
    channel: String,
    task: Option<String>,
}

const ACTIONS: [&str; 6] = [
    "email_send",
    "calendar_create",
    "calendar_update",
    "calendar_delete",
    "calendar_rsvp",
    "other",
];

fn action(value: &str) -> Option<Action> {
    match value {
        "email_send" => Some(Action::EmailSend),
        "calendar_create" => Some(Action::CalendarCreate),
        "calendar_update" => Some(Action::CalendarUpdate),
        "calendar_delete" => Some(Action::CalendarDelete),
        "calendar_rsvp" => Some(Action::CalendarRsvp),
        _ => None,
    }
}

pub fn request_schema() -> Value {
    let state = json!({"enum": ["known", "any", "unknown", "ambiguous"]});
    let strings = json!({"type": "array", "items": {"type": "string"}, "maxItems": 8});
    let local = json!({"type": ["string", "null"], "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}$"});
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["kind", "task", "actions_state", "actions", "recipients_state", "addresses",
            "names", "start_state", "start", "prohibited", "max_actions_state", "max_actions",
            "valid_until", "approval", "approves", "cancels"],
        "properties": {
            "kind": {"enum": ["new_task", "revise_task", "not_a_task"]},
            "task": {"type": ["string", "null"]},
            "actions_state": state,
            "actions": {"type": "array", "items": {"enum": ACTIONS}, "maxItems": 6},
            "recipients_state": state,
            "addresses": strings,
            "names": strings,
            "start_state": state,
            "start": local,
            "prohibited": {"type": "array", "items": {"enum": ACTIONS}, "maxItems": 6},
            "max_actions_state": state,
            "max_actions": {"type": ["integer", "null"], "minimum": 0, "maximum": 100},
            "valid_until": local,
            "approval": {"enum": ["required", "not_required", "unknown"]},
            "approves": {"type": "boolean"},
            "cancels": {"type": "boolean"}
        }
    })
}

pub fn claim_schema() -> Value {
    let strings = json!({"type": "array", "items": {"type": "string"}, "maxItems": 8});
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["claims"],
        "properties": {
            "claims": {
                "type": "array",
                "maxItems": MAX_CLAIMS,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["action", "addresses", "names", "channel", "task"],
                    "properties": {
                        "action": {"enum": ACTIONS},
                        "addresses": strings,
                        "names": strings,
                        "channel": {"enum": ["user_account", "agent_account", "unknown"]},
                        "task": {"type": ["string", "null"]}
                    }
                }
            }
        }
    })
}

/// A compact view of an open task for the model, without message text.
fn task_summary(id: &str, request: &Request) -> Value {
    json!({
        "id": id,
        "actions": request.actions,
        "addresses": request.targets,
        "names": request.target_names,
        "approval_required": request.requires_approval,
        "approved": request.approved,
    })
}

fn request_from(raw: RawRequest, input: &Input, text: &str) -> Option<Request> {
    let base = match raw.kind {
        RawKind::None => return None,
        // An unknown task id is not trusted as a follow-up; the message stands alone.
        RawKind::Revise => raw
            .task
            .as_ref()
            .and_then(|task| input.open.get(task))
            .cloned(),
        RawKind::New => None,
    };
    let at = input.at;
    let offset = input.offset;
    let grounded_time = |value: &Option<String>| {
        value
            .as_deref()
            .filter(|_| mentions_time(text))
            .and_then(|value| local_time(value, offset))
            .filter(|time| *time > at - Duration::days(1) && *time < at + Duration::days(400))
    };
    let actions: Vec<_> = raw
        .actions
        .iter()
        .filter_map(|value| action(value))
        .collect();
    let actions = match raw.actions_state {
        // Only unobservable actions such as booking: which observable action is allowed
        // is unknown, so no email or calendar change is either permitted or forbidden.
        RawState::Known if actions.is_empty() && !raw.actions.is_empty() => Field::Unknown,
        state => field(state, actions, true),
    };
    let addresses: Vec<_> = raw
        .addresses
        .iter()
        .map(|address| address.trim().to_lowercase())
        .collect();
    let names: Vec<_> = raw
        .names
        .iter()
        .map(|name| name.trim().to_owned())
        .collect();
    let recipients_grounded = addresses
        .iter()
        .all(|address| address.contains('@') && text.to_lowercase().contains(address.as_str()))
        && names.iter().all(|name| names_in(text, name));
    let targets = field(raw.recipients_state, addresses, recipients_grounded);
    let target_names = if matches!(targets, Field::Known(_)) {
        names
    } else {
        Vec::new()
    };
    let start = grounded_time(&raw.start);
    let calendar_start = match raw.start_state {
        RawState::Known => start.map_or(Field::Unknown, Field::Known),
        state => field(state, at, false),
    };
    let max_grounded = raw.max_actions.is_some_and(|count| counted_in(text, count));
    let max_actions = match (raw.max_actions_state, raw.max_actions) {
        (RawState::Known, Some(count)) if max_grounded => Field::Known(count),
        (RawState::Known, _) => Field::Unknown,
        (state, _) => field(state, 0, false),
    };
    let requires_approval = match raw.approval {
        RawApproval::Required => Field::Known(true),
        RawApproval::NotRequired => Field::Known(false),
        RawApproval::Unknown => Field::Unknown,
    };
    let prohibited: Vec<_> = raw
        .prohibited
        .iter()
        .filter_map(|value| action(value))
        .collect();
    let task = base.as_ref().map_or_else(
        || input.id.chars().take(128).collect(),
        |request| request.task.clone(),
    );
    let mut request = Request {
        agent: input.agent.to_owned(),
        task,
        actions,
        targets,
        calendar_start,
        prohibited_actions: prohibited,
        max_actions,
        valid_until: grounded_time(&raw.valid_until),
        requires_approval,
        approved: false,
        target_names,
    };
    if let Some(base) = base {
        // A follow-up changes only what it states; everything else carries forward.
        request = merge(base, request);
    }
    if raw.approves {
        request.approved = true;
    }
    if raw.cancels {
        request.valid_until = Some(at);
    }
    Some(request)
}

/// A follow-up field marked unknown leaves the task's earlier value in place.
fn keep<T: PartialEq>(update: Field<T>, base: Field<T>) -> Field<T> {
    if update == Field::Unknown {
        base
    } else {
        update
    }
}

fn merge(base: Request, update: Request) -> Request {
    let target_names = if update.targets == Field::Unknown {
        base.target_names
    } else {
        update.target_names
    };
    let mut prohibited = base.prohibited_actions;
    for action in update.prohibited_actions {
        if !prohibited.contains(&action) {
            prohibited.push(action);
        }
    }
    Request {
        agent: base.agent,
        task: base.task,
        actions: keep(update.actions, base.actions),
        targets: keep(update.targets, base.targets),
        calendar_start: keep(update.calendar_start, base.calendar_start),
        prohibited_actions: prohibited,
        max_actions: keep(update.max_actions, base.max_actions),
        valid_until: update.valid_until.or(base.valid_until),
        requires_approval: keep(update.requires_approval, base.requires_approval),
        approved: base.approved,
        target_names,
    }
}

fn claims_from(raw: RawClaims, input: &Input, text: &str) -> Vec<Claim> {
    raw.claims
        .into_iter()
        .take(MAX_CLAIMS)
        .filter_map(|claim| {
            let action = action(&claim.action)?;
            let targets: Vec<_> = claim
                .addresses
                .iter()
                .map(|address| address.trim().to_lowercase())
                .collect();
            let names: Vec<_> = claim
                .names
                .iter()
                .map(|name| name.trim().to_owned())
                .collect();
            // An ungrounded recipient would let a fabricated claim match an effect.
            let grounded = targets.iter().all(|address| {
                address.contains('@') && text.to_lowercase().contains(address.as_str())
            }) && names.iter().all(|name| names_in(text, name));
            if !grounded {
                return None;
            }
            let channel = match claim.channel.as_str() {
                "user_account" => ClaimChannel::UserAccount,
                "agent_account" => ClaimChannel::AgentAccount,
                _ => ClaimChannel::Unknown,
            };
            Some(Claim {
                agent: input.agent.to_owned(),
                task: claim.task.filter(|task| input.open.contains_key(task)),
                action,
                targets,
                provider_resource: None,
                target_names: names,
                channel,
            })
        })
        .collect()
}

fn field<T>(state: RawState, value: T, grounded: bool) -> Field<T> {
    match state {
        RawState::Known if grounded => Field::Known(value),
        RawState::Known | RawState::Unknown => Field::Unknown,
        RawState::Any => Field::ExplicitAny,
        RawState::Ambiguous => Field::Ambiguous,
    }
}

/// Every word of `name` appears as a word of the text.
fn names_in(text: &str, name: &str) -> bool {
    let words: Vec<_> = text
        .split(|character: char| !character.is_alphanumeric())
        .map(str::to_lowercase)
        .collect();
    let name: Vec<_> = name
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    !name.is_empty() && name.iter().all(|word| words.contains(word))
}

const TIME_WORDS: [&str; 21] = [
    "today",
    "tonight",
    "tomorrow",
    "noon",
    "midnight",
    "morning",
    "afternoon",
    "evening",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "week",
    "month",
    "am",
    "pm",
    "next",
    "weekend",
];

fn mentions_time(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.chars().any(|character| character.is_ascii_digit())
        || lower
            .split(|character: char| !character.is_alphanumeric())
            .any(|word| TIME_WORDS.contains(&word))
}

const COUNT_WORDS: [(&str, u32); 8] = [
    ("once", 1),
    ("one", 1),
    ("single", 1),
    ("twice", 2),
    ("two", 2),
    ("three", 3),
    ("none", 0),
    ("no", 0),
];

fn counted_in(text: &str, count: u32) -> bool {
    let lower = text.to_lowercase();
    lower
        .split(|character: char| !character.is_alphanumeric())
        .any(|word| {
            word.parse::<u32>().ok() == Some(count)
                || COUNT_WORDS
                    .iter()
                    .any(|(name, value)| *name == word && *value == count)
        })
}

fn local_time(value: &str, offset: FixedOffset) -> Option<DateTime<Utc>> {
    let naive = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M").ok()?;
    offset
        .from_local_datetime(&naive)
        .single()
        .map(|time| time.with_timezone(&Utc))
}

fn derived(message: &Event, resource: String, payload: EventPayload, model: &str) -> Event {
    Event {
        schema_version: 1,
        key: EventKey {
            account: message.key.account.clone(),
            source: Source::Messages,
            resource,
            revision: message.key.revision.clone(),
        },
        occurred_at: message.occurred_at,
        ingested_at: message.ingested_at,
        source_time_quality: message
            .source_time_quality
            .clone()
            .or(Some(SourceTimeQuality::Provider)),
        adapter: Some(AdapterProvenance {
            name: "extract".into(),
            version: PROMPT_VERSION,
            scope: model.chars().take(128).collect(),
        }),
        payload,
    }
}

fn record(
    message: &Event,
    observed: &MessageObserved,
    outcome: ExtractionOutcome,
    model: &str,
    now: DateTime<Utc>,
) -> Event {
    let mut event = derived(
        message,
        message.key.resource.clone(),
        EventPayload::Extraction(ExtractionRecord {
            agent: observed.agent.clone(),
            outcome,
            model: model.chars().take(128).collect(),
            prompt_version: PROMPT_VERSION,
        }),
        model,
    );
    event.ingested_at = now.max(message.ingested_at);
    event
}

/// A scripted model for tests and the CI evaluation: it answers by message content.
pub struct ScriptedModel {
    pub responses: Vec<(String, Value)>,
    pub calls: usize,
}

impl LocalModel for ScriptedModel {
    fn id(&self) -> &str {
        "scripted"
    }

    fn complete_json(
        &mut self,
        _: &str,
        user: &str,
        _: &Value,
        _: u32,
    ) -> Result<Value, ModelError> {
        self.calls += 1;
        self.responses
            .iter()
            .find(|(needle, _)| user.contains(needle.as_str()))
            .map(|(_, response)| response.clone())
            .ok_or(ModelError::Invalid)
    }
}
