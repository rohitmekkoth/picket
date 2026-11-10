/// Picket's own Google OAuth client (a Desktop client; Google treats its secret as
/// non-confidential). It comes from the environment, or from the git-ignored
/// `local/google-client.env`, and is never committed or printed. Without it the app
/// offers only the bring-your-own-client sign-in.
fn google_client() {
    const KEYS: [&str; 2] = ["PICKET_GOOGLE_CLIENT_ID", "PICKET_GOOGLE_CLIENT_SECRET"];
    let file =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../local/google-client.env");
    println!("cargo:rerun-if-changed={}", file.display());
    let contents = std::fs::read_to_string(&file).unwrap_or_default();
    for key in KEYS {
        println!("cargo:rerun-if-env-changed={key}");
        let from_file = contents.lines().find_map(|line| {
            line.trim()
                .strip_prefix(key)
                .and_then(|rest| rest.strip_prefix('='))
                .map(|value| value.trim().to_owned())
        });
        let value = std::env::var(key).ok().or(from_file).unwrap_or_default();
        if !value.is_empty() && !value.chars().any(char::is_control) {
            println!("cargo:rustc-env={key}={value}");
        }
    }
}

fn main() {
    google_client();
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
            "sign_in_google",
            "complete_setup",
        ]),
    ))
    .unwrap_or_else(|error| panic!("failed to build Picket app metadata: {error}"));
}
