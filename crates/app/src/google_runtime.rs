//! One native worker owns Google polling and the ordered SQLite writer.

use std::{
    fs,
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};

use chrono::{Duration as ChronoDuration, Utc};
use picket_core::{event::Source, store::Store};
use tauri::Manager;

use crate::{
    GoogleState, google_auth, google_collector,
    google_http::{GoogleHttp, HttpError},
};

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
            let now = Utc::now();
            for source in [Source::Gmail, Source::Calendar] {
                let _ = google_collector::record_failure(
                    &mut store,
                    &session.credentials.account_id,
                    source,
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
            let mut retry_seconds = 10;
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
                    if matches!(
                        &error,
                        google_collector::CollectorError::Http {
                            source: HttpError::RateLimited,
                            ..
                        }
                    ) {
                        retry_seconds = 60;
                    }
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
            next_gmail = Instant::now()
                + Duration::from_secs(if gmail_healthy { 45 } else { retry_seconds });
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
            next_calendar =
                Instant::now() + Duration::from_secs(if calendar_healthy { 60 } else { 10 });
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
                let _ = google_collector::record_failure(
                    &mut store,
                    &session.credentials.account_id,
                    Source::Calendar,
                    now,
                );
                calendar_healthy = false;
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
