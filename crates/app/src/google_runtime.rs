//! One native worker owns Google polling and the ordered SQLite writer.

use std::{
    fs,
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};

use chrono::Utc;
use picket_core::{event::Source, store::Store};
use tauri::Manager;

use crate::{GoogleState, google_auth, google_collector, google_http::GoogleHttp};

pub fn start_worker(app: tauri::AppHandle) {
    let state = app.state::<GoogleState>();
    if state.worker_started.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Ok(mut status) = state.status.lock() {
        *status = "starting";
    }
    thread::spawn(move || {
        if let Err(reason) = run(&app) {
            tracing::warn!(reason, "Google monitoring stopped");
            if let Ok(mut status) = app.state::<GoogleState>().status.lock() {
                *status = "unavailable";
            }
        }
        app.state::<GoogleState>()
            .worker_started
            .store(false, Ordering::SeqCst);
    });
}

fn run(app: &tauri::AppHandle) -> Result<(), &'static str> {
    let state = app.state::<GoogleState>();
    let mut session = state
        .session
        .lock()
        .map_err(|_| "connection state failed")?
        .take()
        .ok_or("Google connection missing")?;
    let directory = app.path().app_data_dir().map_err(|_| "data path failed")?;
    fs::create_dir_all(&directory).map_err(|_| "data directory failed")?;
    let mut store = Store::open(&directory.join("picket.db")).map_err(|_| "event store failed")?;
    store
        .reconcile(Utc::now())
        .map_err(|_| "event replay failed")?;
    let http = GoogleHttp::new(google_auth::http_client().map_err(|_| "network client failed")?);
    let mut next_gmail = Instant::now();
    let mut next_calendar = Instant::now();
    let mut gmail_healthy = false;
    let mut calendar_healthy = false;
    loop {
        if Instant::now() < next_gmail && Instant::now() < next_calendar {
            thread::sleep(Duration::from_secs(5));
            continue;
        }
        google_auth::refresh(&mut session).map_err(|_| "Google authorization failed")?;
        let now = Utc::now();
        if Instant::now() >= next_gmail {
            gmail_healthy = match google_collector::poll_gmail(
                &http,
                &mut store,
                &session.credentials.account_id,
                &session.credentials.account_email,
                &session.access_token,
                now,
            ) {
                Ok(()) => true,
                Err(error) => {
                    tracing::warn!(%error, "Gmail collection degraded");
                    let _ = google_collector::record_failure(
                        &mut store,
                        &session.credentials.account_id,
                        Source::Gmail,
                        now,
                    );
                    false
                }
            };
            next_gmail = Instant::now() + Duration::from_secs(45);
        }
        if Instant::now() >= next_calendar {
            calendar_healthy = match google_collector::poll_calendar(
                &http,
                &mut store,
                &session.credentials.account_id,
                &session.access_token,
                now,
            ) {
                Ok(()) => true,
                Err(error) => {
                    tracing::warn!(%error, "Calendar collection degraded");
                    let _ = google_collector::record_failure(
                        &mut store,
                        &session.credentials.account_id,
                        Source::Calendar,
                        now,
                    );
                    false
                }
            };
            next_calendar = Instant::now() + Duration::from_secs(60);
        }
        if let Ok(mut status) = state.status.lock() {
            *status = if gmail_healthy && calendar_healthy {
                "monitoring"
            } else {
                "degraded"
            };
        }
    }
}
