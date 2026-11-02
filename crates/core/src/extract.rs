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
    let pending = ledger.pending_extractions(account, MESSAGES_PER_PASS + 1)?;
    // A model is needed only for pending messages; with none, capture is caught up.
    let mut model = match model {
        Some(model) => Some(model),
        None if pending.is_empty() => None,
        None => {
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
    };
    for message in pending.iter().take(MESSAGES_PER_PASS) {
        let Some(model) = model.as_deref_mut() else {
            break;
        };
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
        let previous = match observed.direction {
            MessageDirection::FromAgent => {
                ledger.previous_user_message(account, &agent.id, message.occurred_at)?
            }
            MessageDirection::FromUser => None,
        };
        let context_text = previous
            .as_ref()
            .and_then(|previous| text(&previous.key.resource));
        // The task the user's preceding message created or revised, if any.
        let context_task = match &previous {
            Some(previous) => ledger
                .extractions_since(account, &agent.id, previous.occurred_at)?
                .into_iter()
                .find_map(|event| match event.payload {
                    EventPayload::Request(request)
                        if event.key.resource == format!("{}#request", previous.key.resource)
                            && event.key.revision == previous.key.revision =>
                    {
                        Some(request.task)
                    }
                    _ => None,
                }),
            None => None,
        };
        let context = Context {
            text: context_text.as_deref(),
            task: context_task.as_deref(),
        };
        let events = match extract_one(
            model,
            message,
            content.as_deref(),
            &open,
            context,
            zone,
            now,
        ) {
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
    text: Option<&str>,
    open: &BTreeMap<String, Request>,
    context: Context,
    zone: &dyn Fn(DateTime<Utc>) -> FixedOffset,
    now: DateTime<Utc>,
) -> Result<Vec<Event>, ModelError> {
    let EventPayload::Message(observed) = &message.payload else {
        return Ok(Vec::new());
    };
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
        context,
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
                now,
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
                    now,
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
    /// For an agent reply, what it answers. Never a source of claims.
    pub context: Context<'a>,
}

/// What an agent reply answers: the user's preceding message and the task it created
/// or revised. A reply such as "Sent!" names its task only through this, and a model's
/// task link is kept only when it agrees with `task`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Context<'a> {
    pub text: Option<&'a str>,
    pub task: Option<&'a str>,
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
            .map(|(id, request)| {
                // Only claim extraction uses the link to the latest message.
                let latest = (input.direction == MessageDirection::FromAgent)
                    .then_some(input.context.task == Some(id.as_str()));
                task_summary(id, request, latest)
            })
            .collect::<Vec<_>>(),
    )
    .map_err(|_| ModelError::Invalid)?;
    let context = match (&input.direction, input.context.text) {
        (MessageDirection::FromAgent, Some(context)) => format!(
            "The person's latest message before this reply, only for choosing the task:\n<{tag}-context>\n{}\n</{tag}-context>\n",
            context.chars().take(MAX_TEXT).collect::<String>()
        ),
        _ => String::new(),
    };
    let user = format!(
        "Message time: {} ({}), UTC offset {}\nOpen tasks: {}\n{context}<{tag}>\n{}\n</{tag}>",
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawApproval {
    Required,
    NotRequired,
    Unknown,
}

/// The model's proposal for a user message. Lists hold what the message states; an
/// empty list means it does not say.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRequest {
    kind: RawKind,
    task: Option<String>,
    actions: Vec<String>,
    any_action: bool,
    addresses: Vec<String>,
    names: Vec<String>,
    any_recipient: bool,
    start: Option<String>,
    prohibited: Vec<String>,
    max_actions: Option<u32>,
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
    let strings = json!({"type": "array", "items": {"type": "string"}, "maxItems": 8});
    let actions = json!({"type": "array", "items": {"enum": ACTIONS}, "maxItems": 6});
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["kind", "task", "actions", "any_action", "addresses", "names",
            "any_recipient", "start", "prohibited", "max_actions", "approval", "approves",
            "cancels"],
        "properties": {
            "kind": {"enum": ["new_task", "revise_task", "not_a_task"]},
            "task": {"type": ["string", "null"]},
            "actions": actions,
            "any_action": {"type": "boolean"},
            "addresses": strings,
            "names": strings,
            "any_recipient": {"type": "boolean"},
            "start": {"type": ["string", "null"], "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}$"},
            "prohibited": actions,
            "max_actions": {"type": ["integer", "null"], "minimum": 0, "maximum": 100},
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
fn task_summary(id: &str, request: &Request, from_latest_message: Option<bool>) -> Value {
    let mut summary = json!({
        "id": id,
        "actions": request.actions,
        "addresses": request.targets,
        "names": request.target_names,
        "approval_required": request.requires_approval,
        "approved": request.approved,
    });
    if let Some(latest) = from_latest_message {
        summary["from_latest_message"] = json!(latest);
    }
    summary
}

fn request_from(raw: RawRequest, input: &Input, text: &str) -> Option<Request> {
    let base = match raw.kind {
        // Wording can only restrict: a standing "don't send" is kept even when the
        // model sees no task.
        RawKind::None if !forbids_sending(text) => return None,
        RawKind::None => None,
        // An unknown task id is not trusted as a follow-up; the message stands alone.
        RawKind::Revise => raw
            .task
            .as_ref()
            .and_then(|task| input.open.get(task))
            .cloned(),
        RawKind::New => None,
    };
    let at = input.at;
    // Widening permission needs the model's proposal and the message's own words.
    let actions: Vec<_> = raw
        .actions
        .iter()
        .filter_map(|value| action(value))
        .collect();
    let actions = if raw.any_action && says(text, &ANY_ACTION_CUES) {
        Field::ExplicitAny
    } else if actions.is_empty() {
        // Nothing observable stated, or only actions such as booking.
        Field::Unknown
    } else {
        Field::Known(actions)
    };
    let (addresses, names) = recipients(&raw.addresses, &raw.names);
    let grounded = addresses
        .iter()
        .all(|address| text.to_lowercase().contains(address.as_str()))
        && names.iter().all(|name| names_in(text, name));
    let (targets, target_names) = if raw.any_recipient && says(text, &ANY_RECIPIENT_CUES) {
        (Field::ExplicitAny, Vec::new())
    } else if grounded && !(addresses.is_empty() && names.is_empty()) {
        (Field::Known(addresses), names)
    } else {
        (Field::Unknown, Vec::new())
    };
    let calendar = matches!(&actions, Field::Known(actions) if actions.iter().any(|action| *action != Action::EmailSend));
    let calendar_start = raw
        .start
        .as_deref()
        .filter(|_| calendar && mentions_time(text))
        .and_then(|value| local_time(value, input.offset))
        .filter(|time| *time > at - Duration::days(1) && *time < at + Duration::days(400))
        .map_or(Field::Unknown, Field::Known);
    let max_actions = raw
        .max_actions
        .filter(|count| counted_in(text, *count))
        .map_or(Field::Unknown, Field::Known);
    let requires_approval = match raw.approval {
        RawApproval::Required => Field::Known(true),
        RawApproval::NotRequired if says(text, &NO_APPROVAL_CUES) => Field::Known(false),
        _ => Field::Unknown,
    };
    let mut prohibited: Vec<_> = raw
        .prohibited
        .iter()
        .filter_map(|value| action(value))
        .collect();
    // Wording can only ever restrict: "don't send" forbids sending even if the model
    // missed it, unless it redirects ("don't send it to Alex").
    if forbids_sending(text) && !prohibited.contains(&Action::EmailSend) {
        prohibited.push(Action::EmailSend);
    }
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
        valid_until: None,
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
    // A forbidden action is never also allowed, including after a follow-up merges in.
    if let Field::Known(allowed) = &mut request.actions {
        allowed.retain(|action| !request.prohibited_actions.contains(action));
    }
    Some(request)
}

/// Whether a task says anything about an action: allows it, leaves the choice open, or
/// forbids it. A forbidden action still links, so the conflict can be found.
fn concerns(request: &Request, action: &Action) -> bool {
    request.prohibited_actions.contains(action)
        || match &request.actions {
            Field::Known(actions) => actions.contains(action),
            Field::ExplicitAny => true,
            Field::Unknown | Field::Ambiguous => false,
        }
}

/// Explicit wording that lets the agent choose who receives it.
const ANY_RECIPIENT_CUES: [&str; 5] = ["anyone", "anybody", "whoever", "whomever", "everyone"];
/// Explicit wording that leaves the kind of action to the agent.
const ANY_ACTION_CUES: [&str; 6] = [
    "whatever",
    "anything",
    "up to you",
    "your call",
    "you decide",
    "however you",
];
/// Wording that forbids sending, for now or at all.
const NO_SEND_CUES: [&str; 12] = [
    "don't send",
    "do not send",
    "dont send",
    "not send",
    "don't email",
    "do not email",
    "draft only",
    "only draft",
    "hold off",
    "not yet",
    "don't send yet",
    "wait to send",
];
/// Wording that places a claimed email in the user's own account.
const USER_ACCOUNT_CUES: [&str; 6] = [
    "from your",
    "your gmail",
    "your email",
    "your account",
    "your inbox",
    "as you",
];

fn forbids_sending(text: &str) -> bool {
    let normalized = format!(" {} ", words_of(text).join(" "));
    NO_SEND_CUES.iter().any(|cue| {
        let cue = format!(" {} ", words_of(cue).join(" "));
        normalized.match_indices(&cue).any(|(index, _)| {
            let rest = &normalized[index + cue.len()..];
            !(rest.starts_with("it to ") || rest.starts_with("to ") || rest.starts_with("that to "))
        })
    })
}
/// Explicit wording that the agent may act without confirming first.
const NO_APPROVAL_CUES: [&str; 10] = [
    "no need to check",
    "no need to ask",
    "no need to confirm",
    "without asking",
    "without checking",
    "don't ask",
    "don't check",
    "go ahead",
    "just send",
    "don't wait",
];

fn says(text: &str, cues: &[&str]) -> bool {
    let text = format!(" {} ", words_of(text).join(" "));
    cues.iter()
        .any(|cue| text.contains(&format!(" {} ", words_of(cue).join(" "))))
}

fn words_of(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace('\u{2019}', "'")
        .split(|character: char| !(character.is_alphanumeric() || character == '\''))
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Splits proposed recipients into normalized addresses and names. A proposed
/// "address" without an @ is a name the model put in the wrong list.
fn recipients(addresses: &[String], names: &[String]) -> (Vec<String>, Vec<String>) {
    let mut emails = Vec::new();
    // "Dana's email" names Dana.
    let person = |name: &str| {
        let name = name.trim();
        name.strip_suffix("'s")
            .or_else(|| name.strip_suffix("\u{2019}s"))
            .unwrap_or(name)
            .trim()
            .to_owned()
    };
    let mut people: Vec<String> = names.iter().map(|name| person(name)).collect();
    for value in addresses {
        let value = value.trim();
        if value.contains('@') {
            emails.push(value.to_lowercase());
        } else if !value.is_empty()
            && !people
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&person(value)))
        {
            people.push(person(value));
        }
    }
    people.retain(|name| !name.is_empty());
    (emails, people)
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
            let (mut targets, mut names) = recipients(&claim.addresses, &claim.names);
            // Keep only recipients the agent's own words name, for example not a person
            // copied from the user's context. A claim left naming nobody can match only a
            // unique nearby effect.
            let lower = text.to_lowercase();
            targets.retain(|address| lower.contains(address.as_str()));
            names.retain(|name| names_in(text, name));
            // Placing the email in the user's account lets its absence count, so the
            // message must say so.
            let channel = match claim.channel.as_str() {
                "user_account" if says(text, &USER_ACCOUNT_CUES) => ClaimChannel::UserAccount,
                "agent_account" => ClaimChannel::AgentAccount,
                _ => ClaimChannel::Unknown,
            };
            Some(Claim {
                agent: input.agent.to_owned(),
                // Kept only when the model's link agrees with the conversation (the task
                // the user's preceding message created or revised) and that task concerns
                // this action, as allowed or as forbidden.
                task: claim.task.filter(|task| {
                    Some(task.as_str()) == input.context.task
                        && input
                            .open
                            .get(task)
                            .is_some_and(|request| concerns(request, &action))
                }),
                action,
                targets,
                provider_resource: None,
                target_names: names,
                channel,
            })
        })
        .collect()
}

