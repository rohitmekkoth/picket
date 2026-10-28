//! Portable, bounded inputs for the event log and alignment engine.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Gmail,
    Calendar,
    Messages,
    OpenClaw,
    Picket,
}

impl Source {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Gmail => "gmail",
            Self::Calendar => "calendar",
            Self::Messages => "messages",
            Self::OpenClaw => "openclaw",
            Self::Picket => "picket",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EventKey {
    pub account: String,
    pub source: Source,
    pub resource: String,
    pub revision: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    EmailSend,
    CalendarCreate,
    CalendarUpdate,
    CalendarDelete,
    CalendarRsvp,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    #[default]
    Unknown,
    Baseline,
    Live,
    CatchUp,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageState {
    Healthy,
    /// Collection is behind or failing, but its cursor can still catch up without loss.
    Delayed,
    /// Some source changes may never be observed.
    Gap,
    Disabled,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageReason {
    CursorExpired,
    PageLimit,
    MissingResource,
    RequestFailed,
    AuthUnavailable,
    Interrupted,
    /// The OS denied access, for example Full Disk Access for Messages.
    PermissionMissing,
    /// The source's structure is not one this adapter version supports.
    UnsupportedFormat,
    /// Some records exist but their content could not be decoded.
    Undecodable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOutcome {
    Succeeded,
    Failed,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    Forwarding,
    AppAccess,
    Purchase,
    Otp,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EmailMetadata {
    pub thread_id: String,
    pub sender: Option<String>,
    pub subject_hash: String,
    pub subject_snippet: String,
    pub labels: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CalendarMetadata {
    pub end: Option<DateTime<Utc>>,
    pub summary: Option<String>,
    pub organizer: Option<String>,
    pub creator: Option<String>,
    pub attendees: Vec<String>,
    pub self_rsvp: Option<String>,
    pub all_day: bool,
    pub recurring_event_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccountPolicy {
    pub id: String,
    pub forbidden_actions: Vec<Action>,
    pub forbidden_targets: Vec<String>,
    #[serde(default)]
    pub high_consequence: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field<T> {
    Known(T),
    ExplicitAny,
    #[default]
    Unknown,
    Ambiguous,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    pub action: Action,
    pub targets: Vec<String>,
    pub targets_complete: bool,
    #[serde(default)]
    pub calendar_start: Option<DateTime<Utc>>,
    pub origin: Origin,
    pub external_change: bool,
    /// Display names from the effect's own headers, by normalized address.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub target_names: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<EmailMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar: Option<CalendarMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub agent: String,
    pub task: String,
    pub actions: Field<Vec<Action>>,
    pub targets: Field<Vec<String>>,
    #[serde(default)]
    pub calendar_start: Field<DateTime<Utc>>,
    pub prohibited_actions: Vec<Action>,
    pub max_actions: Field<u32>,
    pub valid_until: Option<DateTime<Utc>>,
    pub requires_approval: Field<bool>,
    pub approved: bool,
    /// People the user named without an address. They match an effect recipient only
    /// through that recipient's display name in the effect's headers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_names: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    pub agent: String,
    #[serde(default)]
    pub task: Option<String>,
    pub action: Action,
    pub targets: Vec<String>,
    pub provider_resource: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_names: Vec<String>,
    /// Where a claimed email was sent from. Only the user's own account is observed.
    #[serde(default)]
    pub channel: ClaimChannel,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimChannel {
    UserAccount,
    /// For example Instinct's own mailbox, which Picket cannot see.
    AgentAccount,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub agent: String,
    #[serde(default)]
    pub task: Option<String>,
    pub action: Action,
    pub targets: Vec<String>,
    pub provider_resource: Option<String>,
    #[serde(default)]
    pub provider_revision: Option<String>,
    pub outcome: ToolOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Coverage {
    pub capability: String,
    pub state: CoverageState,
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap_since: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<CoverageReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_pages: Option<u32>,
}

/// How Picket can observe an agent. Effects-only agents have no request or report channel.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMode {
    OpenClaw,
    Messages,
    EffectsOnly,
}

/// A user-registered agent. Handles are normalized (E.164 phone numbers or lower-case
/// emails) and identify the agent's 1:1 Messages conversation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentRegistration {
    pub id: String,
    pub name: String,
    pub mode: AgentMode,
    pub handles: Vec<String>,
    pub connected_since: Option<DateTime<Utc>>,
    pub enabled: bool,
    /// The user's ISO 3166 region, for reading national-format numbers in Messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageDirection {
    /// Sent from the user's own account, on any of their devices.
    FromUser,
    FromAgent,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageText {
    Decoded,
    /// The message carries no text, for example an attachment only.
    Absent,
    Undecodable,
    Retracted,
}

/// One message in a registered agent's conversation. The text itself is never stored;
/// extraction re-reads it from the Messages database by its identity when needed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MessageObserved {
    pub agent: String,
    pub direction: MessageDirection,
    pub service: String,
    pub text: MessageText,
    pub snippet: String,
    pub origin: Origin,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContactObserved {
    pub address: String,
    pub origin: Origin,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Feedback {
    pub finding: String,
    pub expected: Option<bool>,
    pub actor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum SnapshotValue {
    Contact(ContactObserved),
    Coverage(Coverage),
    Feedback(Feedback),
    Policy(AccountPolicy),
    Agent(AgentRegistration),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub original_source: Source,
    pub original_occurred_at: DateTime<Utc>,
    pub value: SnapshotValue,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum EventPayload {
    Effect(Effect),
    Notice {
        kind: NoticeKind,
        #[serde(default)]
        origin: Origin,
    },
    Request(Request),
    Claim(Claim),
    ToolResult(ToolResult),
    Coverage(Coverage),
    ContactObserved(ContactObserved),
    Feedback(Feedback),
    Policy(AccountPolicy),
    Agent(AgentRegistration),
    Message(MessageObserved),
    Snapshot(Snapshot),
    Checkpoint {
        through_sequence: i64,
        hash: String,
    },
}

impl EventPayload {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Effect(_) => "effect",
            Self::Notice { .. } => "notice",
            Self::Request(_) => "request",
            Self::Claim(_) => "claim",
            Self::ToolResult(_) => "tool_result",
            Self::Coverage(_) => "coverage",
            Self::ContactObserved(_) => "contact_observed",
            Self::Feedback(_) => "feedback",
            Self::Policy(_) => "policy",
            Self::Agent(_) => "agent",
            Self::Message(_) => "message",
            Self::Snapshot(_) => "snapshot",
            Self::Checkpoint { .. } => "checkpoint",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub schema_version: u16,
    pub key: EventKey,
    pub occurred_at: DateTime<Utc>,
    pub ingested_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_time_quality: Option<SourceTimeQuality>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<AdapterProvenance>,
    pub payload: EventPayload,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceTimeQuality {
    Provider,
    IngestedFallback,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AdapterProvenance {
    pub name: String,
    pub version: u16,
    pub scope: String,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Canonical {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

impl From<serde_json::Value> for Canonical {
    fn from(value: serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(value) => Self::Bool(value),
            serde_json::Value::Number(value) => Self::Number(value),
            serde_json::Value::String(value) => Self::String(value),
            serde_json::Value::Array(values) => {
                Self::Array(values.into_iter().map(Self::from).collect())
            }
            serde_json::Value::Object(values) => Self::Object(
                values
                    .into_iter()
                    .map(|(key, value)| (key, Self::from(value)))
                    .collect(),
            ),
        }
    }
}

pub fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    let value = serde_json::to_value(value)?;
    serde_json::to_vec(&Canonical::from(value))
}
