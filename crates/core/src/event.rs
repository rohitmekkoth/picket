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
    Baseline,
    #[default]
    Live,
    CatchUp,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageState {
    Healthy,
    Gap,
    Disabled,
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
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    pub agent: String,
    #[serde(default)]
    pub task: Option<String>,
    pub action: Action,
    pub targets: Vec<String>,
    pub provider_resource: Option<String>,
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
    pub payload: EventPayload,
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
