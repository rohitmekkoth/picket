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

pub const PROMPT_VERSION: u16 = 3;
const REQUEST_PROMPT: &str = include_str!("../prompts/request.txt");
const CLAIM_PROMPT: &str = include_str!("../prompts/claim.txt");
const VERIFY_PROMPT: &str = include_str!("../prompts/verify.txt");
const VERIFY_TOKENS: u32 = 16;
/// Model calls per pass; a backlog continues on the next pass.
const MESSAGES_PER_PASS: usize = 4;
const MAX_TEXT: usize = 4_000;
const MAX_TOKENS: u32 = 400;
const MAX_CLAIMS: usize = 4;
/// A message that could not be extracted may hold a request for this long.
const FAILURE_HORIZON: Duration = Duration::days(7);
/// How long after its latest request a task stays under discussion for follow-ups.
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
        // Follow-ups in a conversation refer to the task under discussion: the one the
        // user's most recent request created or revised. Older tasks are not offered,
        // so a short reply cannot be attached to one of them.
        let open = current_task(ledger, account, &agent.id, message.occurred_at)?;
        let previous = match observed.direction {
            MessageDirection::FromAgent => {
                ledger.previous_user_message(account, &agent.id, message.occurred_at)?
            }
            MessageDirection::FromUser => None,
        };
        let context_text = previous
            .as_ref()
            .and_then(|previous| text(&previous.key.resource));
        let context_task = open.keys().next().cloned();
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

/// The task under discussion at `at`: the latest revision of the task that the user's
/// most recent request created or revised, unless it was cancelled.
fn current_task(
    ledger: &impl Ledger,
    account: &str,
    agent: &str,
    at: DateTime<Utc>,
) -> Result<BTreeMap<String, Request>, crate::store::StoreError> {
    let mut latest: Option<(DateTime<Utc>, Request)> = None;
    for event in ledger.extractions_since(account, agent, at - TASK_HORIZON)? {
        if let EventPayload::Request(request) = event.payload
            && event.occurred_at < at
            && latest
                .as_ref()
                .is_none_or(|(time, _)| event.occurred_at >= *time)
        {
            latest = Some((event.occurred_at, request));
        }
    }
    Ok(latest
        .map(|(_, request)| request)
        .filter(|request| request.valid_until.is_none_or(|until| until > at))
        .map(|request| (request.task.clone(), request))
        .into_iter()
        .collect())
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
            .map(|(id, request)| task_summary(id, request))
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
        "Message time: {} ({}), UTC offset {}\nTask under discussion: {}\n{context}<{tag}>\n{}\n</{tag}>",
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
            let mut check =
                |question: &str, quote: &str| confirms(model, question, &text, quote, &tag);
            Ok(Proposal {
                request: request_from(raw, input, &text, &mut check)?,
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

/// Asks one narrow question about words the model quoted from a message. Only a clear
/// yes counts; a malformed answer is a no.
fn confirms(
    model: &mut dyn LocalModel,
    question: &str,
    text: &str,
    quote: &str,
    tag: &str,
) -> Result<bool, ModelError> {
    let user = format!(
        "Question: {question}\nQuoted words:\n<{tag}-quote>\n{quote}\n</{tag}-quote>\nWhole message:\n<{tag}>\n{text}\n</{tag}>"
    );
    match model.complete_json(VERIFY_PROMPT, &user, &verify_schema(), VERIFY_TOKENS) {
        Ok(answer) => Ok(answer.get("answer").and_then(Value::as_str) == Some("yes")),
        Err(ModelError::Unavailable) => Err(ModelError::Unavailable),
        Err(_) => Ok(false),
    }
}

pub fn verify_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["answer"],
        "properties": {"answer": {"enum": ["yes", "no", "unclear"]}}
    })
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
    /// The person's exact words behind each permission the model proposes widening.
    #[serde(default)]
    approves_quote: Option<String>,
    #[serde(default)]
    no_check_quote: Option<String>,
    #[serde(default)]
    any_action_quote: Option<String>,
    #[serde(default)]
    any_recipient_quote: Option<String>,
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
    /// The agent's exact words reporting the action, and placing it in a mailbox.
    #[serde(default)]
    quote: Option<String>,
    #[serde(default)]
    channel_quote: Option<String>,
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

const MAX_QUOTE: usize = 200;

pub fn request_schema() -> Value {
    let strings = json!({"type": "array", "items": {"type": "string"}, "maxItems": 8});
    let quote = json!({"type": ["string", "null"], "maxLength": MAX_QUOTE});
    let actions = json!({"type": "array", "items": {"enum": ACTIONS}, "maxItems": 6});
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["kind", "actions", "any_action", "addresses", "names",
            "any_recipient", "start", "prohibited", "max_actions", "approval", "approves",
            "cancels", "approves_quote", "no_check_quote", "any_action_quote",
            "any_recipient_quote"],
        "properties": {
            "kind": {"enum": ["new_task", "revise_task", "not_a_task"]},
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
            "cancels": {"type": "boolean"},
            "approves_quote": quote,
            "no_check_quote": quote,
            "any_action_quote": quote,
            "any_recipient_quote": quote
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
                    "required": ["action", "addresses", "names", "channel", "task", "quote",
                        "channel_quote"],
                    "properties": {
                        "action": {"enum": ACTIONS},
                        "addresses": strings,
                        "names": strings,
                        "channel": {"enum": ["user_account", "agent_account", "unknown"]},
                        "task": {"type": ["string", "null"]},
                        "quote": {"type": "string", "maxLength": MAX_QUOTE},
                        "channel_quote": {"type": ["string", "null"], "maxLength": MAX_QUOTE}
                    }
                }
            }
        }
    })
}

