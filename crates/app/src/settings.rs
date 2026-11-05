//! Settings, log verification and exports. Every value is validated in Rust; files are
//! written only by Rust, to a place the person picks in a native save panel, so the
//! webview needs no filesystem permission (plan decision 6).

use std::{fs::File, io::BufWriter, path::PathBuf};

use chrono::{Local, Timelike, Utc};
use picket_core::{
    export,
    settings::{Settings, SettingsPatch},
};
use serde::Serialize;
use tauri::Manager;
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_dialog::DialogExt;

use crate::alerts::{self, AlertState};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    #[serde(flatten)]
    settings: Settings,
    /// Read from macOS each time, so it matches System Settings > Login Items.
    start_at_login: bool,
    /// This build can sign in with Picket's own Google client.
    picket_client: bool,
}

fn view(app: &tauri::AppHandle, settings: Settings) -> SettingsView {
    SettingsView {
        settings,
        start_at_login: app.autolaunch().is_enabled().unwrap_or(false),
        picket_client: crate::google_auth::picket_client().is_some(),
    }
}

#[tauri::command]
pub fn get_settings(app: tauri::AppHandle) -> Result<SettingsView, String> {
    let settings = crate::ledger(&app)?
        .read(|store| store.settings())
        .map_err(|_| "settings unavailable".to_string())?;
    Ok(view(&app, settings))
}

/// Applies a change from the Settings screen. Moving the summary to a time that has
/// already passed today counts today as covered, so the change itself posts nothing.
#[tauri::command]
pub fn update_settings(
    app: tauri::AppHandle,
    patch: SettingsPatch,
) -> Result<SettingsView, String> {
    let now = Utc::now();
    let local = now.with_timezone(&Local);
    let settings = crate::ledger(&app)?
        .read(move |store| {
            let previous = store.settings()?.summary_minutes;
            let next = store.update_settings(&patch, now)?;
            let minutes_now = local.hour() * 60 + local.minute();
            if next.summary_minutes != previous
                && next
                    .summary_minutes
                    .is_some_and(|minutes| u32::from(minutes) <= minutes_now)
            {
                store.set_last_summary_day(local.date_naive())?;
            }
            store.settings()
        })
        .map_err(|_| "settings could not be saved".to_string())?;
    app.state::<AlertState>().wake();
    Ok(view(&app, settings))
}

/// Turns start at login on or off. Picket then launches in the background at login.
#[tauri::command]
pub fn set_start_at_login(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    let launcher = app.autolaunch();
    let result = if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    };
    result.map_err(|_| "login item could not be changed".to_string())?;
    launcher
        .is_enabled()
        .map_err(|_| "login item unavailable".to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyView {
    valid: bool,
    through_sequence: Option<i64>,
    retained_events: Option<u64>,
    broken_at: Option<i64>,
}

/// Checks the hash chain of the whole retained log.
#[tauri::command]
pub fn verify_log(app: tauri::AppHandle) -> Result<VerifyView, String> {
    let result = crate::ledger(&app)?
        .read(|store| Ok(store.verify()))
        .map_err(|_| "log unavailable".to_string())?;
    Ok(match result {
        Ok(verification) => VerifyView {
            valid: true,
            through_sequence: Some(verification.through_sequence),
            retained_events: Some(verification.retained_events),
            broken_at: None,
        },
        Err(picket_core::store::StoreError::Corrupt { sequence, .. }) => VerifyView {
            valid: false,
            through_sequence: None,
            retained_events: None,
            broken_at: Some(sequence),
        },
        Err(_) => return Err("log could not be checked".into()),
    })
}

/// Saves one incident as a Markdown report. Returns false when the person cancels.
#[tauri::command]
pub async fn export_incident(app: tauri::AppHandle, id: String) -> Result<bool, String> {
    let id = alerts::incident_id(&id)?.to_owned();
    let detail = crate::ledger(&app)?
        .read(move |store| store.incident(&id))
        .map_err(|_| "incident unavailable".to_string())?
        .ok_or_else(|| "incident not found".to_string())?;
    let Some(path) = choose(&app, "Picket incident.md", "Markdown", &["md"]).await? else {
        return Ok(false);
    };
    let report = export::incident_markdown(&detail, Utc::now(), &Local, env!("CARGO_PKG_VERSION"));
    std::fs::write(path, report).map_err(|_| "report could not be saved".to_string())?;
    Ok(true)
}

/// Saves everything Picket keeps as one JSON file. Returns false when the person cancels.
#[tauri::command]
pub async fn export_all(app: tauri::AppHandle) -> Result<bool, String> {
    let Some(path) = choose(&app, "Picket export.json", "JSON", &["json"]).await? else {
        return Ok(false);
    };
    let file = File::create(&path).map_err(|_| "export could not be saved".to_string())?;
    crate::ledger(&app)?
        .read(move |store| {
            export::write_all(
                store,
                &mut BufWriter::new(file),
                Utc::now(),
                env!("CARGO_PKG_VERSION"),
            )
        })
        .map_err(|_| "export could not be saved".to_string())?;
    Ok(true)
}

async fn choose(
    app: &tauri::AppHandle,
    name: &str,
    kind: &str,
    extensions: &'static [&'static str],
) -> Result<Option<PathBuf>, String> {
    let dialog = app
        .dialog()
        .file()
        .set_file_name(name)
        .add_filter(kind, extensions);
    let chosen = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file())
        .await
        .map_err(|_| "save panel failed".to_string())?;
    chosen
        .map(|path| {
            path.into_path()
                .map_err(|_| "unsupported location".to_string())
        })
        .transpose()
}

/// Records that the setup wizard was finished or closed.
#[tauri::command]
pub fn complete_setup(app: tauri::AppHandle) -> Result<SettingsView, String> {
    let settings = crate::ledger(&app)?
        .read(|store| {
            store.set_setup_completed(Utc::now())?;
            store.settings()
        })
        .map_err(|_| "setup could not be saved".to_string())?;
    Ok(view(&app, settings))
}
