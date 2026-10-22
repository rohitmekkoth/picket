use tauri::{
    Manager,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
#[cfg(target_os = "macos")]
mod google_auth;
#[cfg(target_os = "macos")]
mod google_collector;
#[cfg(target_os = "macos")]
mod google_http;
#[cfg(target_os = "macos")]
mod google_runtime;
#[cfg(target_os = "macos")]
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use tauri_plugin_notification::{Action, ActionType, NotificationExt};
use tracing_subscriber::EnvFilter;

const PROBE_TITLE: &str = "Picket notification test";

#[cfg(target_os = "macos")]
struct GoogleState {
    session: Mutex<Option<google_auth::Session>>,
    worker_started: AtomicBool,
    status: Mutex<&'static str>,
}

#[cfg(target_os = "macos")]
impl Default for GoogleState {
    fn default() -> Self {
        Self {
            session: Mutex::new(None),
            worker_started: AtomicBool::new(false),
            status: Mutex::new("disconnected"),
        }
    }
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

fn send_notification_probe(app: &tauri::AppHandle) -> tauri_plugin_notification::Result<()> {
    app.notification()
        .builder()
        .title(PROBE_TITLE)
        .body("Open this test alert or choose an action.")
        .action_type_id("picket-probe")
        .show()
}

fn handle_notification_probe(app: &tauri::AppHandle, action: &str) -> tauri::Result<()> {
    let route = match action {
        "tap" | "review" => "#/notification-test/review",
        "expected" => "#/notification-test/expected",
        _ => return Ok(()),
    };
    show_window(app)?;
    if let Some(window) = app.get_webview_window("main") {
        window.eval(format!("window.location.hash = '{route}'"))?;
    }
    Ok(())
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

/// Builds a monochrome template icon without loading a runtime asset.
fn tray_image() -> tauri::image::Image<'static> {
    let mut rgba = vec![0; 18 * 18 * 4];
    for y in 3..15 {
        for x in 3..15 {
            if (4..7).contains(&x) || (11..14).contains(&x) || (8..11).contains(&y) {
                rgba[(y * 18 + x) * 4 + 3] = 255;
            }
        }
    }
    tauri::image::Image::new_owned(rgba, 18, 18)
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
        .plugin(tauri_plugin_notification::init());
    #[cfg(target_os = "macos")]
    let builder = builder.invoke_handler(tauri::generate_handler![
        get_runtime_info,
        google_connected,
        connect_google
    ]);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.invoke_handler(tauri::generate_handler![get_runtime_info]);
    builder
        .on_page_load(|_, _| tracing::debug!("App webview loaded"))
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                app.manage(GoogleState::default());
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
            app.notification().register_action_types(vec![
                ActionType::builder("picket-probe")
                    .actions(vec![
                        Action::builder("review", "Review").build(),
                        Action::builder("expected", "This was expected").build(),
                    ])
                    .build(),
            ])?;
            let app_handle = app.handle().clone();
            app.notification().on_action(move |event| {
                if event.notification().and_then(|item| item.title()) != Some(PROBE_TITLE) {
                    return;
                }
                tracing::info!(
                    action = event.action_id(),
                    "notification test action received"
                );
                if let Err(error) = handle_notification_probe(&app_handle, event.action_id()) {
                    tracing::error!(%error, "failed to handle notification test action");
                }
            })?;
            // Left click restores the window; the context menu also supports quit.
            let open = MenuItem::with_id(app, "open", "Open Picket", true, None::<&str>)?;
            let test_notification = MenuItem::with_id(
                app,
                "test-notification",
                "Send test notification",
                true,
                None::<&str>,
            )?;
            let quit = MenuItem::with_id(app, "quit", "Quit Picket", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &test_notification, &quit])?;
            TrayIconBuilder::with_id("picket")
                .icon(tray_image())
                .icon_as_template(true)
                .tooltip("Picket - development foundation")
                .menu(&menu)
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
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => {
                        if let Err(error) = show_window(app) {
                            tracing::error!(%error, "failed to open window");
                        }
                    }
                    "test-notification" => {
                        if let Err(error) = send_notification_probe(app) {
                            tracing::error!(%error, "failed to send notification test");
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            if std::env::var_os("PICKET_NOTIFICATION_PROBE").is_some() {
                if let Some(window) = app.get_webview_window("main") {
                    window.hide()?;
                }
                #[cfg(target_os = "macos")]
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
                send_notification_probe(app.handle())?;
            }
            tracing::info!(
                version = env!("CARGO_PKG_VERSION"),
                "Picket started"
            );
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
