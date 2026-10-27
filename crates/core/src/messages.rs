//! Read-only capture of registered agents' 1:1 conversations from the macOS Messages
//! database. Text is decoded in memory for a bounded snippet and never stored.

use std::{collections::BTreeMap, path::Path};

use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OpenFlags, params_from_iter};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    agent,
    event::{
        AdapterProvenance, AgentMode, AgentRegistration, Coverage, CoverageReason, CoverageState,
        Event, EventKey, EventPayload, MessageDirection, MessageObserved, MessageText, Origin,
        Source, SourceTimeQuality,
    },
    ledger::{Ledger, Progress},
    store::{Batch, CollectorCursor, CoverageReport},
};

/// Rows read per poll; a larger backlog continues on the next poll.
const ROWS_PER_POLL: usize = 500;
/// A newly registered agent's recent history, for standing instructions.
const BASELINE_DAYS: i64 = 30;
const BASELINE_ROWS: usize = 2_000;
const MAX_BODY: usize = 256 * 1024;
/// Seconds between the Unix and Apple (2001-01-01 UTC) epochs.
const APPLE_EPOCH: i64 = 978_307_200;

#[derive(Debug, Error)]
pub enum MessagesError {
    #[error("Messages database needs Full Disk Access")]
    PermissionMissing,
    #[error("Messages database format is not supported")]
    Unsupported,
    #[error("Messages database could not be read")]
    Read,
    #[error("local event store is unavailable")]
    Store,
}

/// One row from a registered agent's conversation.
#[derive(Clone, Debug)]
struct Row {
    rowid: i64,
    guid: String,
    text: Option<String>,
    body: Option<Vec<u8>>,
    from_me: bool,
    date: i64,
    edited: i64,
    retracted: i64,
    service: String,
    handle: i64,
    reaction: bool,
}

/// A read-only Messages database connection with its verified optional columns.
pub struct ChatDb {
    connection: Connection,
    edited: &'static str,
    retracted: &'static str,
}

