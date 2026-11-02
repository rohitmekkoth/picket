//! The local ingest socket for the `picket-openclaw` plugin. It accepts only frames
//! carrying the token from a user-only file, validates each in the core and never
//! replies, so a slow or stopped Picket cannot block the agent.

use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{FileTypeExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use chrono::Utc;
use picket_core::{
    event::AgentMode,
    ledger::{Ledger, Writer},
    openclaw::{self, FrameError, HEARTBEAT, MAX_FRAME, Monitored},
};
use tauri::Manager;

use crate::GoogleState;

const SOCKET: &str = "ingest.sock";
const TOKEN: &str = "ingest.token";
/// macOS limits a socket path to 104 bytes including the terminator.
const MAX_SOCKET_PATH: usize = 103;
/// One gateway normally holds one connection; a few more cover restarts.
const MAX_CONNECTIONS: usize = 4;
const STALE_CHECK: Duration = Duration::from_secs(30);

/// What the ingest socket is doing, for the UI.
pub struct OpenClawState(Mutex<&'static str>);

impl Default for OpenClawState {
    fn default() -> Self {
        Self(Mutex::new("inactive"))
    }
}

impl OpenClawState {
    pub fn status(&self) -> &'static str {
        self.0.lock().map_or("unavailable", |status| *status)
    }
}

/// Opens the ingest socket and starts the stale-plugin check.
pub fn start(app: tauri::AppHandle) {
    let spawned = thread::Builder::new()
        .name("picket-openclaw".into())
        .spawn(move || {
            if let Err(error) = listen(&app) {
                tracing::warn!(error, "OpenClaw ingest unavailable");
                set(&app, "unavailable");
            }
        });
    if spawned.is_err() {
        tracing::warn!("OpenClaw ingest could not start");
    }
}

fn listen(app: &tauri::AppHandle) -> Result<(), &'static str> {
    let directory = app.path().app_data_dir().map_err(|_| "data path failed")?;
    fs::create_dir_all(&directory).map_err(|_| "data directory failed")?;
    let token = Arc::new(token(&directory.join(TOKEN))?);
    let listener = bind(&directory.join(SOCKET))?;
    let stale_app = app.clone();
    thread::Builder::new()
        .name("picket-openclaw-stale".into())
        .spawn(move || {
            loop {
                check_stale(&stale_app);
                thread::sleep(STALE_CHECK);
            }
        })
        .map_err(|_| "stale check failed")?;
    set(app, "listening");
    let open = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        if open.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            open.fetch_sub(1, Ordering::SeqCst);
            continue;
        }
        let (app, token, open) = (app.clone(), Arc::clone(&token), Arc::clone(&open));
        let spawned = thread::Builder::new()
            .name("picket-openclaw-conn".into())
            .spawn(move || {
                serve(&app, stream, &token);
                if open.fetch_sub(1, Ordering::SeqCst) == 1 {
                    set(&app, "listening");
                }
            });
        if spawned.is_err() {
            tracing::warn!("OpenClaw connection could not start");
        }
    }
    Ok(())
}

/// Reads the plugin token, creating it once with owner-only permissions.
fn token(path: &Path) -> Result<String, &'static str> {
    match fs::read_to_string(path) {
        Ok(value) if is_token(value.trim()) => {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .map_err(|_| "token permissions failed")?;
            return Ok(value.trim().to_owned());
        }
        Ok(_) => fs::remove_file(path).map_err(|_| "invalid token could not be replaced")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("token unreadable"),
    }
    let value = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| "token could not be created")?;
    file.write_all(value.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|_| "token could not be written")?;
    Ok(value)
}

fn is_token(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Binds the socket with owner-only permissions, replacing a socket left by a crash.
fn bind(path: &Path) -> Result<UnixListener, &'static str> {
    if path.as_os_str().len() > MAX_SOCKET_PATH {
        return Err("socket path too long");
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            fs::remove_file(path).map_err(|_| "stale socket could not be removed")?;
        }
        Ok(_) => return Err("socket path is not a socket"),
        Err(_) => {}
    }
    let listener = UnixListener::bind(path).map_err(|_| "socket could not be bound")?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|_| "socket permissions failed")?;
    Ok(listener)
}

