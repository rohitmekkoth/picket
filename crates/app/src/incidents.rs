//! Incident review commands. Views and decisions run on the store writer, against the
//! evidence the engine assessed; the renderer only proposes an incident id and choice.

use chrono::Utc;
use picket_core::{
    event::{Event, EventKey, EventPayload, Source},
    ledger::Ledger,
    store::Batch,
    views::{IncidentDetail, IncidentSummary},
};

const MAX_ID_BYTES: usize = 512;

#[tauri::command]
pub fn list_incidents(app: tauri::AppHandle) -> Result<Vec<IncidentSummary>, String> {
    crate::ledger(&app)?
        .read(|store| store.incidents())
        .map_err(|_| "incidents unavailable".to_string())
}

#[tauri::command]
pub fn get_incident(app: tauri::AppHandle, id: String) -> Result<IncidentDetail, String> {
    let id = incident_id(id)?;
    crate::ledger(&app)?
        .read(move |store| store.incident(&id))
        .map_err(|_| "incident unavailable".to_string())?
        .ok_or_else(|| "incident not found".to_string())
}

/// Records "This was fine", "This wasn't OK" or an undo for every finding of one
/// incident. Picket's rules and actor evidence are not changed by it.
#[tauri::command]
pub fn resolve_incident(app: tauri::AppHandle, id: String, decision: String) -> Result<(), String> {
    let id = incident_id(id)?;
    let expected = match decision.as_str() {
        "fine" => Some(true),
        "not_ok" => Some(false),
        "undo" => None,
        _ => return Err("unsupported decision".into()),
    };
    let found = crate::ledger(&app)?
        .read(move |store| store.decide(&id, expected, Utc::now()))
        .map_err(|_| "decision could not be saved".to_string())?;
    if found {
        Ok(())
    } else {
        Err("incident not found".into())
    }
}

/// Marks an agent revoked from now on, or lifts the mark. Picket records the time; it
/// does not revoke or check the agent's access to Google.
#[tauri::command]
pub fn set_agent_revoked(app: tauri::AppHandle, id: String, revoked: bool) -> Result<(), String> {
    let mut ledger = crate::ledger(&app)?;
    let mut registration = ledger
        .agents()
        .map_err(|_| "agents unavailable".to_string())?
        .into_iter()
        .find(|agent| agent.id == id)
        .ok_or_else(|| "agent not found".to_string())?;
    let now = Utc::now();
    registration.revoked_at = revoked.then_some(now);
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
    ledger
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

fn incident_id(id: String) -> Result<String, String> {
    let valid = id.starts_with("incident/")
        && id.len() <= MAX_ID_BYTES
        && !id.chars().any(char::is_control);
    if valid {
        Ok(id)
    } else {
        Err("invalid incident id".into())
    }
}
