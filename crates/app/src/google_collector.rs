//! Bounded, restart-safe polling over the approved Google resources.

use std::collections::BTreeSet;

use chrono::{DateTime, Duration, Utc};
use picket_core::{
    calendar::{CalendarEvent, CalendarState},
    event::{
        Coverage, CoverageReason, CoverageState, Event, EventKey, EventPayload, Origin, Source,
    },
    google::GmailMessage,
    store::{CollectorCursor, Store},
};
use thiserror::Error;
use uuid::Uuid;

use crate::google_http::{
    CalendarPage, GmailHistory, GmailList, GmailProfile, GoogleHttp, HttpError,
};

const MAX_PAGES: usize = 40;
const MAX_MESSAGES: usize = 5000;
type CalendarUpdates = (Vec<Event>, Vec<(String, CalendarState)>);

#[derive(Debug, Error)]
pub enum CollectorError {
    #[error("Google collection is unavailable")]
    Http,
    #[error("Google returned invalid source metadata")]
    Source,
    #[error("local event store is unavailable")]
    Store,
    #[error("Google returned more data than this poll can safely process")]
    Limit,
    #[error("Google account identity changed")]
    Account,
}

pub trait GoogleApi {
    fn gmail_profile(&self, token: &str) -> Result<GmailProfile, HttpError>;
    fn gmail_list(
        &self,
        token: &str,
        label: &str,
        page: Option<&str>,
    ) -> Result<GmailList, HttpError>;
    fn gmail_message(&self, token: &str, id: &str) -> Result<GmailMessage, HttpError>;
    fn gmail_history(
        &self,
        token: &str,
        cursor: &str,
        page: Option<&str>,
    ) -> Result<GmailHistory, HttpError>;
    fn calendar_list(
        &self,
        token: &str,
        sync: Option<&str>,
        page: Option<&str>,
        bounds: Option<(&str, &str)>,
    ) -> Result<CalendarPage, HttpError>;
}

impl GoogleApi for GoogleHttp {
    fn gmail_profile(&self, token: &str) -> Result<GmailProfile, HttpError> {
        GoogleHttp::gmail_profile(self, token)
    }
    fn gmail_list(
        &self,
        token: &str,
        label: &str,
        page: Option<&str>,
    ) -> Result<GmailList, HttpError> {
        GoogleHttp::gmail_list(self, token, label, page)
    }
    fn gmail_message(&self, token: &str, id: &str) -> Result<GmailMessage, HttpError> {
        GoogleHttp::gmail_message(self, token, id)
    }
    fn gmail_history(
        &self,
        token: &str,
        cursor: &str,
        page: Option<&str>,
    ) -> Result<GmailHistory, HttpError> {
        GoogleHttp::gmail_history(self, token, cursor, page)
    }
    fn calendar_list(
        &self,
        token: &str,
        sync: Option<&str>,
        page: Option<&str>,
        bounds: Option<(&str, &str)>,
    ) -> Result<CalendarPage, HttpError> {
        GoogleHttp::calendar_list(self, token, sync, page, bounds)
    }
}