/// Handles one plugin connection: one frame per line, closed on any invalid token.
fn serve(app: &tauri::AppHandle, stream: UnixStream, token: &str) {
    // A connected plugin sends a heartbeat every minute; silence closes the connection.
    let timeout = HEARTBEAT.to_std().ok().map(|value| value * 2);
    if stream.set_read_timeout(timeout).is_err() {
        return;
    }
    let mut ledger: Option<Writer> = None;
    let mut reader = BufReader::new(stream);
    let mut line = Vec::with_capacity(512);
    loop {
        line.clear();
        let limit = u64::try_from(MAX_FRAME + 1).unwrap_or(u64::MAX);
        match (&mut reader).take(limit).read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) if line.last() != Some(&b'\n') => return,
            Ok(_) => {}
        }
        match openclaw::parse(line.trim_ascii_end(), token) {
            Ok(frame) => record(app, &mut ledger, frame),
            Err(FrameError::Token) => {
                tracing::warn!("OpenClaw frame rejected: invalid token");
                return;
            }
            Err(error) => tracing::warn!(%error, "OpenClaw frame rejected"),
        }
    }
}

fn record(app: &tauri::AppHandle, ledger: &mut Option<Writer>, frame: openclaw::Frame) {
    let Some(account) = monitored(app) else {
        // Reports belong to the monitored account; before it connects they cannot be placed.
        set(app, "waiting");
        return;
    };
    if ledger.is_none() {
        *ledger = crate::ledger(app).ok();
    }
    let Some(writer) = ledger.as_mut() else {
        return;
    };
    let Ok(agents) = writer.agents() else {
        return;
    };
    let monitored = Monitored {
        id: &account.id,
        email: &account.email,
    };
    match openclaw::ingest(writer, monitored, &agents, frame, Utc::now()) {
        Ok(()) => set(app, "connected"),
        Err(error) => tracing::warn!(%error, "OpenClaw frame not recorded"),
    }
}

fn check_stale(app: &tauri::AppHandle) {
    let Some(account) = monitored(app) else {
        return;
    };
    let Ok(mut writer) = crate::ledger(app) else {
        return;
    };
    let Ok(agents) = writer.agents() else {
        return;
    };
    if !agents
        .iter()
        .any(|agent| agent.enabled && agent.mode == AgentMode::OpenClaw)
    {
        return;
    }
    if openclaw::check_stale(&mut writer, &account.id, &agents, Utc::now()).is_err() {
        tracing::warn!("OpenClaw coverage could not be recorded");
    }
}

fn monitored(app: &tauri::AppHandle) -> Option<crate::MonitoredAccount> {
    app.state::<GoogleState>()
        .account
        .lock()
        .ok()
        .and_then(|account| account.clone())
}

fn set(app: &tauri::AppHandle, status: &'static str) {
    if let Ok(mut current) = app.state::<OpenClawState>().0.lock() {
        *current = status;
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A short directory under /tmp keeps socket paths within the macOS limit.
    fn directory() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let path = PathBuf::from("/tmp").join(format!("pk-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir(&path)?;
        Ok(path)
    }

    #[test]
    fn the_token_is_created_once_owner_only_and_replaced_if_invalid()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = directory()?;
        let path = directory.join(TOKEN);
        let first = token(&path)?;
        assert!(is_token(&first));
        assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o600);
        assert_eq!(token(&path)?, first, "an existing token is reused");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644))?;
        token(&path)?;
        assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o600);
        fs::write(&path, "short")?;
        assert_ne!(token(&path)?, "short");
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[test]
    fn the_socket_is_owner_only_and_never_replaces_a_regular_file()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = directory()?;
        let path = directory.join(SOCKET);
        drop(bind(&path)?);
        assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o600);
        drop(bind(&path)?);
        fs::remove_file(&path)?;
        fs::write(&path, "not a socket")?;
        assert!(bind(&path).is_err());
        assert!(bind(&directory.join("x".repeat(MAX_SOCKET_PATH))).is_err());
        fs::remove_dir_all(directory)?;
        Ok(())
    }
}