/// A compact view of the task under discussion for the model, without message text.
fn task_summary(id: &str, request: &Request) -> Value {
    json!({
        "id": id,
        "actions": request.actions,
        "prohibited": request.prohibited_actions,
        "addresses": request.targets,
        "names": request.target_names,
        "approval_required": request.requires_approval,
        "approved": request.approved,
    })
}

/// A yes-or-no check of quoted words, answered by the model in a separate call.
type Check<'a> = dyn FnMut(&str, &str) -> Result<bool, ModelError> + 'a;

fn request_from(
    raw: RawRequest,
    input: &Input,
    text: &str,
    check: &mut Check,
) -> Result<Option<Request>, ModelError> {
    // Only the task under discussion is offered, so a revision can only be of it.
    let current = input.open.iter().next();
    // Widening permission needs the model's proposal, the person's own words quoted from
    // this message, and a separate yes to a narrow question about those words.
    let mut grant = |proposed: bool, quote: &Option<String>, question: &str| match quote
        .as_deref()
        .filter(|quote| proposed && quoted_in(text, quote))
    {
        Some(quote) if !negated(text, quote) => check(question, quote),
        _ => Ok(false),
    };
    let approves = match current {
        Some((id, task)) => grant(
            raw.approves,
            &raw.approves_quote,
            &format!(
                "The task under discussion is {}. Do these words give the go-ahead to carry out that task as it stands, rather than ask for something new or different?",
                task_summary(id, task)
            ),
        )?,
        None => false,
    };
    // An approval answers a requirement to check first; it does not remove it.
    // Leaving the recipient or the kind of action open, or skipping the check, are
    // rarer and broader permissions; the person must say so explicitly, so the quote
    // must contain one of a few unmistakable phrases.
    let explicit = |proposed: bool, quote: &Option<String>, phrases: &[&str]| {
        quote.as_deref().is_some_and(|quote| {
            proposed && quoted_in(text, quote) && contains_phrase(quote, phrases)
        })
    };
    let no_check = !approves
        && explicit(
            matches!(raw.approval, RawApproval::NotRequired),
            &raw.no_check_quote,
            &NO_CHECK_PHRASES,
        );
    let any_action = explicit(raw.any_action, &raw.any_action_quote, &ANY_ACTION_PHRASES);
    let any_recipient = explicit(
        raw.any_recipient,
        &raw.any_recipient_quote,
        &ANY_RECIPIENT_PHRASES,
    );
    let current = current.map(|(_, task)| task);
    // Cancelling only restricts, so the model's word is enough, and a cancellation is
    // about the task under discussion even when the model calls it new.
    let cancels = raw.cancels;
    // A message naming its own recipient and its own action or prohibition is a request
    // of its own, even when the model calls it a revision; merging it would carry the
    // earlier task's limits and permissions into it.
    let (addresses, names) = recipients(&raw.addresses, &raw.names);
    let lower = text.to_lowercase();
    let names_someone = addresses
        .iter()
        .any(|address| lower.contains(address.as_str()))
        || names.iter().any(|name| names_in(text, name));
    let states_action = raw
        .actions
        .iter()
        .chain(&raw.prohibited)
        .any(|value| action(value).is_some())
        || forbids_sending(text);
    let standalone = names_someone && states_action && !approves && !cancels;
    let base = match raw.kind {
        RawKind::Revise if !standalone => current.cloned(),
        RawKind::Revise => None,
        _ if approves || cancels => current.cloned(),
        // Wording can only restrict: a standing "don't send" is kept even when the
        // model sees no task.
        RawKind::None if !forbids_sending(text) => return Ok(None),
        RawKind::None | RawKind::New => None,
    };
    let at = input.at;
    // Widening permission needs the model's proposal and the message's own words.
    let actions: Vec<_> = raw
        .actions
        .iter()
        .filter_map(|value| action(value))
        .collect();
    let actions = if any_action {
        Field::ExplicitAny
    } else if actions.is_empty() {
        // Nothing observable stated, or only actions such as booking.
        Field::Unknown
    } else {
        Field::Known(actions)
    };
    let grounded = addresses
        .iter()
        .all(|address| text.to_lowercase().contains(address.as_str()))
        && names.iter().all(|name| names_in(text, name));
    let (targets, target_names) = if any_recipient {
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
        RawApproval::NotRequired if no_check => Field::Known(false),
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
    // A follow-up stays the same kind of task: a short reply cannot turn an email task
    // into a calendar change or the reverse. The task's kind is what it allows or
    // forbids. A message asking for the other kind is a new request with no inherited
    // permissions, unless the model read it as approving or cancelling this task; then
    // the mismatched action is dropped and the task stays as it was.
    let base = match base {
        Some(base) => {
            let kinds: Vec<bool> = match &base.actions {
                Field::Known(actions) => actions.iter().map(is_email).collect(),
                _ => Vec::new(),
            }
            .into_iter()
            .chain(base.prohibited_actions.iter().map(is_email))
            .collect();
            let other_kind = !kinds.is_empty()
                && matches!(&request.actions, Field::Known(actions)
                    if !actions.is_empty() && actions.iter().all(|action| !kinds.contains(&is_email(action))));
            if other_kind && !raw.approves && !raw.cancels {
                request.task = input.id.chars().take(128).collect();
                None
            } else {
                if let Field::Known(actions) = &mut request.actions {
                    actions.retain(|action| kinds.is_empty() || kinds.contains(&is_email(action)));
                    if actions.is_empty() {
                        request.actions = Field::Unknown;
                    }
                }
                Some(base)
            }
        }
        None => None,
    };
    if let Some(mut base) = base {
        // An approval that names an action, such as "yes, send it", lifts an earlier
        // prohibition of that action on this task; it needs the person's words.
        if approves && let Field::Known(allowed) = &request.actions {
            base.prohibited_actions
                .retain(|action| !allowed.contains(action));
        }
        // A follow-up changes only what it states; everything else carries forward.
        request = merge(base, request);
    }
    if approves {
        request.approved = true;
    }
    if cancels {
        request.valid_until = Some(at);
    }
    // A forbidden action is never also allowed, including after a follow-up merges in.
    if let Field::Known(allowed) = &mut request.actions {
        allowed.retain(|action| !request.prohibited_actions.contains(action));
    }
    Ok(Some(request))
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

/// Explicit wording that the assistant may act without checking first.
const NO_CHECK_PHRASES: [&str; 14] = [
    "no need to",
    "don't need to",
    "dont need to",
    "don't have to",
    "dont have to",
    "without asking",
    "without checking",
    "don't ask",
    "don't check",
    "don't wait",
    "no need for",
    "go ahead without",
    "just send",
    "no approval",
];
/// Explicit wording that leaves the kind of action to the assistant.
const ANY_ACTION_PHRASES: [&str; 6] = [
    "whatever",
    "anything",
    "up to you",
    "your call",
    "you decide",
    "however you",
];
/// Explicit wording that lets the assistant choose who receives it.
const ANY_RECIPIENT_PHRASES: [&str; 5] = ["anyone", "anybody", "whoever", "whomever", "everyone"];

fn contains_phrase(quote: &str, phrases: &[&str]) -> bool {
    let words = words_of(quote);
    phrases
        .iter()
        .any(|phrase| position(&words, &words_of(phrase)).is_some())
}

fn is_email(action: &Action) -> bool {
    *action == Action::EmailSend
}

/// Possessives that place an email in the person's account or the agent's.
const YOURS: [&str; 3] = ["your", "yours", "you"];
const MINE: [&str; 5] = ["my", "mine", "our", "own", "i"];

fn says_whose(quote: &str, words: &[&str]) -> bool {
    words_of(quote)
        .iter()
        .any(|word| words.contains(&word.as_str()))
}

/// Function words that deny or undo a report: "not sent", "nothing went out".
const NEGATIONS: [&str; 19] = [
    "not", "never", "no", "nothing", "haven't", "hasn't", "hadn't", "didn't", "wasn't", "weren't",
    "won't", "isn't", "aren't", "can't", "couldn't", "cannot", "unable", "failed", "without",
];
/// Function words that make a report a future, an offer or an attempt: "I'll send",
/// "want me to send", "tried to send".
const UNFINISHED: [&str; 14] = [
    "will", "i'll", "we'll", "gonna", "going", "want", "should", "shall", "could", "would", "can",
    "tried", "try", "trying",
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

/// The quote's words, in order, among the text's words: where it starts.
fn position(words: &[String], quote: &[String]) -> Option<usize> {
    if quote.is_empty() {
        return None;
    }
    words
        .windows(quote.len())
        .position(|window| window == quote)
}

/// The quote appears in the text, ignoring case, spacing, punctuation and apostrophe
/// style.
fn quoted_in(text: &str, quote: &str) -> bool {
    position(&words_of(text), &words_of(quote)).is_some()
}

/// A negation inside the quote or just before it, as in "don't send it".
fn negated(text: &str, quote: &str) -> bool {
    let words = words_of(text);
    let quote = words_of(quote);
    position(&words, &quote).is_none_or(|at| {
        words[at.saturating_sub(3)..at + quote.len()]
            .iter()
            .any(|word| NEGATIONS.contains(&word.as_str()))
    })
}

/// Whether the quote states something done: it lies within one sentence that is not a
/// question, with no negation and no future, offer or attempt word in it or just before.
fn reports_completed(text: &str, quote: &str) -> bool {
    let quote = words_of(quote);
    sentences(text).any(|(sentence, question)| {
        let words = words_of(sentence);
        !question
            && position(&words, &quote).is_some_and(|at| {
                !words[at.saturating_sub(3)..at + quote.len()]
                    .iter()
                    .any(|word| {
                        NEGATIONS.contains(&word.as_str()) || UNFINISHED.contains(&word.as_str())
                    })
            })
    })
}

/// Sentences with whether each is a question. A period inside a word, as in an email
/// address, does not end a sentence.
fn sentences(text: &str) -> impl Iterator<Item = (&str, bool)> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((index, character)) = chars.next() {
        let ends = match character {
            '\n' => true,
            '.' | '!' | '?' => chars.peek().is_none_or(|(_, next)| next.is_whitespace()),
            _ => false,
        };
        if ends {
            let end = index + character.len_utf8();
            parts.push((&text[start..end], character == '?'));
            start = end;
        }
    }
    parts.push((&text[start..], false));
    parts.into_iter()
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
    let mut claims = Vec::new();
    for claim in raw.claims.into_iter().take(MAX_CLAIMS) {
        let Some(action) = action(&claim.action) else {
            continue;
        };
        // A claim needs the agent's own words reporting the action as done: quoted from
        // this message, within one sentence that is a statement, not a question, offer,
        // plan, attempt or denial. This only ever removes a claim.
        let reported = claim
            .quote
            .as_deref()
            .is_some_and(|quote| reports_completed(text, quote));
        if !reported {
            continue;
        }
        let (mut targets, mut names) = recipients(&claim.addresses, &claim.names);
        // Keep only recipients the agent's own words name, for example not a person
        // copied from the user's context. A claim left naming nobody can match only a
        // unique nearby effect.
        let lower = text.to_lowercase();
        targets.retain(|address| lower.contains(address.as_str()));
        names.retain(|name| names_in(text, name));
        // Placing the email in either mailbox changes what its absence means, so the
        // quoted words must say whose: "your" Gmail is the person's account, "my" address
        // is the agent's.
        let channel = match (claim.channel.as_str(), claim.channel_quote.as_deref()) {
            ("user_account", Some(quote))
                if quoted_in(text, quote) && says_whose(quote, &YOURS) =>
            {
                ClaimChannel::UserAccount
            }
            ("agent_account", Some(quote))
                if quoted_in(text, quote) && says_whose(quote, &MINE) =>
            {
                ClaimChannel::AgentAccount
            }
            _ => ClaimChannel::Unknown,
        };
        claims.push(Claim {
            agent: input.agent.to_owned(),
            // Kept only when the model's link agrees with the conversation (the task
            // under discussion) and that task concerns this action, as allowed or as
            // forbidden.
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
        });
    }
    claims
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
#[derive(Default)]
pub struct ScriptedModel {
    pub responses: Vec<(String, Value)>,
    /// Answers to verification questions, by a needle in the question or quote. Without
    /// one, a verification is answered yes, so scripted proposals stand as written.
    pub verdicts: Vec<(String, bool)>,
    pub calls: usize,
}

impl LocalModel for ScriptedModel {
    fn id(&self) -> &str {
        "scripted"
    }

    fn complete_json(
        &mut self,
        system: &str,
        user: &str,
        _: &Value,
        _: u32,
    ) -> Result<Value, ModelError> {
        self.calls += 1;
        if system == VERIFY_PROMPT {
            let asked = user.split("Whole message:").next().unwrap_or(user);
            let yes = self
                .verdicts
                .iter()
                .find(|(needle, _)| asked.contains(needle.as_str()))
                .is_none_or(|(_, verdict)| *verdict);
            return Ok(json!({"answer": if yes { "yes" } else { "no" }}));
        }
        // Answer for the message being extracted, not the context shown before it.
        let message = user.rsplit("-context>").next().unwrap_or(user);
        self.responses
            .iter()
            .find(|(needle, _)| message.contains(needle.as_str()))
            .map(|(_, response)| response.clone())
            .ok_or(ModelError::Invalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_match_words_in_order_regardless_of_case_and_apostrophes() {
        assert!(quoted_in("Fine by me, SEND it.", "fine by me send"));
        assert!(quoted_in(
            "You don\u{2019}t have to ask",
            "you don't have to ask"
        ));
        assert!(!quoted_in("Send it later", "send it now"));
        assert!(!quoted_in("Send it", ""));
    }

    #[test]
    fn a_report_must_be_a_statement_not_a_question_offer_attempt_or_denial() {
        for (text, quote) in [
            ("Fired off the email to Ana.", "Fired off the email to Ana"),
            (
                "Sent from your Gmail to carlos@example.test.",
                "Sent from your Gmail to carlos@example.test",
            ),
            ("Sent to Ana, but Carlos's address bounced.", "Sent to Ana"),
            (
                "Done \u{2705} I let Jordan know you're late.",
                "I let Jordan know you're late",
            ),
        ] {
            assert!(reports_completed(text, quote), "{text}");
        }
        for (text, quote) in [
            (
                "Send this from me@example.test to b@example.test?",
                "Send this from me@example.test to b@example.test",
            ),
            ("Saved it in your drafts. Not sent.", "sent"),
            ("I'll send it in 5 minutes.", "send it in 5 minutes"),
            ("Want me to send it to Alex?", "send it to Alex"),
            ("I tried to send it but Gmail returned an error.", "send it"),
            (
                "Couldn't find Ana's address, so nothing went out.",
                "went out",
            ),
            ("Sent the menu.", "Sent the dessert list"),
        ] {
            assert!(!reports_completed(text, quote), "{text}");
        }
    }

    #[test]
    fn an_approval_quote_may_not_be_negated() {
        assert!(!negated("Yes, send it", "send it"));
        assert!(negated("No, don't send that", "send that"));
        assert!(
            negated("Send it", "something else"),
            "a quote not in the text counts as unsupported"
        );
    }
}
