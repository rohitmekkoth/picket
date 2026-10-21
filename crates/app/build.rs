fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&["get_runtime_info"])),
    )
    .unwrap_or_else(|error| panic!("failed to build Picket app metadata: {error}"));
}
