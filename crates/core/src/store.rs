//! One mutable SQLite connection owns ordered log writes and retention.

use std::{collections::BTreeMap, path::Path};

use chrono::Utc;
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::event::{
    Event, EventKey, EventPayload, Snapshot, SnapshotValue, Source, canonical_json,
};

const SCHEMA_VERSION: i64 = 1;
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
pub struct Store {
    connection: Connection,
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
        if version != SCHEMA_VERSION {
            return Err(StoreError::Schema(version));
        }
        Ok(Self { connection })
    }

    fn initialize(connection: Connection) -> Result<Self> {
        connection.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON;",
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
            SCHEMA_VERSION => {}
            other => return Err(StoreError::Schema(other)),
        }
        Ok(Self { connection })
    }

    pub fn append(&mut self, event: &Event) -> Result<i64> {
        validate_event(event)?;
        let tx = self.connection.transaction()?;
        let sequence = append_in_transaction(&tx, event)?;
        tx.commit()?;
        Ok(sequence)
    }

    pub fn verify(&self) -> Result<Verification> {
        let (anchor_seq, anchor_hash, head_seq, head_hash) = chain_state(&self.connection)?;
        let mut expected_seq = anchor_seq + 1;
        let mut expected_prev = anchor_hash;
        let mut retained = 0;
        let mut statement = self.connection.prepare(
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
            let event: Event =
                serde_json::from_slice(&canonical).map_err(|_| StoreError::Corrupt {
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
            let normalized = canonical_json(&event)?;
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

    pub fn events(&self) -> Result<Vec<StoredEvent>> {
        self.verify()?;
        let mut statement = self
            .connection
            .prepare("SELECT seq, canonical, prev_hash, hash FROM events ORDER BY seq")?;
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

    /// Prunes a verified prefix and records its last hash as the new anchor.
    /// The checkpoint and deletion commit together, so a crash cannot split them.
    pub fn retain_after(&mut self, through_sequence: i64) -> Result<u64> {
        self.verify()?;
        let snapshots = self.retained_state(through_sequence)?;
        let tx = self.connection.transaction()?;
        let (anchor_seq, _, head_seq, _) = chain_state(&tx)?;
        if through_sequence <= anchor_seq {
            return Ok(0);
        }
        if through_sequence > head_seq {
            return Err(StoreError::Invalid("retention exceeds log head"));
        }
        let (hash, count): (Vec<u8>, i64) = tx.query_row(
            "SELECT hash, (SELECT COUNT(*) FROM events WHERE seq <= ?1)
             FROM events WHERE seq = ?1",
            [through_sequence],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let hash = checked_hash(through_sequence, hash)?;
        tx.execute("DELETE FROM events WHERE seq <= ?1", [through_sequence])?;
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
             WHERE seq <= ?1 AND kind IN ('feedback', 'contact_observed', 'coverage', 'policy', 'snapshot') ORDER BY seq",
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
            };
            latest.insert(identity, (sequence, event.key.account, snapshot));
        }
        Ok(latest
            .into_values()
            .map(|(_, account, snapshot)| (account, snapshot))
            .collect())
    }
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
    if matches!(event.payload, EventPayload::Checkpoint { .. })
        && event.key.source != Source::Picket
    {
        return Err(StoreError::Invalid("only Picket may write a checkpoint"));
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
        if canonical_json(&retry)? == existing {
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
    use crate::event::{
        AccountPolicy, Action, ContactObserved, Coverage, CoverageState, Effect, Feedback, Origin,
    };

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
            payload: EventPayload::Effect(Effect {
                action: Action::EmailSend,
                targets: vec!["person@example.test".into()],
                targets_complete: true,
                calendar_start: None,
                origin: Origin::Live,
                external_change: false,
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
    fn coverage_is_a_durable_event() -> Result<()> {
        let mut store = Store::memory()?;
        let mut gap = event("gmail-coverage", "gap-1");
        gap.payload = EventPayload::Coverage(Coverage {
            capability: "sent-records".into(),
            state: CoverageState::Gap,
            cursor: None,
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
}
