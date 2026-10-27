//! Bounded, restart-safe polling over the approved Google resources. Every poll makes
//! durable progress: a source that is behind resumes from its committed position.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    calendar::{CalendarEvent, CalendarState},
    event::{Coverage, CoverageReason, CoverageState, Event, Origin, Source},
    google::{CalendarPage, GmailHistory, GmailList, GmailMessage, GmailProfile, HttpError},
    ledger::{Ledger, Progress},
    store::{Batch, CollectorCursor, CoverageReport},
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Pages read per poll before yielding to other sources.
const PAGES_PER_POLL: usize = 10;
/// Metadata reads per poll; about 30 s at the Gmail pacing interval.
const MESSAGES_PER_POLL: usize = 100;
/// The daily rolling-window refresh is bounded separately because it does not move the cursor.
const WINDOW_PAGES: usize = 40;
const BASELINE_MESSAGES_PER_LABEL: usize = 50;
type CalendarUpdates = (Vec<Event>, Vec<(String, CalendarState)>);

#[derive(Debug, Error)]
pub enum CollectorError {
    #[error("Google {operation} failed: {source}")]
    Http {
        operation: &'static str,
        #[source]
        source: HttpError,
    },
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

/// Replays Gmail history from the committed cursor. Each page commits its observations
/// with the id of the last fully processed history record, so a large backlog resumes on
/// the next poll instead of repeating. Healthy coverage is reported only once caught up.
pub fn poll_gmail(
    api: &impl GoogleApi,
    store: &mut impl Ledger,
    account: &str,
    account_email: &str,
    token: &str,
    now: DateTime<Utc>,
) -> Result<Progress, CollectorError> {
    let checkpoint = store
        .checkpoint(account, &Source::Gmail)
        .map_err(|_| CollectorError::Store)?;
    let caught_up_at = checkpoint.as_ref().map(|value| value.updated_at);
    let start = match checkpoint {
        Some(checkpoint) => checkpoint.value,
        None => bootstrap_gmail(
            api,
            store,
            account,
            account_email,
            token,
            now,
            Origin::Baseline,
        )?,
    };
    let origin = if caught_up_at.is_some_and(|at| now - at > Duration::minutes(2)) {
        Origin::CatchUp
    } else {
        Origin::Live
    };
    let mut durable = start.clone();
    let mut page_token = None;
    let mut budget = MESSAGES_PER_POLL;
    for _ in 0..PAGES_PER_POLL {
        let page = match api.gmail_history(token, &start, page_token.as_deref()) {
            Ok(page) => page,
            Err(HttpError::ExpiredCursor) => {
                let gap = report(
                    account,
                    Source::Gmail,
                    "effects",
                    CoverageState::Gap,
                    Some(CoverageReason::CursorExpired),
                    caught_up_at,
                );
                commit(store, Vec::new(), None, Vec::new(), vec![gap], now)?;
                bootstrap_gmail(
                    api,
                    store,
                    account,
                    account_email,
                    token,
                    now,
                    Origin::CatchUp,
                )?;
                return Ok(Progress::Behind);
            }
            Err(source) => {
                return Err(CollectorError::Http {
                    operation: "history",
                    source,
                });
            }
        };
        let mut events = Vec::new();
        let mut missing = false;
        let mut page_complete = true;
        for record in &page.history {
            // A message is a draft only if every history entry for it says so.
            let mut ids = BTreeMap::new();
            for item in record.messages_added.iter().chain(&record.labels_added) {
                let draft = ids.entry(item.message.id.as_str()).or_insert(true);
                *draft &= item.message.is_draft();
            }
            // A record is processed whole. One larger than a full budget still proceeds,
            // so every poll advances.
            if ids.len() > budget && budget < MESSAGES_PER_POLL {
                page_complete = false;
                break;
            }
            for (id, draft) in ids {
                budget = budget.saturating_sub(1);
                match fetch_message(api, token, id)? {
                    Some(message) => push_new(
                        store,
                        &mut events,
                        message
                            .observations(account, origin.clone(), now)
                            .map_err(|_| CollectorError::Source)?,
                    )?,
                    // A replaced draft is not a send Picket could have missed.
                    None => missing |= !draft,
                }
            }
            durable.clone_from(&record.id);
        }
        let caught_up = page_complete && page.next_page_token.is_none();
        if caught_up {
            durable = page.history_id.clone();
        }
        let mut coverage = Vec::new();
        if missing {
            coverage.push(report(
                account,
                Source::Gmail,
                "effects",
                CoverageState::Gap,
                Some(CoverageReason::MissingResource),
                caught_up_at,
            ));
        } else if caught_up {
            coverage.push(report(
                account,
                Source::Gmail,
                "effects",
                CoverageState::Healthy,
                None,
                None,
            ));
        }
        let cursor = CollectorCursor {
            account: account.into(),
            source: Source::Gmail,
            value: durable.clone(),
            updated_at: if caught_up {
                now
            } else {
                caught_up_at.unwrap_or(now)
            },
        };
        commit(store, events, Some(cursor), Vec::new(), coverage, now)?;
        if caught_up {
            return Ok(Progress::CaughtUp);
        }
        if !page_complete {
            return Ok(Progress::Behind);
        }
        page_token = page.next_page_token;
    }
    Ok(Progress::Behind)
}

/// Fetches one message's metadata; `None` when it no longer exists.
fn fetch_message(
    api: &impl GoogleApi,
    token: &str,
    id: &str,
) -> Result<Option<GmailMessage>, CollectorError> {
    match api.gmail_message(token, id) {
        Ok(message) => Ok(Some(message)),
        Err(HttpError::MissingResource) => Ok(None),
        Err(source) => Err(CollectorError::Http {
            operation: "message",
            source,
        }),
    }
}

/// Adds observations that are neither stored nor already in this page.
fn push_new(
    store: &impl Ledger,
    events: &mut Vec<Event>,
    observations: Vec<Event>,
) -> Result<(), CollectorError> {
    for event in observations {
        let known = store
            .contains(&event.key, event.payload.kind())
            .map_err(|_| CollectorError::Store)?
            || events.iter().any(|existing| {
                existing.key == event.key && existing.payload.kind() == event.payload.kind()
            });
        if !known {
            events.push(event);
        }
    }
    Ok(())
}

fn bootstrap_gmail(
    api: &impl GoogleApi,
    store: &mut impl Ledger,
    account: &str,
    account_email: &str,
    token: &str,
    now: DateTime<Utc>,
    origin: Origin,
) -> Result<String, CollectorError> {
    let profile = api
        .gmail_profile(token)
        .map_err(|source| CollectorError::Http {
            operation: "profile",
            source,
        })?;
    if profile.email_address.to_lowercase() != account_email || profile.history_id.is_empty() {
        return Err(CollectorError::Account);
    }
    let cutoff = now - Duration::days(30);
    let mut ids = BTreeSet::new();
    let mut complete = true;
    let mut missing = false;
    for label in ["SENT", "INBOX"] {
        let page = api
            .gmail_list(token, label, None)
            .map_err(|source| CollectorError::Http {
                operation: "message list",
                source,
            })?;
        complete &=
            page.next_page_token.is_none() && page.messages.len() <= BASELINE_MESSAGES_PER_LABEL;
        ids.extend(
            page.messages
                .into_iter()
                .take(BASELINE_MESSAGES_PER_LABEL)
                .map(|message| message.id),
        );
    }
    let mut events = Vec::new();
    for id in ids {
        let Some(message) = fetch_message(api, token, &id)? else {
            complete = false;
            missing = true;
            continue;
        };
        let milliseconds: i64 = message
            .internal_date
            .parse()
            .map_err(|_| CollectorError::Source)?;
        if DateTime::from_timestamp_millis(milliseconds).is_some_and(|date| date >= cutoff) {
            let observations = message
                .observations(account, origin.clone(), now)
                .map_err(|_| CollectorError::Source)?;
            push_new(store, &mut events, observations)?;
        }
    }
    let baseline = report(
        account,
        Source::Gmail,
        "baseline",
        if complete {
            CoverageState::Healthy
        } else {
            CoverageState::Gap
        },
        (!complete).then_some(if missing {
            CoverageReason::MissingResource
        } else {
            CoverageReason::PageLimit
        }),
        (!complete).then_some(cutoff),
    );
    let cursor = CollectorCursor {
        account: account.into(),
        source: Source::Gmail,
        value: profile.history_id.clone(),
        updated_at: now,
    };
    commit(store, events, Some(cursor), Vec::new(), vec![baseline], now)?;
    Ok(profile.history_id)
}

fn commit(
    store: &mut impl Ledger,
    events: Vec<Event>,
    cursor: Option<CollectorCursor>,
    calendar: Vec<(String, CalendarState)>,
    coverage: Vec<CoverageReport>,
    now: DateTime<Utc>,
) -> Result<(), CollectorError> {
    store
        .commit(
            Batch {
                events,
                cursor,
                calendar,
                coverage,
            },
            now,
        )
        .map_err(|_| CollectorError::Store)?;
    Ok(())
}

fn report(
    account: &str,
    source: Source,
    capability: &str,
    state: CoverageState,
    reason: Option<CoverageReason>,
    gap_since: Option<DateTime<Utc>>,
) -> CoverageReport {
    CoverageReport {
        account: account.into(),
        source,
        coverage: Coverage {
            capability: capability.into(),
            state,
            cursor: None,
            gap_since,
            reason,
            partial_pages: None,
        },
    }
}

/// Records that a source could not be read. The cursor is intact, so this is a delay
/// that catch-up repairs, not a gap; the store logs it once per state change.
pub fn record_failure(
    store: &mut impl Ledger,
    account: &str,
    source: Source,
    reason: CoverageReason,
    now: DateTime<Utc>,
) -> Result<(), CollectorError> {
    let since = store
        .checkpoint(account, &source)
        .map_err(|_| CollectorError::Store)?
        .map(|value| value.updated_at);
    let delayed = report(
        account,
        source,
        "effects",
        CoverageState::Delayed,
        Some(reason),
        since,
    );
    commit(store, Vec::new(), None, Vec::new(), vec![delayed], now)
}

/// Durable Calendar position. Bootstrap imports the rolling window, then lists without
/// bounds to obtain the first sync token; page tokens persist so large listings resume.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
enum CalendarCursor {
    Window {
        origin: Origin,
        page: Option<String>,
    },
    Initial {
        origin: Origin,
        page: Option<String>,
    },
    Sync {
        token: String,
        page: Option<String>,
    },
}

impl CalendarCursor {
    /// A caught-up sync token is stored as itself, as earlier versions did.
    fn decode(value: &str) -> Result<Self, CollectorError> {
        Ok(match value {
            "bootstrap" => Self::Window {
                origin: Origin::Baseline,
                page: None,
            },
            "resync" => Self::Window {
                origin: Origin::CatchUp,
                page: None,
            },
            value if value.starts_with('{') => {
                serde_json::from_str(value).map_err(|_| CollectorError::Source)?
            }
            token => Self::Sync {
                token: token.into(),
                page: None,
            },
        })
    }

