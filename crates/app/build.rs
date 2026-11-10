fn main() {
    // Generate the narrow capability for commands exposed to the webview.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "get_runtime_info",
            "google_connected",
            "connect_google",
            "list_agents",
            "register_agent",
            "get_overview",
            "list_activity",
            "list_incidents",
            "get_incident",
            "resolve_incident",
            "set_agent_revoked",
            "open_link",
            "get_rules",
            "set_rule_level",
            "preview_boundary",
            "save_boundary",
            "delete_boundary",
            "model_status",
            "download_model",
            "reconnect_google",
            "get_settings",
            "update_settings",
            "set_start_at_login",
            "verify_log",
            "export_incident",
            "export_all",
            "wipe_all",
        ]),
    ))
    .unwrap_or_else(|error| panic!("failed to build Picket app metadata: {error}"));
}
