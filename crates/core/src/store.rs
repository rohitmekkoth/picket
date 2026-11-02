//! One mutable SQLite connection owns ordered log writes and retention.

use std::{collections::BTreeMap, path::Path};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::calendar::CalendarState;
use crate::event::{
    AgentRegistration, Coverage, CoverageState, Event, EventKey, EventPayload, Snapshot,
    SnapshotValue, Source, canonical_json,
};
use crate::{
    engine::{Assessor, Finding},
    incidents::{self, ENGINE_VERSION, OutboxBatch, ReconcileSummary},
};

const SCHEMA_VERSION: i64 = 6;
const MAX_EVENT_BYTES: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("SQLite: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid event: {0}")]
    Invalid(&'static str),
    #[error("same source revision has different content")]
    DuplicateConflict,
    #[error("unsupported database schema version {0}")]
    Schema(i64),
    #[error("store writer is not running")]
    Unavailable,
    #[error("log broken at sequence {sequence}: {reason}")]
    Corrupt { sequence: i64, reason: &'static str },
}

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Clone, Debug)]
pub struct StoredEvent {
    pub sequence: i64,
    pub event: Event,
    pub prev_hash: [u8; 32],
    pub hash: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Verification {
    pub through_sequence: i64,
    pub retained_events: u64,
}

/// A mutable store is the sole writer. Readers open separate read-only connections.
/// The in-memory assessor is derived from the log; it is rebuilt after any failed write.
pub struct Store {
    connection: Connection,
    assessor: Option<Assessor>,
}

/// One atomic collector write: observations, their cursor, Calendar state and coverage.
#[derive(Clone, Debug, Default)]
pub struct Batch {
    pub events: Vec<Event>,
    pub cursor: Option<CollectorCursor>,
    pub calendar: Vec<(String, CalendarState)>,
    pub coverage: Vec<CoverageReport>,
}

/// A collector's current coverage. The store logs it only when its state or reason
/// changes, or when an open claim needs a healthy confirmation; liveness stays in cursors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoverageReport {
    pub account: String,
    pub source: Source,
    pub coverage: Coverage,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Committed {
    pub sequences: Vec<i64>,
    pub summary: ReconcileSummary,
}

/// A source cursor advances only after every observation in its page is durable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollectorCursor {
    pub account: String,
    pub source: Source,
    pub value: String,
    pub updated_at: DateTime<Utc>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)?;
        Self::initialize(connection)
    }

    pub fn memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?)
    }

    pub fn open_read_only(path: &Path) -> Result<Self> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if !(1..=SCHEMA_VERSION).contains(&version) {
            return Err(StoreError::Schema(version));
        }
        Ok(Self {
            connection,
            assessor: None,
        })
    }

    fn initialize(connection: Connection) -> Result<Self> {
        // WAL with synchronous=NORMAL cannot corrupt the log. A power loss may roll back the
        // latest commits, and their cursors roll back with them, so sources are read again.
        connection.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;
             PRAGMA secure_delete=ON;",
        )?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        match version {
            0 => connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE events (
                   seq INTEGER PRIMARY KEY,
                   account TEXT NOT NULL,
                   source TEXT NOT NULL,
                   resource TEXT NOT NULL,
                   revision TEXT NOT NULL,
                   kind TEXT NOT NULL,
                   canonical BLOB NOT NULL,
                   prev_hash BLOB NOT NULL,
                   hash BLOB NOT NULL,
                   UNIQUE(account, source, resource, revision, kind)
                 );
                 CREATE TABLE chain_state (
                   id INTEGER PRIMARY KEY CHECK(id = 1),
                   anchor_seq INTEGER NOT NULL,
                   anchor_hash BLOB NOT NULL,
                   head_seq INTEGER NOT NULL,
                   head_hash BLOB NOT NULL
                 );
                 INSERT INTO chain_state VALUES (1, 0, zeroblob(32), 0, zeroblob(32));
                 PRAGMA user_version=1;
                 COMMIT;",
            )?,
            1..=SCHEMA_VERSION => {}
            other => return Err(StoreError::Schema(other)),
        }
        if version < 2 {
            connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE assessment_revisions (
                   finding TEXT NOT NULL,
                   revision INTEGER NOT NULL,
                   assessed_at TEXT NOT NULL,
                   engine_version INTEGER NOT NULL,
                   payload BLOB NOT NULL,
                   PRIMARY KEY(finding, revision)
                 );
                 CREATE TABLE assessment_current (
                   finding TEXT PRIMARY KEY,
                   revision INTEGER NOT NULL,
                   FOREIGN KEY(finding, revision) REFERENCES assessment_revisions(finding, revision)
                 );
                 CREATE TABLE notification_outbox (
                   id TEXT PRIMARY KEY,
                   finding TEXT NOT NULL,
                   revision INTEGER NOT NULL,
                   kind TEXT NOT NULL CHECK(kind IN ('alert', 'correction')),
                   state TEXT NOT NULL CHECK(state IN ('pending', 'delivered', 'cancelled')),
                   attempts INTEGER NOT NULL DEFAULT 0,
                   created_at TEXT NOT NULL,
                   FOREIGN KEY(finding, revision) REFERENCES assessment_revisions(finding, revision)
                 );
                 CREATE TABLE engine_state (
                   id INTEGER PRIMARY KEY CHECK(id = 1),
                   through_sequence INTEGER NOT NULL,
                   engine_version INTEGER NOT NULL
                 );
                 INSERT INTO engine_state VALUES (1, 0, 1);
                 PRAGMA user_version=2;
                 COMMIT;",
            )?;
        }
        if version < 3 {
            connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE collector_cursors (
                   account TEXT NOT NULL,
                   source TEXT NOT NULL,
                   value TEXT NOT NULL,
                   updated_at TEXT NOT NULL,
                   PRIMARY KEY(account, source)
                 );
                 PRAGMA user_version=3;
                 COMMIT;",
            )?;
        }
        if version < 4 {
            connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE calendar_snapshots (
                   account TEXT NOT NULL,
                   resource TEXT NOT NULL,
                   canonical BLOB NOT NULL,
                   PRIMARY KEY(account, resource)
                 );
                 PRAGMA user_version=4;
                 COMMIT;",
            )?;
        }
        if version < 5 {
            connection.execute_batch(
                "BEGIN IMMEDIATE;
                 ALTER TABLE engine_state ADD COLUMN clock TEXT;
                 PRAGMA user_version=5;
                 COMMIT;",
            )?;
        }
        if version < 6 {
            // Extraction reads pending messages and an agent's requests by kind.
            connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE INDEX IF NOT EXISTS events_kind ON events(kind, seq);
                 PRAGMA user_version=6;
                 COMMIT;",
            )?;
        }
        Ok(Self {
            connection,
            assessor: None,
        })
    }

    pub fn calendar_snapshot(
        &self,
        account: &str,
        resource: &str,
    ) -> Result<Option<CalendarState>> {
        let bytes: Option<Vec<u8>> = self
            .connection
            .query_row(
                "SELECT canonical FROM calendar_snapshots WHERE account=?1 AND resource=?2",
                params![account, resource],
                |row| row.get(0),
            )
            .optional()?;
        bytes
            .map(|bytes| serde_json::from_slice(&bytes).map_err(StoreError::from))
            .transpose()
    }

    pub fn collector_cursor(&self, account: &str, source: &Source) -> Result<Option<String>> {
        Ok(self
            .collector_checkpoint(account, source)?
            .map(|cursor| cursor.value))
    }

    pub fn collector_checkpoint(
        &self,
        account: &str,
        source: &Source,
    ) -> Result<Option<CollectorCursor>> {
        let row: Option<(String, String)> = self
            .connection
            .query_row(
                "SELECT value, updated_at FROM collector_cursors WHERE account=?1 AND source=?2",
                params![account, source.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        row.map(|(value, updated_at)| {
            let updated_at = DateTime::parse_from_rfc3339(&updated_at)
                .map_err(|_| StoreError::Invalid("invalid collector cursor time"))?
                .with_timezone(&Utc);
            Ok(CollectorCursor {
                account: account.into(),
                source: source.clone(),
                value,
                updated_at,
            })
        })
        .transpose()
    }

    pub fn event_exists(&self, key: &EventKey, kind: &str) -> Result<bool> {
        let exists: i64 = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM events WHERE account=?1 AND source=?2 AND resource=?3 AND revision=?4 AND kind=?5)",
            params![key.account, key.source.as_str(), key.resource, key.revision, kind],
            |row| row.get(0),
        )?;
        Ok(exists != 0)
    }

    /// Appends without assessment. The next assessed write catches up from the log.
    pub fn append(&mut self, event: &Event) -> Result<i64> {
        validate_event(event)?;
        self.assessor = None;
        let tx = self.connection.transaction()?;
        let sequence = append_in_transaction(&tx, event)?;
        tx.commit()?;
        Ok(sequence)
    }

    /// Appends an input and commits resulting assessments/outbox work together.
    pub fn append_assessed(
        &mut self,
        event: &Event,
        now: DateTime<Utc>,
    ) -> Result<(i64, ReconcileSummary)> {
        let batch = Batch {
            events: vec![event.clone()],
            ..Batch::default()
        };
        let committed = self.commit(batch, now)?;
        let sequence = committed
            .sequences
            .first()
            .copied()
            .ok_or(StoreError::Invalid("missing committed sequence"))?;
        Ok((sequence, committed.summary))
    }

    /// Reassesses findings whose deadlines passed, and any input not yet assessed.
    pub fn reconcile(&mut self, now: DateTime<Utc>) -> Result<ReconcileSummary> {
        Ok(self.commit(Batch::default(), now)?.summary)
    }

    /// Commits a batch and the assessments it changes in one transaction. Work is
    /// proportional to the batch and the findings it can affect, not to the log size.
    pub fn commit(&mut self, batch: Batch, now: DateTime<Utc>) -> Result<Committed> {
        validate_batch(&batch)?;
        if self.assessor.is_none() {
            self.assessor = Some(load_assessor(&self.connection)?);
        }
        let result = self.commit_loaded(&batch, now);
        if result.is_err() {
            self.assessor = None;
        }
        result
    }

    fn commit_loaded(&mut self, batch: &Batch, now: DateTime<Utc>) -> Result<Committed> {
        let Self {
            connection,
            assessor,
        } = self;
        let assessor = assessor
            .as_mut()
            .ok_or(StoreError::Invalid("assessment state unavailable"))?;
        let tx = connection.transaction()?;
        let previous_head = chain_state(&tx)?.2;
        let mut sequences = Vec::with_capacity(batch.events.len());
        for event in &batch.events {
            let sequence = append_in_transaction(&tx, event)?;
            if sequence > previous_head {
                assessor.observe(sequence, event.clone(), now);
            }
            sequences.push(sequence);
        }
        let mut changed =
            !batch.events.is_empty() || batch.cursor.is_some() || !batch.calendar.is_empty();
        for report in &batch.coverage {
            let next = chain_state(&tx)?.2 + 1;
            if let Some(event) = coverage_transition(assessor, report, next, now) {
                validate_event(&event)?;
                let sequence = append_in_transaction(&tx, &event)?;
                assessor.observe(sequence, event, now);
                changed = true;
            }
        }
        for (resource, snapshot) in &batch.calendar {
            let canonical = canonical_json(snapshot)?;
            if canonical.len() > MAX_EVENT_BYTES {
                return Err(StoreError::Invalid("Calendar snapshot exceeds 64 KiB"));
            }
            let account = batch
                .cursor
                .as_ref()
                .map(|cursor| cursor.account.as_str())
                .ok_or(StoreError::Invalid("snapshots require a Calendar cursor"))?;
            tx.execute(
                "INSERT INTO calendar_snapshots(account, resource, canonical) VALUES (?1, ?2, ?3)
                 ON CONFLICT(account, resource) DO UPDATE SET canonical=excluded.canonical",
                params![account, resource, canonical],
            )?;
        }
        if let Some(cursor) = &batch.cursor {
            tx.execute(
                "INSERT INTO collector_cursors(account, source, value, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(account, source) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
                params![cursor.account, cursor.source.as_str(), cursor.value, cursor.updated_at.to_rfc3339()],
            )?;
        }
        let findings = assessor.advance(now);
        let head = chain_state(&tx)?.2;
        // An idle tick that changes nothing skips the write. No deadline passed, so the
        // older durable clock makes a restart recheck only subjects that are unchanged.
        if !changed && findings.is_empty() {
            return Ok(Committed {
                sequences,
                summary: ReconcileSummary {
                    through_sequence: head,
                    assessed: 0,
                    revised: 0,
                    queued: 0,
                },
            });
        }
        let summary = incidents::persist(&tx, findings, now, head)?;
        tx.commit()?;
        Ok(Committed { sequences, summary })
    }

    /// The latest registration of every agent, including retained snapshots.
    pub fn agents(&self) -> Result<Vec<AgentRegistration>> {
        let mut statement = self.connection.prepare(
            "SELECT canonical FROM events WHERE kind IN ('agent', 'snapshot') ORDER BY seq",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
        let mut latest = BTreeMap::new();
        for row in rows {
            let event: Event = serde_json::from_slice(&row?)?;
            let agent = match event.payload {
                EventPayload::Agent(value) => value,
                EventPayload::Snapshot(Snapshot {
                    value: SnapshotValue::Agent(value),
                    ..
                }) => value,
                _ => continue,
            };
            latest.insert(agent.id.clone(), agent);
        }
        Ok(latest.into_values().collect())
    }

    /// Message revisions without an extraction record, oldest first.
    pub fn pending_extractions(&self, account: &str, limit: usize) -> Result<Vec<Event>> {
        let mut statement = self.connection.prepare(
            "SELECT m.canonical FROM events AS m
             WHERE m.kind = 'message' AND m.account = ?1
               AND NOT EXISTS (SELECT 1 FROM events AS x
                 WHERE x.account = m.account AND x.source = m.source AND x.resource = m.resource
                   AND x.revision = m.revision AND x.kind = 'extraction')
             ORDER BY m.seq LIMIT ?2",
        )?;
        let rows = statement.query_map(params![account, limit as i64], |row| {
            row.get::<_, Vec<u8>>(0)
        })?;
        rows.map(|row| Ok(serde_json::from_slice(&row?)?)).collect()
    }

    /// The user's latest message to `agent` before `before`, for reading a reply in
    /// context.
    pub fn previous_user_message(
        &self,
        account: &str,
        agent: &str,
        before: DateTime<Utc>,
    ) -> Result<Option<Event>> {
        let mut statement = self.connection.prepare(
            "SELECT canonical FROM events WHERE kind = 'message' AND account = ?1
             ORDER BY seq DESC LIMIT 200",
        )?;
        let rows = statement.query_map([account], |row| row.get::<_, Vec<u8>>(0))?;
        let mut latest: Option<Event> = None;
        for row in rows {
            let event: Event = serde_json::from_slice(&row?)?;
            if let EventPayload::Message(message) = &event.payload
                && message.agent == agent
                && message.direction == crate::event::MessageDirection::FromUser
                && event.occurred_at < before
                && latest
                    .as_ref()
                    .is_none_or(|current| event.occurred_at > current.occurred_at)
            {
                latest = Some(event);
            }
        }
        Ok(latest)
    }

    /// An agent's extracted requests and failed extractions since `since`, oldest first.
    pub fn extractions_since(
        &self,
        account: &str,
        agent: &str,
        since: DateTime<Utc>,
    ) -> Result<Vec<Event>> {
        let mut statement = self.connection.prepare(
            "SELECT canonical FROM events WHERE kind IN ('request', 'extraction') AND account = ?1
             ORDER BY seq",
        )?;
        let rows = statement.query_map([account], |row| row.get::<_, Vec<u8>>(0))?;
        let mut events = Vec::new();
        for row in rows {
            let event: Event = serde_json::from_slice(&row?)?;
            let belongs = match &event.payload {
                EventPayload::Request(request) => request.agent == agent,
                EventPayload::Extraction(record) => record.agent == agent,
                _ => false,
            };
            if belongs && event.occurred_at >= since {
                events.push(event);
            }
        }
        Ok(events)
    }

    /// Every retained revision of one finding, oldest first.
    pub fn finding_history(&self, finding: &str) -> Result<Vec<(i64, Finding)>> {
        let mut statement = self.connection.prepare(
            "SELECT revision, payload FROM assessment_revisions WHERE finding=?1 ORDER BY revision",
        )?;
        let rows = statement.query_map([finding], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        rows.map(|row| {
            let (revision, payload) = row?;
            Ok((revision, serde_json::from_slice(&payload)?))
        })
        .collect()
    }

    pub fn current_findings(&self) -> Result<Vec<(i64, Finding)>> {
        let mut statement = self.connection.prepare(
            "SELECT revisions.revision,revisions.payload FROM assessment_current AS current
             JOIN assessment_revisions AS revisions
               ON revisions.finding=current.finding AND revisions.revision=current.revision
             ORDER BY current.finding",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        rows.map(|row| {
            let (revision, payload) = row?;
            Ok((revision, serde_json::from_slice(&payload)?))
        })
        .collect()
    }

    pub fn pending_notifications(&self) -> Result<OutboxBatch> {
        incidents::pending(&self.connection)
    }

    /// A delivery worker acknowledges only after handing a notification to the OS.
    pub fn mark_notification_delivered(&mut self, id: &str) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE notification_outbox SET state='delivered',attempts=attempts+1
             WHERE id=?1 AND state='pending'",
            [id],
        )? == 1)
    }

    pub fn verify(&self) -> Result<Verification> {
        verify_chain(&self.connection)
    }

    pub fn events(&self) -> Result<Vec<StoredEvent>> {
        read_events(&self.connection)
    }

    /// Retains a verified prefix older than the cutoff without crossing a newer row.
    pub fn retain_older_than(&mut self, cutoff: DateTime<Utc>) -> Result<u64> {
        let through = self
            .events()?
            .into_iter()
            .take_while(|row| row.event.ingested_at < cutoff)
            .last()
            .map(|row| row.sequence);
        through.map_or(Ok(0), |sequence| self.retain_after(sequence))
    }

    /// Prunes a verified prefix and records its last hash as the new anchor.
    /// The checkpoint and deletion commit together, so a crash cannot split them.
    pub fn retain_after(&mut self, through_sequence: i64) -> Result<u64> {
        self.verify()?;
        self.assessor = None;
        let snapshots = self.retained_state(through_sequence)?;
        let tx = self.connection.transaction()?;
        let (anchor_seq, _, head_seq, _) = chain_state(&tx)?;
        if through_sequence <= anchor_seq {
            return Ok(0);
        }
        if through_sequence > head_seq {
            return Err(StoreError::Invalid("retention exceeds log head"));
        }
        let (hash, canonical, count): (Vec<u8>, Vec<u8>, i64) = tx.query_row(
            "SELECT hash, canonical, (SELECT COUNT(*) FROM events WHERE seq <= ?1)
             FROM events WHERE seq = ?1",
            [through_sequence],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let hash = checked_hash(through_sequence, hash)?;
        let cutoff = serde_json::from_slice::<Event>(&canonical)?.ingested_at;
        tx.execute("DELETE FROM events WHERE seq <= ?1", [through_sequence])?;
        incidents::prune(&tx, cutoff)?;
        tx.execute(
            "UPDATE chain_state SET anchor_seq=?1, anchor_hash=?2 WHERE id=1",
            params![through_sequence, hash.as_slice()],
        )?;
        let checkpoint = Event {
            schema_version: 1,
            key: EventKey {
                account: "system".into(),
                source: Source::Picket,
                resource: "retention".into(),
                revision: through_sequence.to_string(),
            },
            occurred_at: Utc::now(),
            ingested_at: Utc::now(),
            source_time_quality: None,
            adapter: None,
            payload: EventPayload::Checkpoint {
                through_sequence,
                hash: hash.iter().map(|byte| format!("{byte:02x}")).collect(),
            },
        };
        append_in_transaction(&tx, &checkpoint)?;
        for (index, (account, snapshot)) in snapshots.into_iter().enumerate() {
            let event = Event {
                schema_version: 1,
                key: EventKey {
                    account,
                    source: Source::Picket,
                    resource: format!("retained/{through_sequence}/{index}"),
                    revision: "1".into(),
                },
                occurred_at: Utc::now(),
                ingested_at: Utc::now(),
                source_time_quality: None,
                adapter: None,
                payload: EventPayload::Snapshot(snapshot),
            };
            append_in_transaction(&tx, &event)?;
        }
        tx.commit()?;
        Ok(count as u64)
    }

    fn retained_state(&self, through_sequence: i64) -> Result<Vec<(String, Snapshot)>> {
        let mut latest = BTreeMap::new();
        let mut statement = self.connection.prepare(
            "SELECT seq, canonical FROM events
             WHERE seq <= ?1 AND kind IN ('feedback', 'contact_observed', 'coverage', 'policy', 'agent', 'snapshot') ORDER BY seq",
        )?;
        let rows = statement.query_map([through_sequence], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        for row in rows {
            let (sequence, canonical) = row?;
            let event: Event = serde_json::from_slice(&canonical)?;
            let snapshot = match event.payload {
                EventPayload::Feedback(value) => Snapshot {
                    original_source: event.key.source,
                    original_occurred_at: event.occurred_at,
                    value: SnapshotValue::Feedback(value),
                },
                EventPayload::ContactObserved(value) => Snapshot {
                    original_source: event.key.source,
                    original_occurred_at: event.occurred_at,
                    value: SnapshotValue::Contact(value),
                },
                EventPayload::Coverage(value) => Snapshot {
                    original_source: event.key.source,
                    original_occurred_at: event.occurred_at,
                    value: SnapshotValue::Coverage(value),
                },
                EventPayload::Policy(value) => Snapshot {
                    original_source: event.key.source,
                    original_occurred_at: event.occurred_at,
                    value: SnapshotValue::Policy(value),
                },
                EventPayload::Agent(value) => Snapshot {
                    original_source: event.key.source,
                    original_occurred_at: event.occurred_at,
                    value: SnapshotValue::Agent(value),
                },
                EventPayload::Snapshot(value) => value,
                _ => continue,
            };
            let identity = match &snapshot.value {
                SnapshotValue::Feedback(value) => {
                    format!("feedback/{}/{}", event.key.account, value.finding)
                }
                SnapshotValue::Contact(value) => {
                    format!("contact/{}/{}", event.key.account, value.address)
                }
                SnapshotValue::Coverage(value) => format!(
                    "coverage/{}/{}/{}",
                    event.key.account,
                    snapshot.original_source.as_str(),
                    value.capability
                ),
                SnapshotValue::Policy(value) => {
                    format!("policy/{}/{}", event.key.account, value.id)
                }
                SnapshotValue::Agent(value) => format!("agent/{}", value.id),
            };
            latest.insert(identity, (sequence, event.key.account, snapshot));
        }
        Ok(latest
            .into_values()
            .map(|(_, account, snapshot)| (account, snapshot))
            .collect())
    }
}

fn verify_chain(connection: &Connection) -> Result<Verification> {
    let (anchor_seq, anchor_hash, head_seq, head_hash) = chain_state(connection)?;
    let mut expected_seq = anchor_seq + 1;
    let mut expected_prev = anchor_hash;
    let mut retained = 0;
    let mut statement = connection.prepare(
        "SELECT seq, account, source, resource, revision, kind, canonical, prev_hash, hash
         FROM events ORDER BY seq",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, Vec<u8>>(6)?,
            row.get::<_, Vec<u8>>(7)?,
            row.get::<_, Vec<u8>>(8)?,
        ))
    })?;
    for row in rows {
        let (seq, account, source, resource, revision, kind, canonical, prev, hash) = row?;
        if seq != expected_seq {
            return broken(expected_seq, "missing or reordered row");
        }
        if prev.as_slice() != expected_prev {
            return broken(seq, "previous hash changed");
        }
        let event: Event = serde_json::from_slice(&canonical).map_err(|_| StoreError::Corrupt {
            sequence: seq,
            reason: "invalid event JSON",
        })?;
        if event.key.account != account
            || event.key.source.as_str() != source
            || event.key.resource != resource
            || event.key.revision != revision
            || event.payload.kind() != kind
        {
            return broken(seq, "indexed identity differs from event");
        }
        let value: serde_json::Value = serde_json::from_slice(&canonical)?;
        let normalized = canonical_json(&value)?;
        if normalized != canonical {
            return broken(seq, "event JSON is not canonical");
        }
        let computed = hash_event(&expected_prev, &canonical);
        if hash.as_slice() != computed {
            return broken(seq, "event hash changed");
        }
        expected_prev = computed;
        expected_seq += 1;
        retained += 1;
    }
    if expected_seq != head_seq + 1 || expected_prev != head_hash {
        return broken(expected_seq, "stored log head differs from rows");
    }
    Ok(Verification {
        through_sequence: head_seq,
        retained_events: retained,
    })
}

