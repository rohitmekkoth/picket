use tauri::{
    Manager,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
#[cfg(target_os = "macos")]
mod activity;
#[cfg(target_os = "macos")]
mod agents;
#[cfg(target_os = "macos")]
mod alerts;
#[cfg(target_os = "macos")]
mod google_auth;
#[cfg(target_os = "macos")]
mod google_http;
#[cfg(target_os = "macos")]
mod google_runtime;
#[cfg(target_os = "macos")]
mod incidents;
#[cfg(target_os = "macos")]
mod links;
#[cfg(target_os = "macos")]
mod model;
#[cfg(target_os = "macos")]
mod rules;
#[cfg(target_os = "macos")]
mod settings;
mod tray;
#[cfg(target_os = "macos")]
mod wipe;
#[cfg(target_os = "macos")]
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use tracing_subscriber::EnvFilter;

const BACKGROUND_ARG: &str = "--background";

#[cfg(target_os = "macos")]
struct GoogleState {
    session: Mutex<Option<google_auth::Session>>,
    /// The monitored account's local id, once a worker holds its session.
    account: Mutex<Option<String>>,
    worker_started: AtomicBool,
    status: Mutex<&'static str>,
}

#[cfg(target_os = "macos")]
impl Default for GoogleState {
    fn default() -> Self {
        Self {
            session: Mutex::new(None),
            account: Mutex::new(None),
            worker_started: AtomicBool::new(false),
            status: Mutex::new("disconnected"),
        }
    }
}

/// The single store writer, started on first use and shared by every collector thread.
#[cfg(target_os = "macos")]
#[derive(Default)]
struct LedgerState(Mutex<Option<picket_core::ledger::Writer>>);

/// A writer handle for one collector thread.
#[cfg(target_os = "macos")]
fn ledger(app: &tauri::AppHandle) -> Result<picket_core::ledger::Writer, &'static str> {
    if wipe::wiping() {
        return Err("Picket is being wiped");
    }
    let state = app.state::<LedgerState>();
    let mut writer = state.0.lock().map_err(|_| "event store state failed")?;
    if writer.is_none() {
        let directory = app.path().app_data_dir().map_err(|_| "data path failed")?;
        std::fs::create_dir_all(&directory).map_err(|_| "data directory failed")?;
        // The writer thread runs for the app's lifetime because this handle is kept.
        let (started, _thread) = picket_core::ledger::Writer::start(&directory.join("picket.db"))
            .map_err(|_| "event store failed")?;
        *writer = Some(started);
    }
    writer
        .as_ref()
        .ok_or("event store failed")?
        .connect()
        .map_err(|_| "event store failed")
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn google_connected(state: tauri::State<'_, GoogleState>) -> String {
    state
        .status
        .lock()
        .map_or("unavailable", |status| *status)
        .into()
}

#[cfg(target_os = "macos")]
#[tauri::command]
async fn connect_google(
    app: tauri::AppHandle,
    state: tauri::State<'_, GoogleState>,
    client_id: String,
    client_secret: String,
) -> Result<(), String> {
    let session = tauri::async_runtime::spawn_blocking(move || {
        google_auth::connect(&client_id, &client_secret)
    })
    .await
    .map_err(|_| "Google connection task failed".to_string())?
    .map_err(|error| error.to_string())?;
    let mut current = state
        .session
        .lock()
        .map_err(|_| "Google connection state unavailable".to_string())?;
    *current = Some(session);
    drop(current);
    google_runtime::start_worker(app);
    Ok(())
}

/// Signs in with Picket's own Google client: one click, no Cloud Console.
#[cfg(target_os = "macos")]
#[tauri::command]
async fn sign_in_google(
    app: tauri::AppHandle,
    state: tauri::State<'_, GoogleState>,
) -> Result<(), String> {
    let (client_id, client_secret) = google_auth::picket_client()
        .ok_or_else(|| "This build has no Picket Google client; use your own".to_string())?;
    connect_google(app, state, client_id.into(), client_secret.into()).await
}

/// Signs in again with the saved client after Google rejected the grant.
#[cfg(target_os = "macos")]
#[tauri::command]
async fn reconnect_google(
    app: tauri::AppHandle,
    state: tauri::State<'_, GoogleState>,
) -> Result<(), String> {
    let (client_id, client_secret) =
        tauri::async_runtime::spawn_blocking(google_auth::stored_client)
            .await
            .map_err(|_| "Google connection task failed".to_string())?
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "No saved Google connection".to_string())?;
    connect_google(app, state, client_id, client_secret).await
}

/// Exposes the portable, read-only diagnostic through the Tauri boundary.
#[tauri::command]
fn get_runtime_info(app: tauri::AppHandle) -> picket_core::RuntimeInfo {
    tracing::debug!("Runtime diagnostic requested through IPC");
    let mut info = picket_core::runtime_info();
    #[cfg(target_os = "macos")]
    {
        info.monitoring = app
            .state::<GoogleState>()
            .status
            .lock()
            .is_ok_and(|status| *status == "monitoring");
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
    info
}

/// Restores the primary window and its normal macOS application presence.
fn show_window(app: &tauri::AppHandle) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window("main") else {
        tracing::error!("main window is unavailable");
        return Ok(());
    };
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Regular)?;
    window.show()?;
    window.unminimize()?;
    window.set_focus()?;
    Ok(())
}