pub fn poll_gmail(
    api: &impl GoogleApi,
    store: &mut Store,
    account: &str,
    account_email: &str,
    token: &str,
    now: DateTime<Utc>,
) -> Result<(), CollectorError> {
    let checkpoint = store
        .collector_checkpoint(account, &Source::Gmail)
        .map_err(|_| CollectorError::Store)?;
    let cursor = if let Some(checkpoint) = &checkpoint {
        checkpoint.value.clone()
    } else {
        bootstrap_gmail(api, store, account, account_email, token, now)?
    };
    let origin = if checkpoint
        .as_ref()
        .is_some_and(|value| now - value.updated_at > Duration::minutes(2))
    {
        Origin::CatchUp
    } else {
        Origin::Live
    };
    let mut page_token = None;
    let mut message_ids = BTreeSet::new();
    let mut last_history = None;
    for _ in 0..MAX_PAGES {
        let page = match api.gmail_history(token, &cursor, page_token.as_deref()) {
            Ok(page) => page,
            Err(HttpError::ExpiredCursor) => {
                record_gap(
                    store,
                    account,
                    Source::Gmail,
                    Some(cursor),
                    Some(CoverageReason::CursorExpired),
                    checkpoint.map(|value| value.updated_at),
                    now,
                )?;
                return bootstrap_gmail(api, store, account, account_email, token, now).map(|_| ());
            }
            Err(_) => return Err(CollectorError::Http),
        };
        for record in &page.history {
            for item in record.messages_added.iter().chain(&record.labels_added) {
                message_ids.insert(item.message.id.clone());
            }
        }
        if message_ids.len() > MAX_MESSAGES {
            record_gap(
                store,
                account,
                Source::Gmail,
                Some(cursor),
                Some(CoverageReason::PageLimit),
                checkpoint.map(|value| value.updated_at),
                now,
            )?;
            return Err(CollectorError::Limit);
        }
        last_history = Some(page.history_id);
        page_token = page.next_page_token;
        if page_token.is_none() {
            break;
        }
    }
    if page_token.is_some() {
        record_gap(
            store,
            account,
            Source::Gmail,
            Some(cursor),
            Some(CoverageReason::PageLimit),
            checkpoint.map(|value| value.updated_at),
            now,
        )?;
        return Err(CollectorError::Limit);
    }
    let mut events = Vec::new();
    let mut missing = false;
    for id in message_ids {
        let message = match api.gmail_message(token, &id) {
            Ok(message) => message,
            Err(HttpError::MissingResource) => {
                missing = true;
                continue;
            }
            Err(_) => return Err(CollectorError::Http),
        };
        for event in message
            .observations(account, origin.clone(), now)
            .map_err(|_| CollectorError::Source)?
        {
            if !store
                .event_exists(&event.key, event.payload.kind())
                .map_err(|_| CollectorError::Store)?
            {
                events.push(event);
            }
        }
    }
    let next = last_history.ok_or(CollectorError::Source)?;
    events.push(coverage_event(
        account,
        Source::Gmail,
        if missing {
            CoverageState::Gap
        } else {
            CoverageState::Healthy
        },
        Some(next.clone()),
        missing.then_some(CoverageReason::MissingResource),
        checkpoint.map(|value| value.updated_at),
        now,
    ));
    commit_page(store, &events, account, Source::Gmail, next, now)
}

fn bootstrap_gmail(
    api: &impl GoogleApi,
    store: &mut Store,
    account: &str,
    account_email: &str,
    token: &str,
    now: DateTime<Utc>,
) -> Result<String, CollectorError> {
    let profile = api.gmail_profile(token).map_err(|_| CollectorError::Http)?;
    if profile.email_address.to_lowercase() != account_email || profile.history_id.is_empty() {
        return Err(CollectorError::Account);
    }
    let cutoff = now - Duration::days(30);
    for label in ["SENT", "INBOX"] {
        let mut page_token = None;
        let mut complete = false;
        for _ in 0..MAX_PAGES {
            let page = api
                .gmail_list(token, label, page_token.as_deref())
                .map_err(|_| CollectorError::Http)?;
            let mut events = Vec::new();
            for item in page.messages {
                let message = match api.gmail_message(token, &item.id) {
                    Ok(message) => message,
                    Err(HttpError::MissingResource) => continue,
                    Err(_) => return Err(CollectorError::Http),
                };
                let milliseconds: i64 = message
                    .internal_date
                    .parse()
                    .map_err(|_| CollectorError::Source)?;
                if DateTime::from_timestamp_millis(milliseconds).is_some_and(|date| date >= cutoff)
                {
                    for event in message
                        .observations(account, Origin::Baseline, now)
                        .map_err(|_| CollectorError::Source)?
                    {
                        if !store
                            .event_exists(&event.key, event.payload.kind())
                            .map_err(|_| CollectorError::Store)?
                        {
                            events.push(event);
                        }
                    }
                }
            }
            for event in &events {
                store.append(event).map_err(|_| CollectorError::Store)?;
            }
            page_token = page.next_page_token;
            if page_token.is_none() {
                complete = true;
                break;
            }
        }
        if !complete {
            record_gap(
                store,
                account,
                Source::Gmail,
                None,
                Some(CoverageReason::PageLimit),
                Some(cutoff),
                now,
            )?;
            return Err(CollectorError::Limit);
        }
    }
    commit_page(
        store,
        &[],
        account,
        Source::Gmail,
        profile.history_id.clone(),
        now,
    )?;
    Ok(profile.history_id)
}