fn read_events(connection: &Connection) -> Result<Vec<StoredEvent>> {
    verify_chain(connection)?;
    let mut statement =
        connection.prepare("SELECT seq, canonical, prev_hash, hash FROM events ORDER BY seq")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Vec<u8>>(1)?,
            row.get::<_, Vec<u8>>(2)?,
            row.get::<_, Vec<u8>>(3)?,
        ))
    })?;
    rows.map(|row| {
        let (sequence, json, prev, hash) = row?;
        Ok(StoredEvent {
            sequence,
            event: serde_json::from_slice(&json)?,
            prev_hash: checked_hash(sequence, prev)?,
            hash: checked_hash(sequence, hash)?,
        })
    })
    .collect()
}

fn load_assessor(connection: &Connection) -> Result<Assessor> {
    let rows = read_events(connection)?;
    let (through, version, clock): (i64, i64, Option<String>) = connection.query_row(
        "SELECT through_sequence, engine_version, clock FROM engine_state WHERE id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    // A different engine version reassesses everything once, on purpose.
    let clock = clock
        .filter(|_| version == ENGINE_VERSION)
        .map(|value| {
            DateTime::parse_from_rfc3339(&value)
                .map(|time| time.with_timezone(&Utc))
                .map_err(|_| StoreError::Invalid("invalid assessment clock"))
        })
        .transpose()?;
    Ok(Assessor::resume(
        rows.into_iter().map(|row| (row.sequence, row.event)),
        through,
        clock,
    ))
}

/// Builds a coverage event when the report differs from the logged state, or when an
/// open claim needs healthy evidence after its deadline.
fn coverage_transition(
    assessor: &Assessor,
    report: &CoverageReport,
    sequence: i64,
    now: DateTime<Utc>,
) -> Option<Event> {
    let coverage = &report.coverage;
    let latest =
        assessor
            .evidence()
            .coverage_latest(&report.account, &report.source, &coverage.capability);
    let changed = latest.is_none_or(|(logged, _)| {
        logged.state != coverage.state || logged.reason != coverage.reason
    });
    let confirm = coverage.state == CoverageState::Healthy
        && assessor.evidence().confirmation_due(
            &report.account,
            &report.source,
            &coverage.capability,
            now,
        );
    (changed || confirm).then(|| Event {
        schema_version: 1,
        key: EventKey {
            account: report.account.clone(),
            source: report.source.clone(),
            resource: format!("coverage/{}", coverage.capability),
            revision: sequence.to_string(),
        },
        occurred_at: now,
        ingested_at: now,
        source_time_quality: None,
        adapter: None,
        payload: EventPayload::Coverage(coverage.clone()),
    })
}

fn validate_batch(batch: &Batch) -> Result<()> {
    for event in &batch.events {
        validate_event(event)?;
    }
    if let Some(cursor) = &batch.cursor {
        validate_cursor(cursor)?;
        if batch
            .events
            .iter()
            .any(|event| event.key.account != cursor.account || event.key.source != cursor.source)
        {
            return Err(StoreError::Invalid(
                "collector page mixes accounts or sources",
            ));
        }
    }
    if !batch.calendar.is_empty()
        && batch
            .cursor
            .as_ref()
            .is_none_or(|cursor| cursor.source != Source::Calendar)
    {
        return Err(StoreError::Invalid("snapshots require a Calendar cursor"));
    }
    for (resource, _) in &batch.calendar {
        if resource.is_empty() || resource.len() > 512 || resource.chars().any(char::is_control) {
            return Err(StoreError::Invalid("invalid Calendar snapshot resource"));
        }
    }
    for report in &batch.coverage {
        let capability = &report.coverage.capability;
        if report.source == Source::Picket
            || capability.is_empty()
            || capability.len() > 64
            || !capability.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
            })
        {
            return Err(StoreError::Invalid("invalid coverage report"));
        }
    }
    Ok(())
}

