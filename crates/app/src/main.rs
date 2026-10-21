use tauri::{
    Manager,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tracing_subscriber::EnvFilter;

/// Exposes the portable, read-only diagnostic through the Tauri boundary.
#[tauri::command]
fn get_runtime_info() -> picket_core::RuntimeInfo {
    tracing::debug!("Runtime diagnostic requested through IPC");
    picket_core::runtime_info()
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

    tauri::Builder::default()
        // A second launch restores the existing process instead of duplicating it.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Err(error) = show_window(app) {
                tracing::error!(%error, "failed to show existing window");
            }
        }))
        .invoke_handler(tauri::generate_handler![get_runtime_info])
        .on_page_load(|_, _| tracing::debug!("App webview loaded"))
        .setup(|app| {
            // Left click restores the window; the context menu also supports quit.
            let open = MenuItem::with_id(app, "open", "Open Picket", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Picket", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
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
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            tracing::info!(
                version = env!("CARGO_PKG_VERSION"),
                "Picket started; monitoring is not implemented"
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
