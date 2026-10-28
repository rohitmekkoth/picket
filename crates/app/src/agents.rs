//! Agent registration commands and the Messages capture worker.

use std::{
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

use chrono::{Local, Offset, TimeZone, Utc};
use picket_core::{
    agent,
    event::{AgentMode, AgentRegistration, Event, EventKey, EventPayload, Source},
    extract::{LocalModel, extract_pending},
    ledger::{Ledger, Progress, Writer},
    messages::{self, ChatDb, MessagesError},
    store::Batch,
};
use serde::Serialize;
use tauri::Manager;

use crate::GoogleState;

const POLL: Duration = Duration::from_secs(5);
/// The 2.7 GB model stays loaded only while it has recent work.
const MODEL_IDLE: Duration = Duration::from_secs(10 * 60);

/// The loaded extractor and when it last ran.
#[derive(Default)]
struct Extractor {
    runner: Option<picket_model::Runner>,
    used: Option<Instant>,
    failed: bool,
}

/// What Messages capture is doing, for the UI.
pub struct MessagesState(Mutex<&'static str>);

impl Default for MessagesState {
    fn default() -> Self {
        Self(Mutex::new("inactive"))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentView {
    id: String,
    name: String,
    mode: AgentMode,
    handles: Vec<String>,
    enabled: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentsView {
    agents: Vec<AgentView>,
    messages: &'static str,
}

#[tauri::command]
pub fn list_agents(app: tauri::AppHandle) -> Result<AgentsView, String> {
    let agents = crate::ledger(&app)?
        .agents()
        .map_err(|_| "agents unavailable".to_string())?;
    let messages = app
        .state::<MessagesState>()
        .0
        .lock()
        .map_or("unavailable", |status| *status);
    Ok(AgentsView {
        agents: agents
            .into_iter()
            .map(|agent| AgentView {
                id: agent.id,
                name: agent.name,
                mode: agent.mode,
                handles: agent.handles,
                enabled: agent.enabled,
            })
            .collect(),
        messages,
    })
}

/// Registers or updates an agent. An iMessage agent needs its handle; an effects-only
/// agent, such as Meta Muse, has no channel Picket can read. Every field is validated
/// here; the renderer only proposes values.
#[tauri::command]
pub fn register_agent(
    app: tauri::AppHandle,
    name: String,
    mode: String,
    handle: Option<String>,
    region: Option<String>,
) -> Result<(), String> {
    let region = region
        .map(|value| value.trim().to_ascii_uppercase())
        .filter(|value| !value.is_empty());
    let mode = match mode.as_str() {
        "messages" => AgentMode::Messages,
        "effects_only" => AgentMode::EffectsOnly,
        _ => return Err("unsupported agent mode".into()),
    };
    let handles = match (&mode, handle.as_deref().map(str::trim)) {
        (AgentMode::Messages, Some(handle)) => vec![
            agent::normalize_handle(handle, region.as_deref())
                .map_err(|error| error.to_string())?,
        ],
        (AgentMode::Messages, None) => return Err(agent::AgentError::Handles.to_string()),
        _ => Vec::new(),
    };
    let registration = AgentRegistration {
        id: agent::id_from_name(&name).map_err(|error| error.to_string())?,
        name: name.trim().to_owned(),
        mode,
        handles,
        connected_since: None,
        enabled: true,
        region,
    };
    agent::validate(&registration).map_err(|error| error.to_string())?;
    let now = Utc::now();
    let event = Event {
        schema_version: 1,
        key: EventKey {
            account: "local".into(),
            source: Source::Picket,
            resource: format!("agent/{}", registration.id),
            revision: now.timestamp_nanos_opt().unwrap_or_default().to_string(),
        },
        occurred_at: now,
        ingested_at: now,
        source_time_quality: None,
        adapter: None,
        payload: EventPayload::Agent(registration),
    };
    crate::ledger(&app)?
        .commit(
            Batch {
                events: vec![event],
                ..Batch::default()
            },
            now,
        )
        .map(|_| ())
        .map_err(|_| "agent could not be saved".to_string())
}

/// Starts Messages capture. It waits for a Google account (observations belong to the
/// monitored account) and a registered iMessage agent, and rechecks access each poll,
/// so granting Full Disk Access starts capture without a restart.
pub fn start(app: tauri::AppHandle) {
    let spawned = thread::Builder::new()
        .name("picket-messages".into())
        .spawn(move || run(&app));
    if spawned.is_err() {
        tracing::warn!("Messages capture could not start");
    }
}

fn run(app: &tauri::AppHandle) {
    let path = match app.path().home_dir() {
        Ok(home) => home.join("Library/Messages/chat.db"),
        Err(_) => return set(app, "unavailable"),
    };
    let mut ledger: Option<Writer> = None;
    let mut db: Option<ChatDb> = None;
    let mut extractor = Extractor::default();
    loop {
        let progress = poll(app, &path, &mut ledger, &mut db, &mut extractor);
        if progress != Some(Progress::Behind) {
            thread::sleep(POLL);
        }
    }
}

fn poll(
    app: &tauri::AppHandle,
    path: &std::path::Path,
    ledger: &mut Option<Writer>,
    db: &mut Option<ChatDb>,
    extractor: &mut Extractor,
) -> Option<Progress> {
    let account = app
        .state::<GoogleState>()
        .account
        .lock()
        .ok()
        .and_then(|account| account.clone());
    let Some(account) = account else {
        set(app, "waiting");
        return None;
    };
    if ledger.is_none() {
        *ledger = crate::ledger(app).ok();
    }
    let writer = ledger.as_mut()?;
    let agents: Vec<_> = writer
        .agents()
        .ok()?
        .into_iter()
        .filter(|agent| agent.enabled && agent.mode == AgentMode::Messages)
        .collect();
    if agents.is_empty() {
        *db = None;
        set(app, "inactive");
        return None;
    }
    let now = Utc::now();
    if db.is_none() {
        match ChatDb::open(path) {
            Ok(opened) => *db = Some(opened),
            Err(error) => {
                unavailable(app, writer, &account, &agents, &error);
                return None;
            }
        }
    }
    let chat = db.as_ref()?;
    match messages::poll_messages(chat, writer, &account, &agents, now) {
        Ok(progress) => {
            set(app, "capturing");
            extract(app, chat, writer, &account, extractor);
            Some(progress)
        }
        Err(error) => {
            // Reopen next time; access or the schema may have changed.
            *db = None;
            unavailable(app, writer, &account, &agents, &error);
            None
        }
    }
}

/// Extracts captured messages. Text is read from Messages for each call and dropped.
fn extract(
    app: &tauri::AppHandle,
    chat: &ChatDb,
    writer: &mut Writer,
    account: &str,
    extractor: &mut Extractor,
) {
    let pending = writer
        .pending_extractions(account, 1)
        .is_ok_and(|pending| !pending.is_empty());
    if pending && extractor.runner.is_none() && !extractor.failed {
        extractor.runner = load(app);
        extractor.failed =
            extractor.runner.is_none() && crate::model::path(app).is_some_and(|path| path.exists());
    }
    let zone = |at: chrono::DateTime<Utc>| Local.offset_from_utc_datetime(&at.naive_utc()).fix();
    let mut text = |guid: &str| chat.text(guid);
    let model = extractor
        .runner
        .as_mut()
        .map(|runner| runner as &mut dyn LocalModel);
    if extract_pending(writer, model, &mut text, &zone, account, Utc::now()).is_err() {
        tracing::warn!("extraction pass failed");
    }
    if pending {
        extractor.used = Some(Instant::now());
    } else if extractor
        .used
        .is_some_and(|used| used.elapsed() > MODEL_IDLE)
    {
        extractor.runner = None;
        extractor.used = None;
    }
}

/// Loads and verifies the installed model, if the user downloaded one.
fn load(app: &tauri::AppHandle) -> Option<picket_model::Runner> {
    let path = crate::model::path(app)?;
    if !path.exists() {
        return None;
    }
    match picket_model::Runner::load(&path, crate::model::spec()?) {
        Ok(runner) => Some(runner),
        Err(error) => {
            tracing::warn!(%error, "local model unavailable");
            None
        }
    }
}

fn unavailable(
    app: &tauri::AppHandle,
    writer: &mut Writer,
    account: &str,
    agents: &[AgentRegistration],
    error: &MessagesError,
) {
    set(
        app,
        match error {
            MessagesError::PermissionMissing => "permission_missing",
            MessagesError::Unsupported => "unsupported",
            MessagesError::Read | MessagesError::Store => "unavailable",
        },
    );
    if messages::record_unavailable(writer, account, agents, error, Utc::now()).is_err() {
        tracing::warn!("Messages coverage could not be recorded");
    }
}

fn set(app: &tauri::AppHandle, status: &'static str) {
    if let Ok(mut current) = app.state::<MessagesState>().0.lock() {
        *current = status;
    }
}
