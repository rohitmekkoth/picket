//! Rule commands. The renderer proposes levels and boundaries; Rust validates them,
//! records each change as one rule-set version, and fills in the Mac's time zone.

use chrono::Utc;
use picket_core::{
    event::{BoundaryKind, Level},
    views::{PreviewView, RulesView},
};

#[tauri::command]
pub fn get_rules(app: tauri::AppHandle) -> Result<RulesView, String> {
    crate::ledger(&app)?
        .read(|store| store.rules(Utc::now()))
        .map_err(|_| "rules unavailable".to_string())
}

#[tauri::command]
pub fn set_rule_level(app: tauri::AppHandle, rule: String, level: Level) -> Result<(), String> {
    crate::ledger(&app)?
        .read(move |store| store.set_rule_level(&rule, level, Utc::now()))
        .map_err(|_| "rule could not be saved".to_string())
}

#[tauri::command]
pub fn preview_boundary(app: tauri::AppHandle, kind: BoundaryKind) -> Result<PreviewView, String> {
    let kind = with_local_zone(kind)?;
    crate::ledger(&app)?
        .read(move |store| store.preview_boundary(kind, Utc::now()))
        .map_err(|_| "check the boundary's details".to_string())
}

#[tauri::command]
pub fn save_boundary(
    app: tauri::AppHandle,
    id: Option<String>,
    level: Level,
    kind: BoundaryKind,
) -> Result<String, String> {
    if level == Level::Off {
        return Err("a boundary needs a level other than Off".into());
    }
    let kind = with_local_zone(kind)?;
    crate::ledger(&app)?
        .read(move |store| store.save_boundary(id.as_deref(), level, kind, Utc::now()))
        .map_err(|_| "check the boundary's details".to_string())
}

#[tauri::command]
pub fn delete_boundary(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let removed = crate::ledger(&app)?
        .read(move |store| store.delete_boundary(&id, Utc::now()))
        .map_err(|_| "boundary could not be removed".to_string())?;
    if removed {
        Ok(())
    } else {
        Err("boundary not found".into())
    }
}

/// Quiet hours follow this Mac's clock. The zone is recorded with the rule, so replay
/// gives the same answer after the user travels or the zone's rules change.
fn with_local_zone(kind: BoundaryKind) -> Result<BoundaryKind, String> {
    match kind {
        BoundaryKind::QuietHours {
            start_minute,
            end_minute,
            days,
            zone,
            actions,
        } => Ok(BoundaryKind::QuietHours {
            start_minute,
            end_minute,
            days,
            zone: if zone.is_empty() {
                iana_time_zone::get_timezone().map_err(|_| "time zone unavailable".to_string())?
            } else {
                zone
            },
            actions,
        }),
        other => Ok(other),
    }
}
