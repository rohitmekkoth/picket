//! The shared write path. One thread owns the mutable store; every collector holds a
//! `Writer` with its own read-only connection and submits batches that commit in order.

use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};

use crate::{
    calendar::CalendarState,
    event::{AgentRegistration, Event, EventKey, Source},
    store::{Batch, CollectorCursor, Committed, Result, Store, StoreError},
};

/// Deadlines pass without new input, so the writer reassesses on this interval.
const TICK: Duration = Duration::from_secs(5);
const RETENTION_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const RETENTION_DAYS: i64 = 180;
/// Pending commits per writer; a full queue makes producers wait, not drop.
const QUEUE: usize = 64;

/// Whether a poll reached the source's current state. A source that is behind
/// continues from its committed position on the next poll.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Progress {
    CaughtUp,
    Behind,
}

/// What a collector needs from the store: committed reads and atomic batch writes.
pub trait Ledger {
    fn checkpoint(&self, account: &str, source: &Source) -> Result<Option<CollectorCursor>>;
    fn contains(&self, key: &EventKey, kind: &str) -> Result<bool>;
    fn calendar_snapshot(&self, account: &str, resource: &str) -> Result<Option<CalendarState>>;
    fn commit(&mut self, batch: Batch, now: DateTime<Utc>) -> Result<Committed>;
    fn agents(&self) -> Result<Vec<AgentRegistration>>;
    fn pending_extractions(&self, account: &str, limit: usize) -> Result<Vec<Event>>;
    fn extractions_since(
        &self,
        account: &str,
        agent: &str,
        since: DateTime<Utc>,
    ) -> Result<Vec<Event>>;
    fn previous_user_message(
        &self,
        account: &str,
        agent: &str,
        before: DateTime<Utc>,
    ) -> Result<Option<Event>>;
}

impl Ledger for Store {
    fn checkpoint(&self, account: &str, source: &Source) -> Result<Option<CollectorCursor>> {
        self.collector_checkpoint(account, source)
    }

    fn contains(&self, key: &EventKey, kind: &str) -> Result<bool> {
        self.event_exists(key, kind)
    }

    fn calendar_snapshot(&self, account: &str, resource: &str) -> Result<Option<CalendarState>> {
        Store::calendar_snapshot(self, account, resource)
    }

    fn commit(&mut self, batch: Batch, now: DateTime<Utc>) -> Result<Committed> {
        Store::commit(self, batch, now)
    }

    fn agents(&self) -> Result<Vec<AgentRegistration>> {
        Store::agents(self)
    }

    fn pending_extractions(&self, account: &str, limit: usize) -> Result<Vec<Event>> {
        Store::pending_extractions(self, account, limit)
    }

    fn extractions_since(
        &self,
        account: &str,
        agent: &str,
        since: DateTime<Utc>,
    ) -> Result<Vec<Event>> {
        Store::extractions_since(self, account, agent, since)
    }

    fn previous_user_message(
        &self,
        account: &str,
        agent: &str,
        before: DateTime<Utc>,
    ) -> Result<Option<Event>> {
        Store::previous_user_message(self, account, agent, before)
    }
}

struct Request {
    batch: Batch,
    now: DateTime<Utc>,
    reply: SyncSender<Result<Committed>>,
}

/// A collector's handle. Reads see every batch this handle committed, because a
/// commit returns only after the writer's transaction is durable.
pub struct Writer {
    path: PathBuf,
    reader: Store,
    sender: SyncSender<Request>,
}

impl Writer {
    /// Opens or migrates the store and starts its writer thread. The thread runs an
    /// assessment tick and daily retention, and stops when every handle is dropped.
    pub fn start(path: &Path) -> Result<(Self, JoinHandle<()>)> {
        let mut store = Store::open(path)?;
        let now = Utc::now();
        store.reconcile(now)?;
        store.retain_older_than(now - chrono::Duration::days(RETENTION_DAYS))?;
        let (sender, receiver) = mpsc::sync_channel(QUEUE);
        let thread = thread::Builder::new()
            .name("picket-writer".into())
            .spawn(move || run(store, receiver))
            .map_err(|_| StoreError::Unavailable)?;
        Ok((
            Self {
                path: path.to_owned(),
                reader: Store::open_read_only(path)?,
                sender,
            },
            thread,
        ))
    }

