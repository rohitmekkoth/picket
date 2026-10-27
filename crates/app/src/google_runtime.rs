//! One native worker owns Google polling and the ordered SQLite writer.

use std::{
    fs,
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};

use chrono::{Duration as ChronoDuration, Utc};
use picket_core::{
    event::{CoverageReason, Source},
    google::HttpError,
    google_collector,
    store::Store,
};
use tauri::Manager;

use crate::{GoogleState, google_auth, google_http::GoogleHttp};

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
        if app
            .state::<GoogleState>()
            .session
            .lock()
            .is_ok_and(|pending| pending.is_some())
        {
            start_worker(app);
        }
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
    store
        .retain_older_than(Utc::now() - ChronoDuration::days(180))
        .map_err(|_| "event retention failed")?;
    let http = GoogleHttp::new(google_auth::http_client().map_err(|_| "network client failed")?);
    let mut next_gmail = Instant::now();
    let mut next_calendar = Instant::now();
    let mut next_window = Instant::now() + Duration::from_secs(24 * 60 * 60);
    let mut next_retention = next_window;
    let mut gmail_healthy = false;
    let mut calendar_healthy = false;
    let mut window_healthy = true;
    loop {
        if let Some(replacement) = state
            .session
            .lock()
            .map_err(|_| "connection state failed")?
            .take()
        {
            session = replacement;
            gmail_healthy = false;
            calendar_healthy = false;
            window_healthy = true;
            next_gmail = Instant::now();
            next_calendar = Instant::now();
            next_window = Instant::now() + Duration::from_secs(24 * 60 * 60);
        }
        // Deadlines pass without new input; the store writes only when a finding changes.
        if store.reconcile(Utc::now()).is_err() {
            tracing::warn!("assessment tick failed");
        }
        if Instant::now() < next_gmail
            && Instant::now() < next_calendar
            && (!calendar_healthy || Instant::now() < next_window)
            && Instant::now() < next_retention
        {
            thread::sleep(Duration::from_secs(5));
            continue;
        }
        if let Err(error) = google_auth::refresh(&mut session) {
            tracing::warn!(%error, "Google authorization unavailable");
            let reason = if matches!(error, google_auth::AuthError::Network) {
                CoverageReason::RequestFailed
            } else {
                CoverageReason::AuthUnavailable
            };
            let now = Utc::now();
            for source in [Source::Gmail, Source::Calendar] {
                let _ = google_collector::record_failure(
                    &mut store,
                    &session.credentials.account_id,
                    source,
                    reason.clone(),
                    now,
                );
            }
            if let Ok(mut status) = state.status.lock() {
                *status = "degraded";
            }
            next_gmail = Instant::now() + Duration::from_secs(10);
            next_calendar = next_gmail;
            continue;
        }
        let now = Utc::now();
        if Instant::now() >= next_gmail {
            let result = google_collector::poll_gmail(
                &http,
                &mut store,
                &session.credentials.account_id,
                &session.credentials.account_email,
                &session.access_token,
                now,
            );
            gmail_healthy = result.is_ok();
            next_gmail =
                Instant::now() + after_poll(&mut store, &session, Source::Gmail, result, 45);
        }
        if Instant::now() >= next_calendar {
            let result = google_collector::poll_calendar(
                &http,
                &mut store,
                &session.credentials.account_id,
                &session.access_token,
                now,
            );
            calendar_healthy = result.is_ok();
            next_calendar =
                Instant::now() + after_poll(&mut store, &session, Source::Calendar, result, 60);
        }
        if calendar_healthy && Instant::now() >= next_window {
            if let Err(error) = google_collector::refresh_calendar_window(
                &http,
                &mut store,
                &session.credentials.account_id,
                &session.access_token,
                now,
            ) {
                tracing::warn!(%error, "Calendar window refresh degraded");
                window_healthy = false;
                next_window = Instant::now() + Duration::from_secs(10 * 60);
            } else {
                window_healthy = true;
                next_window = Instant::now() + Duration::from_secs(24 * 60 * 60);
            }
        }
        if Instant::now() >= next_retention {
            if store
                .retain_older_than(now - ChronoDuration::days(180))
                .is_err()
            {
                tracing::warn!("event retention failed");
            }
            next_retention = Instant::now() + Duration::from_secs(24 * 60 * 60);
        }
        if let Ok(mut status) = state.status.lock() {
            *status = if gmail_healthy && calendar_healthy && window_healthy {
                "monitoring"
            } else {
                "degraded"
            };
        }
    }
}

/// Schedules the next poll: soon while catching up, later when current, and with
/// backoff after a failure, which is recorded as a delay rather than a gap.
fn after_poll(
    store: &mut Store,
    session: &google_auth::Session,
    source: Source,
    result: Result<google_collector::Progress, google_collector::CollectorError>,
    interval: u64,
) -> Duration {
    match result {
        Ok(google_collector::Progress::CaughtUp) => Duration::from_secs(interval),
        Ok(google_collector::Progress::Behind) => Duration::from_secs(1),
        Err(error) => {
            tracing::warn!(%error, source = source.as_str(), "collection degraded");
            let rate_limited = matches!(
                &error,
                google_collector::CollectorError::Http {
                    source: HttpError::RateLimited,
                    ..
                }
            );
            let _ = google_collector::record_failure(
                store,
                &session.credentials.account_id,
                source,
                CoverageReason::RequestFailed,
                Utc::now(),
            );
            Duration::from_secs(if rate_limited { 60 } else { 10 })
        }
    }
}