    fn encode(&self) -> Result<String, CollectorError> {
        match self {
            Self::Sync { token, page: None } => Ok(token.clone()),
            other => serde_json::to_string(other).map_err(|_| CollectorError::Source),
        }
    }
}

pub fn poll_calendar(
    api: &impl GoogleApi,
    store: &mut impl Ledger,
    account: &str,
    token: &str,
    now: DateTime<Utc>,
) -> Result<Progress, CollectorError> {
    let checkpoint = store
        .checkpoint(account, &Source::Calendar)
        .map_err(|_| CollectorError::Store)?;
    let caught_up_at = checkpoint.as_ref().map(|value| value.updated_at);
    let mut cursor = match &checkpoint {
        Some(checkpoint) => CalendarCursor::decode(&checkpoint.value)?,
        None => CalendarCursor::Window {
            origin: Origin::Baseline,
            page: None,
        },
    };
    let from = (now - Duration::days(30)).to_rfc3339();
    let to = (now + Duration::days(90)).to_rfc3339();
    let live = if caught_up_at.is_some_and(|at| now - at > Duration::minutes(2)) {
        Origin::CatchUp
    } else {
        Origin::Live
    };
    for _ in 0..PAGES_PER_POLL {
        let (sync, page_token, bounds, origin) = match &cursor {
            CalendarCursor::Window { origin, page } => {
                (None, page, Some((from.as_str(), to.as_str())), origin)
            }
            CalendarCursor::Initial { origin, page } => (None, page, None, origin),
            CalendarCursor::Sync { token, page } => (Some(token.as_str()), page, None, &live),
        };
        let page = match api.calendar_list(token, sync, page_token.as_deref(), bounds) {
            Ok(page) => page,
            Err(HttpError::ExpiredCursor) if sync.is_some() => {
                cursor = CalendarCursor::Window {
                    origin: Origin::CatchUp,
                    page: None,
                };
                let gap = report(
                    account,
                    Source::Calendar,
                    "effects",
                    CoverageState::Gap,
                    Some(CoverageReason::CursorExpired),
                    caught_up_at,
                );
                let restart = CollectorCursor {
                    account: account.into(),
                    source: Source::Calendar,
                    value: cursor.encode()?,
                    updated_at: caught_up_at.unwrap_or(now),
                };
                commit(store, Vec::new(), Some(restart), Vec::new(), vec![gap], now)?;
                continue;
            }
            Err(source) => {
                return Err(CollectorError::Http {
                    operation: "calendar list",
                    source,
                });
            }
        };
        let origin = origin.clone();
        let items = select_calendar_items(store, account, page.items, now)?;
        let (events, snapshots) =
            calendar_observations(store, account, items, origin.clone(), now)?;
        let next = match (cursor, page.next_page_token) {
            (CalendarCursor::Window { .. }, Some(page)) => CalendarCursor::Window {
                origin,
                page: Some(page),
            },
            (CalendarCursor::Window { .. }, None) => CalendarCursor::Initial { origin, page: None },
            (CalendarCursor::Initial { .. }, Some(page)) => CalendarCursor::Initial {
                origin,
                page: Some(page),
            },
            (CalendarCursor::Sync { token, .. }, Some(page)) => CalendarCursor::Sync {
                token,
                page: Some(page),
            },
            (CalendarCursor::Initial { .. } | CalendarCursor::Sync { .. }, None) => {
                CalendarCursor::Sync {
                    token: page.next_sync_token.ok_or(CollectorError::Source)?,
                    page: None,
                }
            }
        };
        let caught_up = matches!(next, CalendarCursor::Sync { page: None, .. });
        let coverage = if caught_up {
            vec![report(
                account,
                Source::Calendar,
                "effects",
                CoverageState::Healthy,
                None,
                None,
            )]
        } else {
            Vec::new()
        };
        let position = CollectorCursor {
            account: account.into(),
            source: Source::Calendar,
            value: next.encode()?,
            updated_at: if caught_up {
                now
            } else {
                caught_up_at.unwrap_or(now)
            },
        };
        commit(store, events, Some(position), snapshots, coverage, now)?;
        if caught_up {
            return Ok(Progress::CaughtUp);
        }
        cursor = next;
    }
    Ok(Progress::Behind)
}

/// Re-lists the rolling window daily so unchanged events that enter it become known.
/// It keeps the incremental cursor; a window too large to list is a visible gap.
pub fn refresh_calendar_window(
    api: &impl GoogleApi,
    store: &mut impl Ledger,
    account: &str,
    token: &str,
    now: DateTime<Utc>,
) -> Result<(), CollectorError> {
    let checkpoint = store
        .checkpoint(account, &Source::Calendar)
        .map_err(|_| CollectorError::Store)?
        .ok_or(CollectorError::Source)?;
    if !matches!(
        CalendarCursor::decode(&checkpoint.value)?,
        CalendarCursor::Sync { page: None, .. }
    ) {
        return Err(CollectorError::Source);
    }
    let from = (now - Duration::days(30)).to_rfc3339();
    let to = (now + Duration::days(90)).to_rfc3339();
    let mut page_token = None;
    for _ in 0..WINDOW_PAGES {
        let page = api
            .calendar_list(token, None, page_token.as_deref(), Some((&from, &to)))
            .map_err(|source| CollectorError::Http {
                operation: "calendar window",
                source,
            })?;
        let items = select_calendar_items(store, account, page.items, now)?;
        let (events, snapshots) =
            calendar_observations(store, account, items, Origin::Baseline, now)?;
        commit(
            store,
            events,
            Some(checkpoint.clone()),
            snapshots,
            Vec::new(),
            now,
        )?;
        page_token = page.next_page_token;
        if page_token.is_none() {
            return Ok(());
        }
    }
    let gap = report(
        account,
        Source::Calendar,
        "effects",
        CoverageState::Gap,
        Some(CoverageReason::PageLimit),
        Some(checkpoint.updated_at),
    );
    commit(store, Vec::new(), None, Vec::new(), vec![gap], now)?;
    Err(CollectorError::Limit)
}

fn calendar_observations(
    store: &impl Ledger,
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
    store: &impl Ledger,
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
    use crate::{engine::FindingDetail, event::EventPayload, store::Store};
    use std::{collections::VecDeque, sync::Mutex};

    struct FakeApi {
        calendar: Mutex<VecDeque<Result<CalendarPage, HttpError>>>,
        history: Mutex<VecDeque<Result<GmailHistory, HttpError>>>,
        sent_more: bool,
        message: GmailMessage,
    }

    impl FakeApi {
        fn new() -> Result<Self, serde_json::Error> {
            Ok(Self {
                calendar: Mutex::new(VecDeque::new()),
                history: Mutex::new(VecDeque::new()),
                sent_more: false,
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
                    vec![crate::google::GmailRef {
                        id: "abc123".into(),
                        label_ids: Vec::new(),
                    }]
                } else {
                    Vec::new()
                },
                next_page_token: (label == "SENT" && self.sent_more).then(|| "more".into()),
            })
        }
        fn gmail_message(&self, _: &str, id: &str) -> Result<GmailMessage, HttpError> {
            if id.starts_with("gone") {
                return Err(HttpError::MissingResource);
            }
            let mut message = self.message.clone();
            message.id = id.into();
            message.thread_id = id.into();
            Ok(message)
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

    fn test_store()
    -> Result<(Store, std::path::PathBuf, tempfile::TempDir), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("picket.db");
        Ok((Store::open(&path)?, path, directory))
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
        let (mut store, path, _directory) = test_store()?;
        let api = FakeApi::new()?;
        let now = DateTime::parse_from_rfc3339("2026-10-22T12:00:00Z")?.with_timezone(&Utc);
        for _ in 0..2 {
            api.history.lock().map_err(|_| "lock")?.push_back(Ok(serde_json::from_value(serde_json::json!({
                "historyId": "11", "history": [{"id": "11", "messagesAdded": [{"message": {"id": "abc123"}}]}]
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
    fn limited_gmail_baseline_records_gap_and_keeps_live_cursor()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut store = Store::memory()?;
        let mut api = FakeApi::new()?;
        api.sent_more = true;
        api.history
            .lock()
            .map_err(|_| "lock")?
            .push_back(Ok(serde_json::from_value(
                serde_json::json!({"historyId": "11"}),
            )?));
        let now = DateTime::parse_from_rfc3339("2026-10-22T12:00:00Z")?.with_timezone(&Utc);
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
            Some("11")
        );
        assert!(store.events()?.iter().any(|record| matches!(
            &record.event.payload,
            EventPayload::Coverage(Coverage {
                capability,
                state: CoverageState::Gap,
                reason: Some(CoverageReason::PageLimit),
                ..
            }) if capability == "baseline"
        )));
        Ok(())
    }

    #[test]
    fn calendar_revision_and_cursor_survive_restart() -> Result<(), Box<dyn std::error::Error>> {
        let (mut store, path, _directory) = test_store()?;
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
        assert!(
            store
                .current_findings()?
                .iter()
                .any(|(_, finding)| matches!(finding.detail, FindingDetail::Effect(_)))
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

    fn count(store: &Store, predicate: fn(&EventPayload) -> bool) -> usize {
        store
            .events()
            .map(|rows| {
                rows.iter()
                    .filter(|row| predicate(&row.event.payload))
                    .count()
            })
            .unwrap_or_default()
    }

    #[test]
    fn gmail_backlog_resumes_from_the_last_processed_record()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut store = Store::memory()?;
        let api = FakeApi::new()?;
        let now = DateTime::parse_from_rfc3339("2026-10-22T12:00:00Z")?.with_timezone(&Utc);
        let records: Vec<_> = (0..150)
            .map(|index| {
                serde_json::json!({
                    "id": format!("{}", 100 + index),
                    "messagesAdded": [{"message": {"id": format!("m{index}")}}]
                })
            })
            .collect();
        {
            let mut history = api.history.lock().map_err(|_| "lock")?;
            history.push_back(Ok(serde_json::from_value(
                serde_json::json!({"historyId": "300", "history": records}),
            )?));
            history.push_back(Ok(serde_json::from_value(serde_json::json!({
                "historyId": "300", "history": records[MESSAGES_PER_POLL..]
            }))?));
        }
        let progress = poll_gmail(
            &api,
            &mut store,
            "synthetic-account",
            "sender@example.test",
            "token",
            now,
        )?;
        assert_eq!(progress, Progress::Behind);
        let resumed = format!("{}", 100 + MESSAGES_PER_POLL - 1);
        assert_eq!(
            store
                .collector_cursor("synthetic-account", &Source::Gmail)?
                .as_deref(),
            Some(resumed.as_str())
        );
        let progress = poll_gmail(
            &api,
            &mut store,
            "synthetic-account",
            "sender@example.test",
            "token",
            now,
        )?;
        assert_eq!(progress, Progress::CaughtUp);
        assert_eq!(
            store
                .collector_cursor("synthetic-account", &Source::Gmail)?
                .as_deref(),
            Some("300")
        );
        assert_eq!(
            count(&store, |payload| matches!(payload, EventPayload::Effect(_))),
            151
        );
        assert_eq!(
            count(&store, |payload| matches!(
                payload,
                EventPayload::Coverage(Coverage {
                    state: CoverageState::Gap,
                    ..
                })
            )),
            0,
            "a backlog is not a gap"
        );
        Ok(())
    }

    #[test]
    fn idle_polls_and_repeated_failures_add_no_events() -> Result<(), Box<dyn std::error::Error>> {
        let mut store = Store::memory()?;
        let api = FakeApi::new()?;
        let now = DateTime::parse_from_rfc3339("2026-10-22T12:00:00Z")?.with_timezone(&Utc);
        for _ in 0..20 {
            api.history
                .lock()
                .map_err(|_| "lock")?
                .push_back(Ok(serde_json::from_value(
                    serde_json::json!({"historyId": "11"}),
                )?));
        }
        let poll = |store: &mut Store, minute| {
            poll_gmail(
                &api,
                store,
                "synthetic-account",
                "sender@example.test",
                "token",
                now + Duration::minutes(minute),
            )
        };
        poll(&mut store, 0)?;
        let settled = store.events()?.len();
        for minute in 1..10 {
            poll(&mut store, minute)?;
        }
        assert_eq!(store.events()?.len(), settled);
        for minute in 10..15 {
            record_failure(
                &mut store,
                "synthetic-account",
                Source::Gmail,
                CoverageReason::RequestFailed,
                now + Duration::minutes(minute),
            )?;
        }
        assert_eq!(store.events()?.len(), settled + 1);
        poll(&mut store, 15)?;
        assert_eq!(store.events()?.len(), settled + 2);
        Ok(())
    }

    #[test]
    fn large_calendar_listing_resumes_with_its_page_token() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut store = Store::memory()?;
        let api = FakeApi::new()?;
        let now = DateTime::parse_from_rfc3339("2026-10-22T12:00:00Z")?.with_timezone(&Utc);
        {
            let mut pages = api.calendar.lock().map_err(|_| "lock")?;
            pages.push_back(Ok(calendar_page("rev1", None)?));
            for _ in 0..PAGES_PER_POLL + 3 {
                let mut page = calendar_page("rev1", None)?;
                page.next_page_token = Some("next".into());
                pages.push_back(Ok(page));
            }
            pages.push_back(Ok(calendar_page("rev1", Some("sync1"))?));
        }
        assert_eq!(
            poll_calendar(&api, &mut store, "synthetic-account", "token", now)?,
            Progress::Behind
        );
        let position = store
            .collector_cursor("synthetic-account", &Source::Calendar)?
            .ok_or("cursor")?;
        assert_eq!(
            CalendarCursor::decode(&position)?,
            CalendarCursor::Initial {
                origin: Origin::Baseline,
                page: Some("next".into())
            }
        );
        assert_eq!(
            poll_calendar(&api, &mut store, "synthetic-account", "token", now)?,
            Progress::CaughtUp
        );
        assert_eq!(
            store
                .collector_cursor("synthetic-account", &Source::Calendar)?
                .as_deref(),
            Some("sync1")
        );
        Ok(())
    }

    #[test]
    fn a_replaced_draft_is_not_a_gap_but_a_vanished_send_is()
    -> Result<(), Box<dyn std::error::Error>> {
        let now = DateTime::parse_from_rfc3339("2026-10-22T12:00:00Z")?.with_timezone(&Utc);
        let gaps = |labels: serde_json::Value| -> Result<usize, Box<dyn std::error::Error>> {
            let mut store = Store::memory()?;
            let api = FakeApi::new()?;
            api.history
                .lock()
                .map_err(|_| "lock")?
                .push_back(Ok(serde_json::from_value(serde_json::json!({
                    "historyId": "12",
                    "history": [{"id": "12", "messagesAdded": [
                        {"message": {"id": "gone1", "labelIds": labels}}
                    ]}]
                }))?));
            poll_gmail(
                &api,
                &mut store,
                "synthetic-account",
                "sender@example.test",
                "token",
                now,
            )?;
            Ok(count(&store, |payload| {
                matches!(
                    payload,
                    EventPayload::Coverage(Coverage {
                        reason: Some(CoverageReason::MissingResource),
                        ..
                    })
                )
            }))
        };
        assert_eq!(gaps(serde_json::json!(["DRAFT"]))?, 0);
        assert_eq!(gaps(serde_json::json!(["SENT"]))?, 1);
        assert_eq!(gaps(serde_json::json!([]))?, 1);
        Ok(())
    }
}