fn validate_event(event: &Event) -> Result<()> {
    if event.schema_version != 1 {
        return Err(StoreError::Invalid("unsupported event schema version"));
    }
    for field in [&event.key.account, &event.key.resource, &event.key.revision] {
        if field.is_empty() || field.len() > 512 || field.chars().any(char::is_control) {
            return Err(StoreError::Invalid(
                "source identity must be bounded and nonempty",
            ));
        }
    }
    if let Some(adapter) = &event.adapter
        && (adapter.version == 0
            || adapter.name.is_empty()
            || adapter.scope.is_empty()
            || adapter.name.len() > 128
            || adapter.scope.len() > 256
            || adapter.name.chars().any(char::is_control)
            || adapter.scope.chars().any(char::is_control))
    {
        return Err(StoreError::Invalid("invalid adapter provenance"));
    }
    if matches!(
        event.payload,
        EventPayload::Checkpoint { .. } | EventPayload::Agent(_)
    ) && event.key.source != Source::Picket
    {
        return Err(StoreError::Invalid("only Picket may write this record"));
    }
    match &event.payload {
        EventPayload::Agent(agent) => crate::agent::validate(agent)
            .map_err(|_| StoreError::Invalid("invalid agent registration"))?,
        EventPayload::Message(message)
            if event.key.source != Source::Messages
                || !crate::agent::valid_id(&message.agent)
                || message.service.is_empty()
                || message.service.len() > 32
                || message.snippet.chars().count() > crate::agent::SNIPPET_CHARS
                || message.snippet.chars().any(char::is_control) =>
        {
            return Err(StoreError::Invalid("invalid message observation"));
        }
        _ => {}
    }
    Ok(())
}