/// Every word of `name` appears as a word of the text outside email addresses, so a
/// name cannot be lifted from an address.
fn names_in(text: &str, name: &str) -> bool {
    let words: Vec<_> = text
        .split_whitespace()
        .filter(|token| !token.contains('@'))
        .flat_map(|token| token.split(|character: char| !character.is_alphanumeric()))
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

/// An event derived from a message: it happened when the message did, and Picket knew
/// it when it was extracted.
fn derived(
    message: &Event,
    resource: String,
    payload: EventPayload,
    model: &str,
    now: DateTime<Utc>,
) -> Event {
    Event {
        schema_version: 1,
        key: EventKey {
            account: message.key.account.clone(),
            source: Source::Messages,
            resource,
            revision: message.key.revision.clone(),
        },
        occurred_at: message.occurred_at,
        ingested_at: now.max(message.ingested_at),
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
    derived(
        message,
        message.key.resource.clone(),
        EventPayload::Extraction(ExtractionRecord {
            agent: observed.agent.clone(),
            outcome,
            model: model.chars().take(128).collect(),
            prompt_version: PROMPT_VERSION,
        }),
        model,
        now,
    )
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
        // Answer for the message being extracted, not the context shown before it.
        let message = user.rsplit("-context>").next().unwrap_or(user);
        self.responses
            .iter()
            .find(|(needle, _)| message.contains(needle.as_str()))
            .map(|(_, response)| response.clone())
            .ok_or(ModelError::Invalid)
    }
}
