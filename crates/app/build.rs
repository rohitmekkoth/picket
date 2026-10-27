fn main() {
    // Generate the narrow capability for commands exposed to the webview.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "get_runtime_info",
            "google_connected",
            "connect_google",
            "list_agents",
            "register_agent",
        ]),
    ))
    .unwrap_or_else(|error| panic!("failed to build Picket app metadata: {error}"));
}