fn validate_cursor(cursor: &CollectorCursor) -> Result<()> {
    if cursor.account.is_empty()
        || cursor.account.len() > 512
        || cursor.value.is_empty()
        || cursor.value.len() > 4096
        || cursor.account.chars().any(char::is_control)
        || cursor.value.chars().any(char::is_control)
        || !matches!(
            cursor.source,
            Source::Gmail | Source::Calendar | Source::Messages
        )
    {
        return Err(StoreError::Invalid("invalid collector cursor"));
    }
    Ok(())
}

fn append_in_transaction(tx: &Transaction<'_>, event: &Event) -> Result<i64> {
    let canonical = canonical_json(event)?;
    if canonical.len() > MAX_EVENT_BYTES {
        return Err(StoreError::Invalid("event exceeds 64 KiB"));
    }
    let existing: Option<(i64, Vec<u8>)> = tx
        .query_row(
            "SELECT seq, canonical FROM events
         WHERE account=?1 AND source=?2 AND resource=?3 AND revision=?4 AND kind=?5",
            params![
                event.key.account,
                event.key.source.as_str(),
                event.key.resource,
                event.key.revision,
                event.payload.kind()
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((seq, existing)) = existing {
        let mut retry = event.clone();
        let prior: Event = serde_json::from_slice(&existing)?;
        retry.ingested_at = prior.ingested_at;
        if retry == prior {
            return Ok(seq);
        }
        return Err(StoreError::DuplicateConflict);
    }
    let (_, _, head_seq, head_hash) = chain_state(tx)?;
    let sequence = head_seq + 1;
    let hash = hash_event(&head_hash, &canonical);
    tx.execute(
        "INSERT INTO events(seq, account, source, resource, revision, kind, canonical, prev_hash, hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![sequence, event.key.account, event.key.source.as_str(), event.key.resource,
                event.key.revision, event.payload.kind(), canonical, head_hash.as_slice(), hash.as_slice()],
    )?;
    tx.execute(
        "UPDATE chain_state SET head_seq=?1, head_hash=?2 WHERE id=1",
        params![sequence, hash.as_slice()],
    )?;
    Ok(sequence)
}

fn chain_state(connection: &Connection) -> Result<(i64, [u8; 32], i64, [u8; 32])> {
    let (anchor_seq, anchor_hash, head_seq, head_hash): (i64, Vec<u8>, i64, Vec<u8>) = connection
        .query_row(
        "SELECT anchor_seq, anchor_hash, head_seq, head_hash FROM chain_state WHERE id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    if anchor_seq < 0 || head_seq < anchor_seq {
        return broken(anchor_seq + 1, "invalid chain bounds");
    }
    Ok((
        anchor_seq,
        checked_hash(anchor_seq + 1, anchor_hash)?,
        head_seq,
        checked_hash(head_seq, head_hash)?,
    ))
}

fn checked_hash(sequence: i64, value: Vec<u8>) -> Result<[u8; 32]> {
    value.try_into().map_err(|_| StoreError::Corrupt {
        sequence,
        reason: "hash has wrong length",
    })
}

fn hash_event(previous: &[u8; 32], canonical: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(previous);
    hasher.update(canonical);
    hasher.finalize().into()
}

fn broken<T>(sequence: i64, reason: &'static str) -> Result<T> {
    Err(StoreError::Corrupt { sequence, reason })
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use serde_json::json;

    use super::*;
    use crate::calendar::CalendarEvent;
    use crate::event::{
        AccountPolicy, Action, ContactObserved, Coverage, CoverageState, Effect, Feedback, Origin,
    };

    fn page(
        events: Vec<Event>,
        cursor: &CollectorCursor,
        calendar: Vec<(String, CalendarState)>,
    ) -> Batch {
        Batch {
            events,
            cursor: Some(cursor.clone()),
            calendar,
            coverage: Vec::new(),
        }
    }

    fn event(resource: &str, revision: &str) -> Event {
        let timestamp = DateTime::<Utc>::UNIX_EPOCH;
        Event {
            schema_version: 1,
            key: EventKey {
                account: "synthetic-account".into(),
                source: Source::Gmail,
                resource: resource.into(),
                revision: revision.into(),
            },
            occurred_at: timestamp,
            ingested_at: timestamp,
            source_time_quality: None,
            adapter: None,
            payload: EventPayload::Effect(Effect {
                target_names: Default::default(),
                action: Action::EmailSend,
                targets: vec!["person@example.test".into()],
                targets_complete: true,
                calendar_start: None,
                origin: Origin::Live,
                external_change: false,
                email: None,
                calendar: None,
            }),
        }
    }

    #[test]
    fn canonical_json_sorts_nested_keys() -> Result<()> {
        let value = json!({"z": {"b": 2, "a": 1}, "a": [2, 1]});
        assert_eq!(canonical_json(&value)?, br#"{"a":[2,1],"z":{"a":1,"b":2}}"#);
        Ok(())
    }

    #[test]
    fn keeps_revisions_and_deduplicates_retries() -> Result<()> {
        let mut store = Store::memory()?;
        let first = event("message-1", "rev-1");
        assert_eq!(store.append(&first)?, 1);
        let mut retry = first.clone();
        retry.ingested_at += chrono::Duration::seconds(1);
        assert_eq!(store.append(&retry)?, 1);
        assert_eq!(store.append(&event("message-1", "rev-2"))?, 2);
        assert_eq!(store.verify()?.retained_events, 2);
        let mut conflict = first;
        if let EventPayload::Effect(effect) = &mut conflict.payload {
            effect.targets = vec!["other@example.test".into()];
        }
        assert!(matches!(
            store.append(&conflict),
            Err(StoreError::DuplicateConflict)
        ));
        Ok(())
    }

    #[test]
    fn collector_page_and_cursor_commit_or_roll_back_together() -> Result<()> {
        let mut store = Store::memory()?;
        let cursor = CollectorCursor {
            account: "synthetic-account".into(),
            source: Source::Gmail,
            value: "history-10".into(),
            updated_at: DateTime::<Utc>::UNIX_EPOCH,
        };
        store.commit(
            page(vec![event("message-1", "rev-1")], &cursor, vec![]),
            cursor.updated_at,
        )?;
        assert_eq!(
            store.collector_cursor(&cursor.account, &cursor.source)?,
            Some(cursor.value.clone())
        );
        let mut conflict = event("message-1", "rev-1");
        if let EventPayload::Effect(effect) = &mut conflict.payload {
            effect.targets = vec!["different@example.test".into()];
        }
        let later = CollectorCursor {
            value: "history-20".into(),
            ..cursor.clone()
        };
        assert!(matches!(
            store.commit(
                page(vec![event("message-2", "rev-1"), conflict], &later, vec![]),
                later.updated_at
            ),
            Err(StoreError::DuplicateConflict)
        ));
        assert_eq!(store.verify()?.retained_events, 1);
        assert_eq!(
            store.collector_cursor(&cursor.account, &cursor.source)?,
            Some(cursor.value)
        );
        Ok(())
    }

    #[test]
    fn calendar_snapshot_and_cursor_commit_together()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let mut store = Store::memory()?;
        let wire: CalendarEvent = serde_json::from_str(
            r#"{"id":"meeting_1","etag":"rev1","start":{"dateTime":"2026-10-23T09:00:00Z"},"attendees":[{"email":"person@example.test"}]}"#,
        )?;
        let (event, state) =
            wire.observation("account-1", crate::event::Origin::Live, Utc::now(), None)?;
        let cursor = CollectorCursor {
            account: "account-1".into(),
            source: Source::Calendar,
            value: "sync-1".into(),
            updated_at: Utc::now(),
        };
        store.commit(
            page(
                vec![event.ok_or("missing event")?],
                &cursor,
                vec![("meeting_1".into(), state.clone())],
            ),
            cursor.updated_at,
        )?;
        assert_eq!(
            store.calendar_snapshot("account-1", "meeting_1")?,
            Some(state.clone())
        );
        assert_eq!(
            store.collector_cursor("account-1", &Source::Calendar)?,
            Some("sync-1".into())
        );
        let later = CollectorCursor {
            value: "sync-2".into(),
            ..cursor
        };
        assert!(
            store
                .commit(
                    page(
                        vec![],
                        &later,
                        vec![("bad\nresource".into(), state.clone())]
                    ),
                    later.updated_at,
                )
                .is_err()
        );
        assert_eq!(
            store.collector_cursor("account-1", &Source::Calendar)?,
            Some("sync-1".into())
        );
        assert_eq!(
            store.calendar_snapshot("account-1", "meeting_1")?,
            Some(state)
        );
        Ok(())
    }

    #[test]
    fn first_modified_or_deleted_row_is_reported() -> Result<()> {
        let mut store = Store::memory()?;
        for n in 1..=3 {
            store.append(&event(&format!("message-{n}"), "rev-1"))?;
        }
        store
            .connection
            .execute("UPDATE events SET canonical = x'7b7d' WHERE seq=2", [])?;
        assert!(matches!(
            store.verify(),
            Err(StoreError::Corrupt { sequence: 2, .. })
        ));
        store
            .connection
            .execute("DELETE FROM events WHERE seq=1", [])?;
        assert!(matches!(
            store.verify(),
            Err(StoreError::Corrupt { sequence: 1, .. })
        ));
        Ok(())
    }

    #[test]
    fn detects_removed_tail_and_index_tampering() -> Result<()> {
        let mut store = Store::memory()?;
        store.append(&event("message-1", "rev-1"))?;
        store.append(&event("message-2", "rev-1"))?;
        store
            .connection
            .execute("UPDATE events SET account='other' WHERE seq=1", [])?;
        assert!(matches!(
            store.verify(),
            Err(StoreError::Corrupt { sequence: 1, .. })
        ));
        store.connection.execute(
            "UPDATE events SET account='synthetic-account' WHERE seq=1",
            [],
        )?;
        store
            .connection
            .execute("DELETE FROM events WHERE seq=2", [])?;
        assert!(matches!(
            store.verify(),
            Err(StoreError::Corrupt { sequence: 2, .. })
        ));
        Ok(())
    }

    #[test]
    fn retention_preserves_anchor_and_checkpoint() -> Result<()> {
        let mut store = Store::memory()?;
        for n in 1..=3 {
            store.append(&event(&format!("message-{n}"), "rev-1"))?;
        }
        assert_eq!(store.retain_after(2)?, 2);
        let events = store.events()?;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].sequence, 3);
        assert_eq!(events[1].sequence, 4);
        assert!(matches!(
            events[1].event.payload,
            EventPayload::Checkpoint {
                through_sequence: 2,
                ..
            }
        ));
        assert_eq!(store.verify()?.through_sequence, 4);
        store
            .connection
            .execute("DELETE FROM events WHERE seq=3", [])?;
        assert!(matches!(
            store.verify(),
            Err(StoreError::Corrupt { sequence: 3, .. })
        ));
        Ok(())
    }

    #[test]
    fn time_retention_keeps_newer_rows_and_verified_chain() -> Result<()> {
        let mut store = Store::memory()?;
        store.append(&event("old", "rev-1"))?;
        let mut recent = event("recent", "rev-1");
        recent.ingested_at = Utc::now();
        store.append(&recent)?;
        let cutoff = Utc::now() - chrono::Duration::days(180);
        assert_eq!(store.retain_older_than(cutoff)?, 1);
        assert_eq!(store.retain_older_than(cutoff)?, 0);
        assert!(
            store
                .events()?
                .iter()
                .any(|row| row.event.key.resource == "recent")
        );
        store.verify()?;
        Ok(())
    }

    #[test]
    fn coverage_is_a_durable_event() -> Result<()> {
        let mut store = Store::memory()?;
        let mut gap = event("gmail-coverage", "gap-1");
        gap.payload = EventPayload::Coverage(Coverage {
            capability: "sent-records".into(),
            state: CoverageState::Gap,
            cursor: None,
            gap_since: None,
            reason: None,
            partial_pages: None,
        });
        store.append(&gap)?;
        assert_eq!(
            store.events()?.first().map(|row| &row.event.payload),
            Some(&gap.payload)
        );
        Ok(())
    }

    #[test]
    fn retention_preserves_latest_feedback_contact_and_coverage() -> Result<()> {
        let mut store = Store::memory()?;
        let mut feedback = event("finding-1", "decision-1");
        feedback.key.source = Source::Picket;
        feedback.payload = EventPayload::Feedback(Feedback {
            finding: "finding-1".into(),
            expected: Some(false),
            actor: None,
        });
        store.append(&feedback)?;
        let mut contact = event("contact-1", "seen-1");
        contact.payload = EventPayload::ContactObserved(ContactObserved {
            address: "person@example.test".into(),
            origin: Origin::Baseline,
        });
        store.append(&contact)?;
        let mut coverage = event("coverage-1", "gap-1");
        coverage.payload = EventPayload::Coverage(Coverage {
            capability: "sent-records".into(),
            state: CoverageState::Gap,
            cursor: None,
            gap_since: None,
            reason: None,
            partial_pages: None,
        });
        store.append(&coverage)?;
        assert_eq!(store.retain_after(3)?, 3);
        let remaining = store.events()?;
        assert_eq!(remaining.len(), 4);
        assert!(
            remaining
                .iter()
                .all(|row| row.event.key.source == Source::Picket)
        );
        assert!(remaining.iter().any(|row| matches!(&row.event.payload,
            EventPayload::Snapshot(Snapshot { value: SnapshotValue::Feedback(value), .. })
                if Some(value) == match &feedback.payload { EventPayload::Feedback(value) => Some(value), _ => None })));
        assert!(remaining.iter().any(|row| matches!(&row.event.payload,
            EventPayload::Snapshot(Snapshot { value: SnapshotValue::Contact(value), .. })
                if Some(value) == match &contact.payload { EventPayload::ContactObserved(value) => Some(value), _ => None })));
        assert!(remaining.iter().any(|row| matches!(&row.event.payload,
            EventPayload::Snapshot(Snapshot { original_source: Source::Gmail,
                value: SnapshotValue::Coverage(value), .. })
                if Some(value) == match &coverage.payload { EventPayload::Coverage(value) => Some(value), _ => None })));
        assert_eq!(store.retain_after(6)?, 3);
        assert_eq!(store.events()?.len(), 4);
        Ok(())
    }

    #[test]
    fn agent_registration_survives_retention_and_only_picket_writes_it() -> Result<()> {
        let mut store = Store::memory()?;
        let mut registration = event("agent/instinct", "1");
        registration.key.source = Source::Picket;
        registration.payload = EventPayload::Agent(crate::event::AgentRegistration {
            id: "instinct".into(),
            name: "Instinct".into(),
            mode: crate::event::AgentMode::Messages,
            handles: vec!["+14155550132".into()],
            connected_since: None,
            enabled: true,
            region: None,
        });
        let mut forged = registration.clone();
        forged.key.source = Source::Messages;
        assert!(store.append(&forged).is_err());
        store.append(&registration)?;
        store.append(&event("message-1", "rev-1"))?;
        store.retain_after(2)?;
        assert!(
            store
                .events()?
                .iter()
                .any(|row| matches!(&row.event.payload,
            EventPayload::Snapshot(Snapshot { value: SnapshotValue::Agent(value), .. })
                if value.id == "instinct"))
        );
        Ok(())
    }

    #[test]
    fn retention_preserves_policy_as_of_cutoff_even_with_later_revision() -> Result<()> {
        let mut store = Store::memory()?;
        let mut first = event("policy-1", "1");
        first.key.source = Source::Picket;
        first.payload = EventPayload::Policy(AccountPolicy {
            id: "no-send".into(),
            forbidden_actions: vec![Action::EmailSend],
            forbidden_targets: vec![],
            high_consequence: true,
            enabled: true,
        });
        store.append(&first)?;
        let mut later = first.clone();
        later.key.revision = "2".into();
        later.occurred_at += chrono::Duration::minutes(1);
        later.ingested_at = later.occurred_at;
        if let EventPayload::Policy(value) = &mut later.payload {
            value.enabled = false;
        }
        store.append(&later)?;
        store.retain_after(1)?;
        let retained = store.events()?;
        assert!(retained.iter().any(|row| matches!(&row.event.payload,
            EventPayload::Snapshot(Snapshot { value: SnapshotValue::Policy(value), .. })
                if value.id == "no-send" && value.enabled)));
        assert!(retained.iter().any(|row| matches!(&row.event.payload,
            EventPayload::Policy(value) if !value.enabled)));
        Ok(())
    }

    #[test]
    fn old_canonical_event_without_new_optional_field_remains_verifiable() -> Result<()> {
        let mut store = Store::memory()?;
        let current = event("old-message", "rev-1");
        let mut old_value = serde_json::to_value(&current)?;
        if let Some(fields) = old_value
            .pointer_mut("/payload/data")
            .and_then(|value| value.as_object_mut())
        {
            fields.remove("calendar_start");
        }
        let old_bytes = canonical_json(&old_value)?;
        let hash = hash_event(&[0; 32], &old_bytes);
        store.connection.execute(
            "INSERT INTO events(seq,account,source,resource,revision,kind,canonical,prev_hash,hash)
             VALUES (1,?1,?2,?3,?4,'effect',?5,?6,?7)",
            params![
                current.key.account,
                current.key.source.as_str(),
                current.key.resource,
                current.key.revision,
                old_bytes,
                [0u8; 32].as_slice(),
                hash.as_slice()
            ],
        )?;
        store.connection.execute(
            "UPDATE chain_state SET head_seq=1,head_hash=?1 WHERE id=1",
            [hash.as_slice()],
        )?;
        assert_eq!(store.verify()?.retained_events, 1);
        assert_eq!(store.events()?.len(), 1);
        assert_eq!(store.append(&current)?, 1);
        Ok(())
    }
}
