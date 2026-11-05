//! Uninstall and wipe (plan decision 7). Deletes everything Picket keeps on this Mac:
//! the database with its WAL files, the model, settings, caches and web storage, the
//! Google connection in the Keychain and the login item. It then opens Google's
//! third-party access page, so the person can remove Picket's grant, and quits.
//! `scripts/verify-wipe.sh` checks each of these is gone. macOS privacy grants (Full
//! Disk Access, notifications) cannot be removed by an app; the UI says so.

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
};

use tauri::Manager;
use tauri_plugin_autostart::ManagerExt as _;

use crate::{google_auth, links};

/// Set once a wipe starts. The store refuses to open again, so no thread can recreate
/// the database while files are being deleted.
static WIPING: AtomicBool = AtomicBool::new(false);

/// The binary's name, which unbundled development runs use instead of the identifier.
const DEV_NAME: &str = "picket-app";

pub fn wiping() -> bool {
    WIPING.load(Ordering::SeqCst)
}

/// Every directory and file Picket creates for this user, whether or not it exists.
fn paths(app: &tauri::AppHandle) -> Result<Vec<PathBuf>, String> {
    let resolver = app.path();
    let home = resolver.home_dir().map_err(|_| "home folder unavailable")?;
    let library = home.join("Library");
    let identifier = app.config().identifier.clone();
    let mut paths = vec![
        resolver
            .app_data_dir()
            .map_err(|_| "data folder unavailable")?,
        library.join("LaunchAgents/Picket.plist"),
        library.join(format!("Saved Application State/{identifier}.savedState")),
        library.join(format!("HTTPStorages/{identifier}")),
    ];
    for name in [identifier.as_str(), DEV_NAME] {
        paths.push(library.join("Caches").join(name));
        paths.push(library.join("WebKit").join(name));
    }
    Ok(paths)
}

/// The preference files the running process may write again as it quits; they are
/// removed by a short-lived helper after Picket has exited.
fn late_paths(app: &tauri::AppHandle) -> Result<Vec<PathBuf>, String> {
    let home = app
        .path()
        .home_dir()
        .map_err(|_| "home folder unavailable")?;
    let identifier = &app.config().identifier;
    let mut paths = vec![home.join(format!("Library/Preferences/{identifier}.plist"))];
    // Web storage can be flushed on exit too, so it is swept again.
    for name in [identifier.as_str(), DEV_NAME] {
        paths.push(home.join("Library/Caches").join(name));
        paths.push(home.join("Library/WebKit").join(name));
    }
    Ok(paths)
}

#[tauri::command]
pub fn wipe_all(app: tauri::AppHandle) -> Result<(), String> {
    WIPING.store(true, Ordering::SeqCst);
    tracing::info!("wipe started");
    let mut failed = Vec::new();
    if app.autolaunch().disable().is_err() {
        failed.push("login item");
    }
    if google_auth::forget().is_err() {
        failed.push("Keychain");
    }
    for path in paths(&app)? {
        if remove(&path).is_err() {
            failed.push("files");
        }
    }
    if !failed.is_empty() {
        failed.dedup();
        // The person stays in the app and can try again; nothing is opened or quit.
        WIPING.store(false, Ordering::SeqCst);
        return Err(format!("Could not remove: {}", failed.join(", ")));
    }
    sweep_after_exit(&late_paths(&app)?);
    let _ = Command::new("/usr/bin/open")
        .arg(links::GOOGLE_ACCESS)
        .status();
    tracing::info!("wipe finished");
    app.exit(0);
    Ok(())
}

fn remove(path: &Path) -> std::io::Result<()> {
    let result = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match result {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Starts a detached helper that waits for Picket to exit, then removes `paths` and
/// clears the preferences cache. Paths are passed as arguments, never as shell text.
fn sweep_after_exit(paths: &[PathBuf]) {
    let script = r#"pid="$1"; shift
domain="$1"; shift
while kill -0 "$pid" 2>/dev/null; do sleep 0.2; done
/usr/bin/defaults delete "$domain" >/dev/null 2>&1
/bin/rm -rf -- "$@""#;
    let domain = paths
        .first()
        .and_then(|path| path.file_stem())
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let spawned = Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .arg("picket-wipe")
        .arg(std::process::id().to_string())
        .arg(domain)
        .args(paths)
        .spawn();
    if spawned.is_err() {
        tracing::warn!("wipe helper did not start");
    }
}