    /// Another handle with its own read connection, for a different collector thread.
    pub fn connect(&self) -> Result<Self> {
        Ok(Self {
            path: self.path.clone(),
            reader: Store::open_read_only(&self.path)?,
            sender: self.sender.clone(),
        })
    }
}

impl Ledger for Writer {
    fn checkpoint(&self, account: &str, source: &Source) -> Result<Option<CollectorCursor>> {
        self.reader.collector_checkpoint(account, source)
    }

    fn contains(&self, key: &EventKey, kind: &str) -> Result<bool> {
        self.reader.event_exists(key, kind)
    }

    fn calendar_snapshot(&self, account: &str, resource: &str) -> Result<Option<CalendarState>> {
        self.reader.calendar_snapshot(account, resource)
    }

    fn commit(&mut self, batch: Batch, now: DateTime<Utc>) -> Result<Committed> {
        let (reply, response) = mpsc::sync_channel(1);
        self.sender
            .send(Request { batch, now, reply })
            .map_err(|_| StoreError::Unavailable)?;
        response.recv().map_err(|_| StoreError::Unavailable)?
    }

    fn agents(&self) -> Result<Vec<AgentRegistration>> {
        self.reader.agents()
    }

    fn pending_extractions(&self, account: &str, limit: usize) -> Result<Vec<Event>> {
        self.reader.pending_extractions(account, limit)
    }

    fn extractions_since(
        &self,
        account: &str,
        agent: &str,
        since: DateTime<Utc>,
    ) -> Result<Vec<Event>> {
        self.reader.extractions_since(account, agent, since)
    }

    fn previous_user_message(
        &self,
        account: &str,
        agent: &str,
        before: DateTime<Utc>,
    ) -> Result<Option<Event>> {
        self.reader.previous_user_message(account, agent, before)
    }
}

fn run(mut store: Store, receiver: Receiver<Request>) {
    let mut next_retention = Instant::now() + RETENTION_INTERVAL;
    loop {
        match receiver.recv_timeout(TICK) {
            Ok(request) => {
                let result = store.commit(request.batch, request.now);
                // A producer that stopped waiting has nothing left to do with the result.
                let _ = request.reply.send(result);
            }
            Err(RecvTimeoutError::Timeout) => {
                if store.reconcile(Utc::now()).is_err() {
                    tracing::warn!("assessment tick failed");
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if Instant::now() >= next_retention {
            let cutoff = Utc::now() - chrono::Duration::days(RETENTION_DAYS);
            if store.retain_older_than(cutoff).is_err() {
                tracing::warn!("event retention failed");
            }
            next_retention = Instant::now() + RETENTION_INTERVAL;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Coverage, CoverageState};
    use crate::store::CoverageReport;

    fn healthy() -> Batch {
        Batch {
            coverage: vec![CoverageReport {
                account: "synthetic-account".into(),
                source: Source::Gmail,
                coverage: Coverage {
                    capability: "effects".into(),
                    state: CoverageState::Healthy,
                    cursor: None,
                    gap_since: None,
                    reason: None,
                    partial_pages: None,
                },
            }],
            ..Batch::default()
        }
    }

    #[test]
    fn handles_on_separate_threads_share_one_ordered_log()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("picket.db");
        let (writer, thread) = Writer::start(&path)?;
        let workers: Vec<_> = (0..4)
            .map(|index| {
                let mut handle = writer.connect()?;
                Ok(thread::spawn(move || {
                    let cursor = CollectorCursor {
                        account: format!("account-{index}"),
                        source: Source::Calendar,
                        value: "sync".into(),
                        updated_at: Utc::now(),
                    };
                    let batch = Batch {
                        cursor: Some(cursor.clone()),
                        ..Batch::default()
                    };
                    handle.commit(batch, Utc::now())?;
                    // A handle reads its own committed write.
                    handle.checkpoint(&cursor.account, &cursor.source)
                }))
            })
            .collect::<Result<_>>()?;
        for worker in workers {
            assert!(worker.join().map_err(|_| "worker panicked")??.is_some());
        }
        let mut writer = writer;
        writer.commit(healthy(), Utc::now())?;
        writer.commit(healthy(), Utc::now())?;
        drop(writer);
        thread.join().map_err(|_| "writer panicked")?;
        let store = Store::open_read_only(&path)?;
        assert_eq!(store.verify()?.retained_events, 1);
        Ok(())
    }
}