impl ChatDb {
    pub fn open(path: &Path) -> Result<Self, MessagesError> {
        // Without Full Disk Access, macOS denies the file itself.
        if let Err(error) = std::fs::File::open(path) {
            return Err(if error.kind() == std::io::ErrorKind::PermissionDenied {
                MessagesError::PermissionMissing
            } else {
                MessagesError::Read
            });
        }
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|_| MessagesError::Read)?;
        let columns = |table: &str| -> Result<Vec<String>, MessagesError> {
            let mut statement = connection
                .prepare(&format!("PRAGMA table_info({table})"))
                .map_err(|_| MessagesError::Read)?;
            statement
                .query_map([], |row| row.get::<_, String>(1))
                .and_then(Iterator::collect)
                .map_err(|_| MessagesError::Read)
        };
        let message = columns("message")?;
        for (table, names) in [
            (
                "message",
                &[
                    "ROWID",
                    "guid",
                    "text",
                    "attributedBody",
                    "is_from_me",
                    "date",
                    "service",
                    "associated_message_type",
                ][..],
            ),
            ("handle", &["ROWID", "id"][..]),
            ("chat_handle_join", &["chat_id", "handle_id"][..]),
            ("chat_message_join", &["chat_id", "message_id"][..]),
        ] {
            let present = if table == "message" {
                message.clone()
            } else {
                columns(table)?
            };
            if names
                .iter()
                .any(|name| !present.iter().any(|column| column == name))
            {
                return Err(MessagesError::Unsupported);
            }
        }
        // Edits and unsends exist since macOS 13; older databases have neither.
        let has = |name: &str| message.iter().any(|column| column == name);
        let edited = if has("date_edited") {
            "COALESCE(m.date_edited, 0)"
        } else {
            "0"
        };
        let retracted = if has("date_retracted") {
            "COALESCE(m.date_retracted, 0)"
        } else {
            "0"
        };
        Ok(Self {
            connection,
            edited,
            retracted,
        })
    }

    /// Substitutes the optional edit and unsend columns into a query fragment.
    fn columns(&self, fragment: &str) -> String {
        fragment
            .replace("EDITED", self.edited)
            .replace("RETRACTED", self.retracted)
    }

    fn handles(&self) -> Result<Vec<(i64, String)>, MessagesError> {
        let mut statement = self
            .connection
            .prepare("SELECT ROWID, id FROM handle")
            .map_err(|_| MessagesError::Read)?;
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .and_then(Iterator::collect)
            .map_err(|_| MessagesError::Read)
    }

    /// Rows of 1:1 conversations with the given handles that match `condition`.
    fn rows(
        &self,
        handles: &[i64],
        condition: &str,
        bound: i64,
        order: &str,
        limit: usize,
    ) -> Result<Vec<Row>, MessagesError> {
        if handles.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = vec!["?"; handles.len()].join(",");
        let sql = format!(
            "SELECT m.ROWID, m.guid, m.text, m.attributedBody, m.is_from_me, m.date,
                    {edited}, {retracted}, COALESCE(m.service, ''), chj.handle_id,
                    m.associated_message_type
             FROM message m
             JOIN chat_message_join cmj ON cmj.message_id = m.ROWID
             JOIN chat_handle_join chj ON chj.chat_id = cmj.chat_id
             WHERE chj.handle_id IN ({placeholders}) AND {condition}
               AND (SELECT COUNT(*) FROM chat_handle_join x WHERE x.chat_id = cmj.chat_id) = 1
             ORDER BY {order} LIMIT {limit}",
            edited = self.edited,
            retracted = self.retracted,
            condition = self.columns(condition),
            order = self.columns(order),
        );
        let mut statement = self
            .connection
            .prepare(&sql)
            .map_err(|_| MessagesError::Read)?;
        let values = handles.iter().copied().chain(std::iter::once(bound));
        statement
            .query_map(params_from_iter(values), |row| {
                Ok(Row {
                    rowid: row.get(0)?,
                    guid: row.get(1)?,
                    text: row.get(2)?,
                    body: row.get(3)?,
                    from_me: row.get::<_, i64>(4)? != 0,
                    date: row.get(5)?,
                    edited: row.get(6)?,
                    retracted: row.get(7)?,
                    service: row.get(8)?,
                    handle: row.get(9)?,
                    reaction: row.get::<_, i64>(10)? != 0,
                })
            })
            .and_then(Iterator::collect)
            .map_err(|_| MessagesError::Read)
    }
}

/// Extracts the plain text of an `NSAttributedString` typedstream: the first string
/// object's bytes follow a `+` type tag and a typedstream length. Anything malformed or
/// outside bounds yields `None`, which is recorded as undecodable, never guessed.
pub fn decode_body(blob: &[u8]) -> Option<String> {
    if blob.len() > MAX_BODY {
        return None;
    }
    let class = [&b"NSString"[..], &b"NSMutableString"[..]]
        .iter()
        .filter_map(|name| {
            blob.windows(name.len())
                .position(|window| window == *name)
                .map(|at| at + name.len())
        })
        .min()?;
    let rest = blob.get(class..)?;
    let tag = rest.iter().take(16).position(|byte| *byte == b'+')?;
    let rest = rest.get(tag + 1..)?;
    let (length, header): (usize, usize) = match *rest.first()? {
        0x81 => (
            usize::from(u16::from_le_bytes([*rest.get(1)?, *rest.get(2)?])),
            3,
        ),
        0x82 => (
            usize::try_from(u32::from_le_bytes([
                *rest.get(1)?,
                *rest.get(2)?,
                *rest.get(3)?,
                *rest.get(4)?,
            ]))
            .ok()?,
            5,
        ),
        byte if byte < 0x80 => (usize::from(byte), 1),
        _ => return None,
    };
    let bytes = rest.get(header..header.checked_add(length)?)?;
    String::from_utf8(bytes.to_vec()).ok()
}