fn commit_page(
    store: &mut Store,
    events: &[Event],
    account: &str,
    source: Source,
    cursor: String,
    now: DateTime<Utc>,
) -> Result<(), CollectorError> {
    store
        .append_collector_page(
            events,
            &CollectorCursor {
                account: account.into(),
                source,
                value: cursor,
                updated_at: now,
            },
        )
        .map_err(|_| CollectorError::Store)?;
    store.reconcile(now).map_err(|_| CollectorError::Store)?;
    Ok(())
}

fn record_gap(
    store: &mut Store,
    account: &str,
    source: Source,
    cursor: Option<String>,
    reason: Option<CoverageReason>,
    gap_since: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Result<(), CollectorError> {
    let event = coverage_event(
        account,
        source,
        CoverageState::Gap,
        cursor,
        reason,
        gap_since,
        now,
    );
    store
        .append_assessed(&event, now)
        .map_err(|_| CollectorError::Store)?;
    Ok(())
}

fn coverage_event(
    account: &str,
    source: Source,
    state: CoverageState,
    cursor: Option<String>,
    reason: Option<CoverageReason>,
    gap_since: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Event {
    Event {
        schema_version: 1,
        key: EventKey {
            account: account.into(),
            source,
            resource: "coverage/effects".into(),
            revision: Uuid::new_v4().to_string(),
        },
        occurred_at: now,
        ingested_at: now,
        source_time_quality: None,
        adapter: None,
        payload: EventPayload::Coverage(Coverage {
            capability: "effects".into(),
            state,
            cursor,
            gap_since,
            reason,
            partial_pages: None,
        }),
    }
}

pub fn poll_calendar(
    api: &impl GoogleApi,
    store: &mut Store,
    account: &str,
    token: &str,
    now: DateTime<Utc>,
) -> Result<(), CollectorError> {
    let checkpoint = store
        .collector_checkpoint(account, &Source::Calendar)
        .map_err(|_| CollectorError::Store)?;
    match checkpoint {
        None => bootstrap_calendar(api, store, account, token, now, Origin::Baseline),
        Some(checkpoint) if matches!(checkpoint.value.as_str(), "bootstrap" | "resync") => {
            let origin = if checkpoint.value == "resync" {
                Origin::CatchUp
            } else {
                Origin::Baseline
            };
            bootstrap_calendar(api, store, account, token, now, origin)
        }
        Some(checkpoint) => {
            let origin = if now - checkpoint.updated_at > Duration::minutes(2) {
                Origin::CatchUp
            } else {
                Origin::Live
            };
            let mut page_token = None;
            for _ in 0..MAX_PAGES {
                let page = match api.calendar_list(
                    token,
                    Some(&checkpoint.value),
                    page_token.as_deref(),
                    None,
                ) {
                    Ok(page) => page,
                    Err(HttpError::ExpiredCursor) => {
                        record_gap(
                            store,
                            account,
                            Source::Calendar,
                            Some(checkpoint.value),
                            Some(CoverageReason::CursorExpired),
                            Some(checkpoint.updated_at),
                            now,
                        )?;
                        store
                            .append_collector_page(
                                &[],
                                &CollectorCursor {
                                    account: account.into(),
                                    source: Source::Calendar,
                                    value: "resync".into(),
                                    updated_at: now,
                                },
                            )
                            .map_err(|_| CollectorError::Store)?;
                        return bootstrap_calendar(
                            api,
                            store,
                            account,
                            token,
                            now,
                            Origin::CatchUp,
                        );
                    }
                    Err(_) => return Err(CollectorError::Http),
                };
                let items = select_calendar_items(store, account, page.items, now)?;
                let (events, snapshots) =
                    calendar_observations(store, account, items, origin.clone(), now)?;
                let next_page = page.next_page_token;
                let final_page = next_page.is_none();
                let next_cursor = if final_page {
                    page.next_sync_token.ok_or(CollectorError::Source)?
                } else {
                    checkpoint.value.clone()
                };
                let mut events = events;
                if final_page {
                    events.push(coverage_event(
                        account,
                        Source::Calendar,
                        CoverageState::Healthy,
                        Some(next_cursor.clone()),
                        None,
                        None,
                        now,
                    ));
                }
                store
                    .append_collector_page_with_snapshots(
                        &events,
                        &CollectorCursor {
                            account: account.into(),
                            source: Source::Calendar,
                            value: next_cursor,
                            updated_at: if final_page {
                                now
                            } else {
                                checkpoint.updated_at
                            },
                        },
                        &snapshots,
                    )
                    .map_err(|_| CollectorError::Store)?;
                store.reconcile(now).map_err(|_| CollectorError::Store)?;
                page_token = next_page;
                if final_page {
                    return Ok(());
                }
            }
            record_gap(
                store,
                account,
                Source::Calendar,
                Some(checkpoint.value),
                Some(CoverageReason::PageLimit),
                Some(checkpoint.updated_at),
                now,
            )?;
            Err(CollectorError::Limit)
        }
    }
}

pub fn record_failure(
    store: &mut Store,
    account: &str,
    source: Source,
    now: DateTime<Utc>,
) -> Result<(), CollectorError> {
    let checkpoint = store
        .collector_checkpoint(account, &source)
        .map_err(|_| CollectorError::Store)?;
    record_gap(
        store,
        account,
        source,
        checkpoint.as_ref().map(|value| value.value.clone()),
        Some(CoverageReason::RequestFailed),
        checkpoint.map(|value| value.updated_at),
        now,
    )
}

pub fn refresh_calendar_window(
    api: &impl GoogleApi,
    store: &mut Store,
    account: &str,
    token: &str,
    now: DateTime<Utc>,
) -> Result<(), CollectorError> {
    let checkpoint = store
        .collector_checkpoint(account, &Source::Calendar)
        .map_err(|_| CollectorError::Store)?
        .ok_or(CollectorError::Source)?;
    if matches!(checkpoint.value.as_str(), "bootstrap" | "resync") {
        return Err(CollectorError::Source);
    }
    let from = (now - Duration::days(30)).to_rfc3339();
    let to = (now + Duration::days(90)).to_rfc3339();
    let mut page_token = None;
    for _ in 0..MAX_PAGES {
        let page = api
            .calendar_list(token, None, page_token.as_deref(), Some((&from, &to)))
            .map_err(|_| CollectorError::Http)?;
        let items = select_calendar_items(store, account, page.items, now)?;
        let (events, snapshots) =
            calendar_observations(store, account, items, Origin::Baseline, now)?;
        store
            .append_collector_page_with_snapshots(&events, &checkpoint, &snapshots)
            .map_err(|_| CollectorError::Store)?;
        store.reconcile(now).map_err(|_| CollectorError::Store)?;
        page_token = page.next_page_token;
        if page_token.is_none() {
            return Ok(());
        }
    }
    record_gap(
        store,
        account,
        Source::Calendar,
        Some(checkpoint.value),
        Some(CoverageReason::PageLimit),
        Some(checkpoint.updated_at),
        now,
    )?;
    Err(CollectorError::Limit)
}

fn bootstrap_calendar(
    api: &impl GoogleApi,
    store: &mut Store,
    account: &str,
    token: &str,
    now: DateTime<Utc>,
    origin: Origin,
) -> Result<(), CollectorError> {
    let from = (now - Duration::days(30)).to_rfc3339();
    let to = (now + Duration::days(90)).to_rfc3339();
    let marker = if origin == Origin::CatchUp {
        "resync"
    } else {
        "bootstrap"
    };
    for bounded in [true, false] {
        let mut page_token = None;
        let mut complete = false;
        for _ in 0..MAX_PAGES {
            let bounds = bounded.then_some((from.as_str(), to.as_str()));
            let page = api
                .calendar_list(token, None, page_token.as_deref(), bounds)
                .map_err(|_| CollectorError::Http)?;
            let items = select_calendar_items(store, account, page.items, now)?;
            let (events, snapshots) =
                calendar_observations(store, account, items, origin.clone(), now)?;
            let next_page = page.next_page_token;
            let final_page = next_page.is_none();
            let cursor = if !bounded && final_page {
                page.next_sync_token.ok_or(CollectorError::Source)?
            } else {
                marker.into()
            };
            let mut events = events;
            if !bounded && final_page {
                events.push(coverage_event(
                    account,
                    Source::Calendar,
                    CoverageState::Healthy,
                    Some(cursor.clone()),
                    None,
                    None,
                    now,
                ));
            }
            store
                .append_collector_page_with_snapshots(
                    &events,
                    &CollectorCursor {
                        account: account.into(),
                        source: Source::Calendar,
                        value: cursor,
                        updated_at: now,
                    },
                    &snapshots,
                )
                .map_err(|_| CollectorError::Store)?;
            store.reconcile(now).map_err(|_| CollectorError::Store)?;
            page_token = next_page;
            if final_page {
                complete = true;
                break;
            }
        }
        if !complete {
            record_gap(
                store,
                account,
                Source::Calendar,
                Some(marker.into()),
                Some(CoverageReason::PageLimit),
                Some(now),
                now,
            )?;
            return Err(CollectorError::Limit);
        }
    }
    Ok(())
}

fn calendar_observations(
    store: &Store,
    account: &str,
    items: Vec<CalendarEvent>,
    origin: Origin,
    now: DateTime<Utc>,
) -> Result<CalendarUpdates, CollectorError> {
    let mut events = Vec::new();
    let mut snapshots = Vec::new();
    for item in items {
        let previous = store
            .calendar_snapshot(account, &item.id)
            .map_err(|_| CollectorError::Store)?;
        let (event, state) = item
            .observation(account, origin.clone(), now, previous.as_ref())
            .map_err(|_| CollectorError::Source)?;
        if let Some(event) = event {
            events.push(event);
        }
        snapshots.push((item.id, state));
    }
    Ok((events, snapshots))
}

fn select_calendar_items(
    store: &Store,
    account: &str,
    items: Vec<CalendarEvent>,
    now: DateTime<Utc>,
) -> Result<Vec<CalendarEvent>, CollectorError> {
    items
        .into_iter()
        .filter_map(|event| {
            let known = store
                .calendar_snapshot(account, &event.id)
                .map_err(|_| CollectorError::Store);
            match known {
                Ok(Some(_)) => Some(Ok(event)),
                Ok(None) => match calendar_in_window(&event, now) {
                    Ok(true) => Some(Ok(event)),
                    Ok(false) => None,
                    Err(error) => Some(Err(error)),
                },
                Err(error) => Some(Err(error)),
            }
        })
        .collect()
}

fn calendar_in_window(event: &CalendarEvent, now: DateTime<Utc>) -> Result<bool, CollectorError> {
    if event.status.as_deref() == Some("cancelled") {
        return Ok(false);
    }
    let from = now - Duration::days(30);
    let to = now + Duration::days(90);
    let Some(start) = &event.start else {
        return Err(CollectorError::Source);
    };
    if let Some(date_time) = &start.date_time {
        let start = DateTime::parse_from_rfc3339(date_time).map_err(|_| CollectorError::Source)?;
        let end = event
            .end
            .as_ref()
            .and_then(|end| end.date_time.as_deref())
            .map(DateTime::parse_from_rfc3339)
            .transpose()
            .map_err(|_| CollectorError::Source)?;
        return Ok(start.with_timezone(&Utc) <= to
            && end.is_none_or(|end| end.with_timezone(&Utc) >= from));
    }
    if let Some(date) = &start.date {
        let start = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map_err(|_| CollectorError::Source)?;
        let end = event
            .end
            .as_ref()
            .and_then(|end| end.date.as_deref())
            .map(|date| chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d"))
            .transpose()
            .map_err(|_| CollectorError::Source)?;
        return Ok(start <= to.date_naive() && end.is_none_or(|end| end > from.date_naive()));
    }
    Err(CollectorError::Source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, sync::Mutex};

    struct FakeApi {
        calendar: Mutex<VecDeque<Result<CalendarPage, HttpError>>>,
        history: Mutex<VecDeque<Result<GmailHistory, HttpError>>>,
        message: GmailMessage,
    }

    impl FakeApi {
        fn new() -> Result<Self, serde_json::Error> {
            Ok(Self {
                calendar: Mutex::new(VecDeque::new()),
                history: Mutex::new(VecDeque::new()),
                message: serde_json::from_value(serde_json::json!({
                    "id": "abc123", "threadId": "abc123", "historyId": "10",
                    "internalDate": "1792627200000", "labelIds": ["SENT"],
                    "payload": {"headers": [
                        {"name": "From", "value": "sender@example.test"},
                        {"name": "To", "value": "receiver@example.test"},
                        {"name": "Subject", "value": "Synthetic"}
                    ]}
                }))?,
            })
        }
    }

    impl GoogleApi for FakeApi {
        fn gmail_profile(&self, _: &str) -> Result<GmailProfile, HttpError> {
            Ok(GmailProfile {
                email_address: "sender@example.test".into(),
                history_id: "10".into(),
            })
        }
        fn gmail_list(
            &self,
            _: &str,
            label: &str,
            _: Option<&str>,
        ) -> Result<GmailList, HttpError> {
            Ok(GmailList {
                messages: if label == "SENT" {
                    vec![crate::google_http::GmailRef {
                        id: "abc123".into(),
                    }]
                } else {
                    Vec::new()
                },
                next_page_token: None,
            })
        }
        fn gmail_message(&self, _: &str, _: &str) -> Result<GmailMessage, HttpError> {
            Ok(self.message.clone())
        }
        fn gmail_history(
            &self,
            _: &str,
            _: &str,
            _: Option<&str>,
        ) -> Result<GmailHistory, HttpError> {
            self.history
                .lock()
                .map_err(|_| HttpError::Request)?
                .pop_front()
                .ok_or(HttpError::Request)?
        }
        fn calendar_list(
            &self,
            _: &str,
            _: Option<&str>,
            _: Option<&str>,
            _: Option<(&str, &str)>,
        ) -> Result<CalendarPage, HttpError> {
            self.calendar
                .lock()
                .map_err(|_| HttpError::Request)?
                .pop_front()
                .ok_or(HttpError::Request)?
        }
    }

    fn test_store() -> Result<(Store, std::path::PathBuf), Box<dyn std::error::Error>> {
        let path = std::env::temp_dir().join(format!("picket-m5-{}.db", Uuid::new_v4()));
        Ok((Store::open(&path)?, path))
    }

    fn calendar_page(etag: &str, sync: Option<&str>) -> Result<CalendarPage, serde_json::Error> {
        serde_json::from_value(serde_json::json!({
            "items": [{
                "id": "event1", "etag": etag, "status": "confirmed",
                "updated": "2026-10-22T12:00:00Z", "summary": "Synthetic",
                "start": {"dateTime": "2026-10-23T12:00:00Z"},
                "end": {"dateTime": "2026-10-23T13:00:00Z"},
                "organizer": {"email": "sender@example.test", "self": true}
            }], "nextSyncToken": sync
        }))
    }

    #[test]
    fn gmail_baseline_and_replayed_history_do_not_duplicate_send()
    -> Result<(), Box<dyn std::error::Error>> {
        let (mut store, path) = test_store()?;
        let api = FakeApi::new()?;
        let now = DateTime::parse_from_rfc3339("2026-10-22T12:00:00Z")?.with_timezone(&Utc);
        for _ in 0..2 {
            api.history.lock().map_err(|_| "lock")?.push_back(Ok(serde_json::from_value(serde_json::json!({
                "historyId": "11", "history": [{"messagesAdded": [{"message": {"id": "abc123"}}]}]
            }))?));
        }
        poll_gmail(
            &api,
            &mut store,
            "synthetic-account",
            "sender@example.test",
            "token",
            now,
        )?;
        poll_gmail(
            &api,
            &mut store,
            "synthetic-account",
            "sender@example.test",
            "token",
            now,
        )?;
        assert_eq!(
            store
                .events()?
                .iter()
                .filter(|record| matches!(record.event.payload, EventPayload::Effect(_)))
                .count(),
            1
        );
        assert_eq!(
            store
                .collector_cursor("synthetic-account", &Source::Gmail)?
                .as_deref(),
            Some("11")
        );
        drop(store);
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn calendar_revision_and_cursor_survive_restart() -> Result<(), Box<dyn std::error::Error>> {
        let (mut store, path) = test_store()?;
        let api = FakeApi::new()?;
        let now = DateTime::parse_from_rfc3339("2026-10-22T12:00:00Z")?.with_timezone(&Utc);
        {
            let mut pages = api.calendar.lock().map_err(|_| "lock")?;
            pages.push_back(Ok(calendar_page("rev1", None)?));
            pages.push_back(Ok(calendar_page("rev1", Some("sync1"))?));
            pages.push_back(Ok(calendar_page("rev2", Some("sync2"))?));
        }
        poll_calendar(&api, &mut store, "synthetic-account", "token", now)?;
        drop(store);
        let mut store = Store::open(&path)?;
        poll_calendar(&api, &mut store, "synthetic-account", "token", now)?;
        assert_eq!(
            store
                .collector_cursor("synthetic-account", &Source::Calendar)?
                .as_deref(),
            Some("sync2")
        );
        assert_eq!(
            store
                .events()?
                .iter()
                .filter(|record| matches!(record.event.payload, EventPayload::Effect(_)))
                .count(),
            2
        );
        store.verify()?;
        drop(store);
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn invalid_calendar_date_is_not_silently_skipped() -> Result<(), Box<dyn std::error::Error>> {
        let mut item = calendar_page("rev1", None)?.items.remove(0);
        item.start.as_mut().ok_or("start")?.date_time = Some("invalid".into());
        let now = Utc::now();
        assert!(calendar_in_window(&item, now).is_err());
        Ok(())
    }

    #[test]
    fn expired_gmail_cursor_records_gap_before_rescan() -> Result<(), Box<dyn std::error::Error>> {
        let mut store = Store::memory()?;
        let api = FakeApi::new()?;
        let now = DateTime::parse_from_rfc3339("2026-10-22T12:00:00Z")?.with_timezone(&Utc);
        {
            let mut history = api.history.lock().map_err(|_| "lock")?;
            history.push_back(Ok(serde_json::from_value(
                serde_json::json!({"historyId": "11"}),
            )?));
            history.push_back(Err(HttpError::ExpiredCursor));
        }
        poll_gmail(
            &api,
            &mut store,
            "synthetic-account",
            "sender@example.test",
            "token",
            now,
        )?;
        poll_gmail(
            &api,
            &mut store,
            "synthetic-account",
            "sender@example.test",
            "token",
            now,
        )?;
        assert_eq!(
            store
                .collector_cursor("synthetic-account", &Source::Gmail)?
                .as_deref(),
            Some("10")
        );
        assert!(store.events()?.iter().any(|record| matches!(
            &record.event.payload,
            EventPayload::Coverage(Coverage {
                state: CoverageState::Gap,
                reason: Some(CoverageReason::CursorExpired),
                ..
            })
        )));
        Ok(())
    }

    #[test]
    fn expired_calendar_token_records_gap_and_resyncs() -> Result<(), Box<dyn std::error::Error>> {
        let mut store = Store::memory()?;
        let api = FakeApi::new()?;
        let now = DateTime::parse_from_rfc3339("2026-10-22T12:00:00Z")?.with_timezone(&Utc);
        {
            let mut pages = api.calendar.lock().map_err(|_| "lock")?;
            pages.push_back(Ok(calendar_page("rev1", None)?));
            pages.push_back(Ok(calendar_page("rev1", Some("sync1"))?));
            pages.push_back(Err(HttpError::ExpiredCursor));
            pages.push_back(Ok(calendar_page("rev2", None)?));
            pages.push_back(Ok(calendar_page("rev2", Some("sync2"))?));
        }
        poll_calendar(&api, &mut store, "synthetic-account", "token", now)?;
        poll_calendar(&api, &mut store, "synthetic-account", "token", now)?;
        assert_eq!(
            store
                .collector_cursor("synthetic-account", &Source::Calendar)?
                .as_deref(),
            Some("sync2")
        );
        assert!(store.events()?.iter().any(|record| matches!(
            &record.event.payload,
            EventPayload::Coverage(Coverage {
                state: CoverageState::Gap,
                reason: Some(CoverageReason::CursorExpired),
                ..
            })
        )));
        Ok(())
    }

    #[test]
    fn rolling_window_discovers_future_event_without_advancing_sync_cursor()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut store = Store::memory()?;
        let api = FakeApi::new()?;
        let initial = DateTime::parse_from_rfc3339("2026-10-22T12:00:00Z")?.with_timezone(&Utc);
        let later = DateTime::parse_from_rfc3339("2026-12-01T12:00:00Z")?.with_timezone(&Utc);
        let mut future = calendar_page("rev1", Some("sync1"))?;
        future.items[0].start.as_mut().ok_or("start")?.date_time =
            Some("2027-02-01T12:00:00Z".into());
        future.items[0].end.as_mut().ok_or("end")?.date_time = Some("2027-02-01T13:00:00Z".into());
        {
            let mut pages = api.calendar.lock().map_err(|_| "lock")?;
            pages.push_back(Ok(serde_json::from_value(
                serde_json::json!({"items": []}),
            )?));
            pages.push_back(Ok(future.clone()));
            pages.push_back(Ok(future));
        }
        poll_calendar(&api, &mut store, "synthetic-account", "token", initial)?;
        assert_eq!(
            store
                .events()?
                .iter()
                .filter(|record| matches!(record.event.payload, EventPayload::Effect(_)))
                .count(),
            0
        );
        refresh_calendar_window(&api, &mut store, "synthetic-account", "token", later)?;
        assert_eq!(
            store
                .events()?
                .iter()
                .filter(|record| matches!(record.event.payload, EventPayload::Effect(_)))
                .count(),
            1
        );
        assert_eq!(
            store
                .collector_cursor("synthetic-account", &Source::Calendar)?
                .as_deref(),
            Some("sync1")
        );
        Ok(())
    }
}