fn on_menu(app: &tauri::AppHandle, id: &str) {
    match id {
        "open" => {
            if let Err(error) = show_window(app) {
                tracing::error!(%error, "failed to open window");
            }
        }
        #[cfg(target_os = "macos")]
        "pause" | "resume" => {
            if let Err(reason) = app
                .state::<alerts::AlertState>()
                .set_paused(app, id == "pause")
            {
                tracing::warn!(reason, "failed to pause alerts");
            }
        }
        #[cfg(target_os = "macos")]
        "summary" => {
            if let Err(reason) = alerts::open(app, "#/overview") {
                tracing::warn!(reason, "failed to open the summary");
            }
        }
        "quit" => app.exit(0),
        #[cfg(target_os = "macos")]
        incident if incident.starts_with("incident/") => {
            if let Err(reason) = alerts::open_incident(app, incident) {
                tracing::warn!(reason, "failed to open incident from menu");
            }
        }
        _ => {}
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Respect RUST_LOG while keeping normal startup output concise.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("picket_app=info")),
        )
        .with_target(true)
        .try_init()
        .map_err(|error| format!("could not initialize diagnostics: {error}"))?;

    let builder = tauri::Builder::default()
        // A second launch restores the existing process instead of duplicating it.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Err(error) = show_window(app) {
                tracing::error!(%error, "failed to show existing window");
            }
        }))
        .plugin(tauri_plugin_notification::init())
        // Rust-only: the webview is granted no dialog or autostart permission.
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![BACKGROUND_ARG]),
        ));
    #[cfg(target_os = "macos")]
    let builder = builder.invoke_handler(tauri::generate_handler![
        get_runtime_info,
        google_connected,
        connect_google,
        reconnect_google,
        sign_in_google,
        agents::list_agents,
        agents::register_agent,
        activity::get_overview,
        activity::list_activity,
        incidents::list_incidents,
        incidents::get_incident,
        incidents::resolve_incident,
        incidents::set_agent_revoked,
        links::open_link,
        rules::get_rules,
        rules::set_rule_level,
        rules::preview_boundary,
        rules::save_boundary,
        rules::delete_boundary,
        model::model_status,
        model::download_model,
        settings::get_settings,
        settings::update_settings,
        settings::set_start_at_login,
        settings::verify_log,
        settings::export_incident,
        settings::export_all,
        settings::complete_setup,
        wipe::wipe_all
    ]);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.invoke_handler(tauri::generate_handler![get_runtime_info]);
    builder
        .on_page_load(|_, _| tracing::debug!("App webview loaded"))
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                app.manage(GoogleState::default());
                app.manage(LedgerState::default());
                app.manage(agents::MessagesState::default());
                app.manage(model::ModelState::default());
                app.manage(alerts::AlertState::default());
                agents::start(app.handle().clone());
                let restore_handle = app.handle().clone();
                tauri::async_runtime::spawn_blocking(move || {
                    loop {
                        let state = restore_handle.state::<GoogleState>();
                        if state.worker_started.load(Ordering::SeqCst)
                            || state.session.lock().is_ok_and(|current| current.is_some())
                        {
                            break;
                        }
                        match google_auth::restore() {
                            Ok(Some(session)) => {
                                let Ok(mut current) = state.session.lock() else {
                                    break;
                                };
                                if current.is_none() && !state.worker_started.load(Ordering::SeqCst)
                                {
                                    *current = Some(session);
                                    drop(current);
                                    google_runtime::start_worker(restore_handle.clone());
                                }
                                break;
                            }
                            Ok(None) => break,
                            Err(google_auth::AuthError::Authorization) => {
                                tracing::warn!("Google rejected the saved sign-in");
                                if let Ok(mut status) = state.status.lock() {
                                    *status = "reconnect_required";
                                }
                                break;
                            }
                            Err(error) => {
                                tracing::warn!(%error, "Google connection unavailable");
                                if let Ok(mut status) = state.status.lock() {
                                    *status = "unavailable";
                                }
                                thread::sleep(Duration::from_secs(30));
                            }
                        }
                    }
                });
            }
            // Left click restores the window; the menu carries status and actions.
            let initial = tray::initial();
            TrayIconBuilder::with_id(tray::TRAY_ID)
                .icon(tray::icon(initial.state))
                .icon_as_template(true)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                        && let Err(error) = show_window(tray.app_handle())
                    {
                        tracing::error!(%error, "failed to open window from tray");
                    }
                })
                .on_menu_event(|app, event| on_menu(app, event.id.as_ref()))
                .build(app)?;
            tray::apply(app.handle(), &initial)?;
            #[cfg(target_os = "macos")]
            alerts::start(app.handle())?;
            // A background launch, such as at login, stays in the menu bar; macOS shows
            // banners only for apps that are not frontmost.
            // Until setup is finished, Picket opens its window even when started in the
            // background, so a first run is never silent.
            #[cfg(target_os = "macos")]
            let setup_done = ledger(app.handle())
                .and_then(|writer| {
                    writer
                        .read(|store| store.settings())
                        .map_err(|_| "settings unavailable")
                })
                .is_ok_and(|settings| settings.setup_completed);
            #[cfg(not(target_os = "macos"))]
            let setup_done = true;
            if setup_done && std::env::args().any(|arg| arg == BACKGROUND_ARG) {
                #[cfg(target_os = "macos")]
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            } else {
                show_window(app.handle())?;
            }
            tracing::info!(version = env!("CARGO_PKG_VERSION"), "Picket started");
            Ok(())
        })
        .on_window_event(|window, event| {
            // Only the primary window adopts the close-to-tray lifecycle.
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                match window.hide() {
                    Ok(()) => {
                        api.prevent_close();
                        #[cfg(target_os = "macos")]
                        if let Err(error) = window
                            .app_handle()
                            .set_activation_policy(tauri::ActivationPolicy::Accessory)
                        {
                            tracing::error!(%error, "failed to hide Dock icon");
                        }
                    }
                    Err(error) => tracing::error!(%error, "failed to hide window"),
                }
            }
        })
        .run(tauri::generate_context!())?;
    Ok(())
}