/// Messages stores nanoseconds since 2001 (seconds before macOS 10.13).
pub fn apple_time(value: i64) -> Option<DateTime<Utc>> {
    if value <= 0 {
        return None;
    }
    let seconds = if value > 10_000_000_000 {
        value / 1_000_000_000
    } else {
        value
    };
    let nanos = if value > 10_000_000_000 {
        u32::try_from(value % 1_000_000_000).ok()?
    } else {
        0
    };
    DateTime::from_timestamp(seconds.checked_add(APPLE_EPOCH)?, nanos)
}

fn apple_value(time: DateTime<Utc>) -> i64 {
    (time.timestamp() - APPLE_EPOCH) * 1_000_000_000
}

/// Durable Messages position: new rows after `rowid`, edits and unsends after `changed`,
/// and the agents whose recent history has been imported.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
struct MessagesCursor {
    rowid: i64,
    changed: i64,
    agents: Vec<String>,
}

/// Captures new, edited and retracted messages in registered agents' conversations.
pub fn poll_messages(
    db: &ChatDb,
    ledger: &mut impl Ledger,
    account: &str,
    agents: &[AgentRegistration],
    region: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Progress, MessagesError> {
    let agents: Vec<_> = agents
        .iter()
        .filter(|agent| agent.enabled && agent.mode == AgentMode::Messages)
        .collect();
    if agents.is_empty() {
        return Ok(Progress::CaughtUp);
    }
    let mut owners = BTreeMap::new();
    for (rowid, raw) in db.handles()? {
        if let Ok(handle) = agent::normalize_handle(&raw, region)
            && let Some(agent) = agents.iter().find(|agent| agent.handles.contains(&handle))
        {
            owners.insert(rowid, agent.id.clone());
        }
    }
    let checkpoint = ledger
        .checkpoint(account, &Source::Messages)
        .map_err(|_| MessagesError::Store)?;
    let mut cursor: MessagesCursor = match &checkpoint {
        Some(value) => serde_json::from_str(&value.value).map_err(|_| MessagesError::Store)?,
        None => MessagesCursor::default(),
    };
    let caught_up_at = checkpoint.map(|value| value.updated_at);
    let mut events = Vec::new();
    for agent in &agents {
        if cursor.agents.contains(&agent.id) {
            continue;
        }
        let handles: Vec<_> = owners
            .iter()
            .filter(|(_, owner)| **owner == agent.id)
            .map(|(rowid, _)| *rowid)
            .collect();
        let cutoff = apple_value(now - Duration::days(BASELINE_DAYS));
        for row in db.rows(&handles, "m.date >= ?", cutoff, "m.ROWID", BASELINE_ROWS)? {
            events.extend(observe(&row, &owners, account, Origin::Baseline, now));
        }
        cursor.agents.push(agent.id.clone());
    }
    let handles: Vec<_> = owners.keys().copied().collect();
    let fresh = db.rows(
        &handles,
        "m.ROWID > ?",
        cursor.rowid,
        "m.ROWID",
        ROWS_PER_POLL,
    )?;
    let changed = db.rows(
        &handles,
        "MAX(EDITED, RETRACTED) > ?",
        cursor.changed,
        "MAX(EDITED, RETRACTED)",
        ROWS_PER_POLL,
    )?;
    let caught_up = fresh.len() < ROWS_PER_POLL && changed.len() < ROWS_PER_POLL;
    for row in &fresh {
        let origin = apple_time(row.date)
            .filter(|at| now - *at <= Duration::minutes(2))
            .map_or(Origin::CatchUp, |_| Origin::Live);
        events.extend(observe(row, &owners, account, origin, now));
        cursor.rowid = cursor.rowid.max(row.rowid);
    }
    for row in &changed {
        events.extend(observe(row, &owners, account, Origin::Live, now));
        cursor.changed = cursor.changed.max(row.edited.max(row.retracted));
    }
    let coverage = if caught_up {
        agents
            .iter()
            .map(|agent| capture_report(account, &agent.id, CoverageState::Healthy, None))
            .collect()
    } else {
        Vec::new()
    };
    let value = serde_json::to_string(&cursor).map_err(|_| MessagesError::Store)?;
    let position = CollectorCursor {
        account: account.into(),
        source: Source::Messages,
        value,
        updated_at: if caught_up {
            now
        } else {
            caught_up_at.unwrap_or(now)
        },
    };
    dedupe(&mut events);
    ledger
        .commit(
            Batch {
                events,
                cursor: Some(position),
                calendar: Vec::new(),
                coverage,
            },
            now,
        )
        .map_err(|_| MessagesError::Store)?;
    Ok(if caught_up {
        Progress::CaughtUp
    } else {
        Progress::Behind
    })
}

/// Records why capture is unavailable for every registered Messages agent.
pub fn record_unavailable(
    ledger: &mut impl Ledger,
    account: &str,
    agents: &[AgentRegistration],
    error: &MessagesError,
    now: DateTime<Utc>,
) -> Result<(), MessagesError> {
    let (state, reason) = match error {
        MessagesError::PermissionMissing => {
            (CoverageState::Disabled, CoverageReason::PermissionMissing)
        }
        MessagesError::Unsupported => (CoverageState::Disabled, CoverageReason::UnsupportedFormat),
        MessagesError::Read | MessagesError::Store => {
            (CoverageState::Delayed, CoverageReason::RequestFailed)
        }
    };
    let coverage = agents
        .iter()
        .filter(|agent| agent.enabled && agent.mode == AgentMode::Messages)
        .map(|agent| capture_report(account, &agent.id, state.clone(), Some(reason.clone())))
        .collect();
    ledger
        .commit(
            Batch {
                coverage,
                ..Batch::default()
            },
            now,
        )
        .map(|_| ())
        .map_err(|_| MessagesError::Store)
}

fn capture_report(
    account: &str,
    agent: &str,
    state: CoverageState,
    reason: Option<CoverageReason>,
) -> CoverageReport {
    CoverageReport {
        account: account.into(),
        source: Source::Messages,
        coverage: Coverage {
            capability: format!("messages_{agent}"),
            state,
            cursor: None,
            gap_since: None,
            reason,
            partial_pages: None,
        },
    }
}

/// One observation per message revision; reactions are not requests or reports.
fn observe(
    row: &Row,
    owners: &BTreeMap<i64, String>,
    account: &str,
    origin: Origin,
    now: DateTime<Utc>,
) -> Option<Event> {
    if row.reaction || row.guid.is_empty() || row.guid.len() > 128 {
        return None;
    }
    let agent = owners.get(&row.handle)?.clone();
    let (revision, at) = if row.retracted > 0 {
        (format!("retracted-{}", row.retracted), row.retracted)
    } else if row.edited > 0 {
        (format!("edited-{}", row.edited), row.edited)
    } else {
        ("0".to_owned(), row.date)
    };
    let decoded = row
        .text
        .clone()
        .filter(|text| !text.trim().is_empty())
        .map(Some)
        .or_else(|| row.body.as_deref().map(decode_body));
    let (text, snippet) = match decoded {
        _ if row.retracted > 0 => (MessageText::Retracted, String::new()),
        Some(Some(text)) => (MessageText::Decoded, agent::snippet(&text)),
        Some(None) => (MessageText::Undecodable, String::new()),
        None => (MessageText::Absent, String::new()),
    };
    let occurred = apple_time(at);
    Some(Event {
        schema_version: 1,
        key: EventKey {
            account: account.into(),
            source: Source::Messages,
            resource: row.guid.clone(),
            revision,
        },
        occurred_at: occurred.unwrap_or(now),
        ingested_at: now,
        source_time_quality: Some(if occurred.is_some() {
            SourceTimeQuality::Provider
        } else {
            SourceTimeQuality::IngestedFallback
        }),
        adapter: Some(AdapterProvenance {
            name: "messages".into(),
            version: 1,
            scope: "chat.db read-only".into(),
        }),
        payload: EventPayload::Message(MessageObserved {
            agent,
            direction: if row.from_me {
                MessageDirection::FromUser
            } else {
                MessageDirection::FromAgent
            },
            service: row.service.chars().take(32).collect(),
            text,
            snippet,
            origin,
        }),
    })
}

/// A baseline and a poll can read the same revision; keep the first.
fn dedupe(events: &mut Vec<Event>) {
    let mut seen = std::collections::BTreeSet::new();
    events.retain(|event| seen.insert((event.key.resource.clone(), event.key.revision.clone())));
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::store::Store;

    /// A typedstream `NSAttributedString` body with the structure Messages writes.
    pub(crate) fn body(text: &str) -> Vec<u8> {
        let mut bytes = b"\x04\x0bstreamtyped\x81\xe8\x03\x84\x01@\x84\x84\x84\x12NSAttributedString\x00\x84\x84\x08NSObject\x00\x85\x92\x84\x84\x84\x08NSString\x01\x94\x84\x01+".to_vec();
        let length = text.len();
        if length < 0x80 {
            bytes.push(length as u8);
        } else {
            bytes.push(0x81);
            bytes.extend((length as u16).to_le_bytes());
        }
        bytes.extend(text.as_bytes());
        bytes.extend(b"\x86\x84\x02iI\x01\x05\x92\x84\x84\x84\x0cNSDictionary\x00");
        bytes
    }

    /// Builds a synthetic Messages database with the tables and columns Picket reads.
    pub(crate) struct Fixture {
        pub path: std::path::PathBuf,
        connection: Connection,
        _directory: tempfile::TempDir,
    }

    impl Fixture {
        pub fn new() -> Self {
            let directory = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
            let path = directory.path().join("chat.db");
            let connection = Connection::open(&path).unwrap_or_else(|error| panic!("{error}"));
            connection
                .execute_batch(
                    "CREATE TABLE handle (ROWID INTEGER PRIMARY KEY, id TEXT, service TEXT);
                     CREATE TABLE chat (ROWID INTEGER PRIMARY KEY, guid TEXT, style INTEGER);
                     CREATE TABLE chat_handle_join (chat_id INTEGER, handle_id INTEGER);
                     CREATE TABLE chat_message_join (chat_id INTEGER, message_id INTEGER);
                     CREATE TABLE message (ROWID INTEGER PRIMARY KEY AUTOINCREMENT, guid TEXT,
                       text TEXT, attributedBody BLOB, handle_id INTEGER, service TEXT,
                       is_from_me INTEGER, date INTEGER, associated_message_type INTEGER,
                       date_edited INTEGER, date_retracted INTEGER);",
                )
                .unwrap_or_else(|error| panic!("{error}"));
            Self {
                path,
                connection,
                _directory: directory,
            }
        }

        pub fn chat(&self, chat: i64, handles: &[(i64, &str)]) {
            self.connection
                .execute(
                    "INSERT INTO chat VALUES (?1, ?2, 45)",
                    (chat, format!("chat-{chat}")),
                )
                .unwrap_or_else(|error| panic!("{error}"));
            for (rowid, id) in handles {
                self.connection
                    .execute(
                        "INSERT OR IGNORE INTO handle VALUES (?1, ?2, 'iMessage')",
                        (rowid, id),
                    )
                    .unwrap_or_else(|error| panic!("{error}"));
                self.connection
                    .execute(
                        "INSERT INTO chat_handle_join VALUES (?1, ?2)",
                        (chat, rowid),
                    )
                    .unwrap_or_else(|error| panic!("{error}"));
            }
        }

        /// Adds a message; `body` stores the text only in `attributedBody`.
        pub fn message(
            &self,
            chat: i64,
            guid: &str,
            text: &str,
            from_me: bool,
            at: DateTime<Utc>,
            body_only: bool,
        ) -> i64 {
            let (column, blob) = if body_only {
                (None, Some(body(text)))
            } else {
                (Some(text.to_owned()), None)
            };
            self.connection
                .execute(
                    "INSERT INTO message (guid, text, attributedBody, handle_id, service, is_from_me,
                       date, associated_message_type, date_edited, date_retracted)
                     VALUES (?1, ?2, ?3, 0, 'iMessage', ?4, ?5, 0, 0, 0)",
                    (guid, column, blob, i64::from(from_me), apple_value(at)),
                )
                .unwrap_or_else(|error| panic!("{error}"));
            let rowid = self.connection.last_insert_rowid();
            self.connection
                .execute(
                    "INSERT INTO chat_message_join VALUES (?1, ?2)",
                    (chat, rowid),
                )
                .unwrap_or_else(|error| panic!("{error}"));
            rowid
        }

        pub fn execute(&self, sql: &str) {
            self.connection
                .execute_batch(sql)
                .unwrap_or_else(|error| panic!("{error}"));
        }
    }

    fn agent() -> AgentRegistration {
        AgentRegistration {
            id: "instinct".into(),
            name: "Instinct".into(),
            mode: AgentMode::Messages,
            handles: vec!["+14155550132".into()],
            connected_since: None,
            enabled: true,
        }
    }

    fn at(minute: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_790_000_000 + minute * 60, 0).unwrap_or_default()
    }

    fn messages(store: &Store) -> Vec<(String, MessageObserved)> {
        store
            .events()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|row| match row.event.payload {
                EventPayload::Message(value) => Some((row.event.key.revision, value)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn decoder_reads_short_and_long_bodies_and_rejects_malformed_ones() {
        assert_eq!(
            decode_body(&body("Book a table")),
            Some("Book a table".into())
        );
        let long = "é".repeat(400);
        assert_eq!(decode_body(&body(&long)), Some(long));
        let mut truncated = body("Book a table");
        truncated.truncate(truncated.len() - 30);
        assert_eq!(decode_body(&truncated), None);
        assert_eq!(decode_body(b"no string here"), None);
        let mut invalid = body("ab");
        // The fixture trailer after the text is 25 bytes.
        let at = invalid.len() - 25 - 2;
        invalid[at] = 0xff;
        assert_eq!(decode_body(&invalid), None);
        assert_eq!(decode_body(&vec![0; MAX_BODY + 1]), None);
    }

    #[test]
    fn apple_time_handles_nanoseconds_seconds_and_missing_dates() {
        let instant = at(0);
        assert_eq!(apple_time(apple_value(instant)), Some(instant));
        assert_eq!(apple_time(instant.timestamp() - APPLE_EPOCH), Some(instant));
        assert_eq!(apple_time(0), None);
    }

    #[test]
    fn captures_only_the_agents_one_to_one_conversation_with_directions_and_times()
    -> Result<(), Box<dyn std::error::Error>> {
        let fixture = Fixture::new();
        fixture.chat(1, &[(1, "+14155550132")]);
        fixture.chat(2, &[(2, "friend@example.test")]);
        fixture.chat(3, &[(1, "+14155550132"), (2, "friend@example.test")]);
        fixture.message(1, "m1", "Book a table for two at 7", true, at(0), true);
        fixture.message(1, "m2", "Booked for 7pm", false, at(1), false);
        fixture.message(2, "m3", "private conversation", true, at(2), false);
        fixture.message(3, "m4", "group conversation", true, at(3), false);
        let db = ChatDb::open(&fixture.path)?;
        let mut store = Store::memory()?;
        let progress = poll_messages(&db, &mut store, "account-1", &[agent()], Some("US"), at(5))?;
        assert_eq!(progress, Progress::CaughtUp);
        let captured = messages(&store);
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0].1.direction, MessageDirection::FromUser);
        assert_eq!(captured[0].1.snippet, "Book a table for two at 7");
        assert_eq!(captured[0].1.origin, Origin::Baseline);
        assert_eq!(captured[1].1.direction, MessageDirection::FromAgent);
        let times: Vec<_> = store
            .events()?
            .into_iter()
            .filter(|row| matches!(row.event.payload, EventPayload::Message(_)))
            .map(|row| row.event.occurred_at)
            .collect();
        assert_eq!(times, vec![at(0), at(1)]);
        Ok(())
    }

    #[test]
    fn new_edited_and_retracted_messages_become_revisions_once()
    -> Result<(), Box<dyn std::error::Error>> {
        let fixture = Fixture::new();
        fixture.chat(1, &[(1, "+14155550132")]);
        let first = fixture.message(1, "m1", "Send it to Sam", true, at(0), false);
        let db = ChatDb::open(&fixture.path)?;
        let mut store = Store::memory()?;
        poll_messages(&db, &mut store, "account-1", &[agent()], None, at(1))?;
        fixture.message(1, "m2", "Done", false, at(2), true);
        let edit = apple_value(at(3));
        fixture.execute(&format!(
            "UPDATE message SET text='Send it to Alex', date_edited={edit} WHERE ROWID={first}"
        ));
        poll_messages(&db, &mut store, "account-1", &[agent()], None, at(3))?;
        poll_messages(&db, &mut store, "account-1", &[agent()], None, at(4))?;
        let retract = apple_value(at(5));
        fixture.execute(&format!(
            "UPDATE message SET text=NULL, date_retracted={retract} WHERE guid='m2'"
        ));
        poll_messages(&db, &mut store, "account-1", &[agent()], None, at(6))?;
        let captured = messages(&store);
        let summary: Vec<_> = captured
            .iter()
            .map(|(revision, value)| {
                (
                    revision.split('-').next().unwrap_or_default(),
                    value.text.clone(),
                    value.snippet.as_str(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                ("0", MessageText::Decoded, "Send it to Sam"),
                ("0", MessageText::Decoded, "Done"),
                ("edited", MessageText::Decoded, "Send it to Alex"),
                ("retracted", MessageText::Retracted, ""),
            ]
        );
        Ok(())
    }

    #[test]
    fn a_large_backlog_resumes_and_reports_healthy_only_when_caught_up()
    -> Result<(), Box<dyn std::error::Error>> {
        let fixture = Fixture::new();
        fixture.chat(1, &[(1, "+14155550132")]);
        let db = ChatDb::open(&fixture.path)?;
        let mut store = Store::memory()?;
        poll_messages(&db, &mut store, "account-1", &[agent()], None, at(0))?;
        for index in 0..(ROWS_PER_POLL + 20) {
            fixture.message(1, &format!("m{index}"), "hello", false, at(1), false);
        }
        assert_eq!(
            poll_messages(&db, &mut store, "account-1", &[agent()], None, at(2))?,
            Progress::Behind
        );
        assert_eq!(
            poll_messages(&db, &mut store, "account-1", &[agent()], None, at(3))?,
            Progress::CaughtUp
        );
        assert_eq!(messages(&store).len(), ROWS_PER_POLL + 20);
        Ok(())
    }

    #[test]
    fn missing_tables_are_an_unsupported_format() -> Result<(), Box<dyn std::error::Error>> {
        let fixture = Fixture::new();
        fixture.execute("DROP TABLE chat_message_join;");
        assert!(matches!(
            ChatDb::open(&fixture.path),
            Err(MessagesError::Unsupported)
        ));
        let mut store = Store::memory()?;
        record_unavailable(
            &mut store,
            "account-1",
            &[agent()],
            &MessagesError::PermissionMissing,
            at(0),
        )?;
        assert!(
            store
                .events()?
                .iter()
                .any(|row| matches!(&row.event.payload,
            EventPayload::Coverage(Coverage { state: CoverageState::Disabled,
                reason: Some(CoverageReason::PermissionMissing), capability, .. })
                if capability == "messages_instinct"))
        );
        Ok(())
    }
}
