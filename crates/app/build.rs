/// Picket's own Google OAuth client (a Desktop client; Google treats its secret as
/// non-confidential). It comes from the environment, or from the git-ignored
/// `local/google-client.env`, and is never committed or printed. Without it the app
/// offers only the bring-your-own-client sign-in.
///
/// Values are trimmed from both sources: v1.0.0 shipped a client ID with a leading
/// space from a CI secret, which Google rejects as "OAuth client was not found". A
/// value that is still malformed after trimming fails the build instead of shipping a
/// sign-in that cannot work, and `PICKET_REQUIRE_GOOGLE_CLIENT=1` (set by the release
/// workflow) also fails a build that has no client at all.
fn google_client() -> Result<(), String> {
    const ID: &str = "PICKET_GOOGLE_CLIENT_ID";
    const SECRET: &str = "PICKET_GOOGLE_CLIENT_SECRET";
    const REQUIRE: &str = "PICKET_REQUIRE_GOOGLE_CLIENT";
    let file =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../local/google-client.env");
    println!("cargo:rerun-if-changed={}", file.display());
    println!("cargo:rerun-if-env-changed={REQUIRE}");
    let contents = std::fs::read_to_string(&file).unwrap_or_default();
    let read = |key: &str| {
        println!("cargo:rerun-if-env-changed={key}");
        let from_file = contents.lines().find_map(|line| {
            line.trim()
                .strip_prefix(key)
                .and_then(|rest| rest.strip_prefix('='))
                .map(str::to_owned)
        });
        let value = std::env::var(key).ok().or(from_file).unwrap_or_default();
        value.trim().trim_matches(['"', '\'']).trim().to_owned()
    };
    let id = read(ID);
    let secret = read(SECRET);
    if id.is_empty() {
        if std::env::var(REQUIRE).is_ok_and(|value| value == "1") {
            return Err(format!("{REQUIRE}=1 but {ID} is not set"));
        }
        return Ok(());
    }
    if !valid_client_id(&id) {
        return Err(format!(
            "{ID} is not a Google OAuth client ID (<digits>-<letters and digits>.apps.googleusercontent.com)"
        ));
    }
    if secret.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!("{SECRET} contains whitespace"));
    }
    println!("cargo:rustc-env={ID}={id}");
    if !secret.is_empty() {
        println!("cargo:rustc-env={SECRET}={secret}");
    }
    Ok(())
}

fn valid_client_id(id: &str) -> bool {
    let Some((project, rest)) = id.split_once('-') else {
        return false;
    };
    let Some(name) = rest.strip_suffix(".apps.googleusercontent.com") else {
        return false;
    };
    !project.is_empty()
        && project.bytes().all(|b| b.is_ascii_digit())
        && !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

fn main() {
    if let Err(error) = google_client() {
        // Never print the values themselves; only which one is wrong.
        println!("cargo::error={error}");
        return;
    }
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
