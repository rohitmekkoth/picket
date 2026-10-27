//! Durable assessment revisions and notification intent. No OS delivery here.

use std::ops::Deref;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::{
    engine::{Finding, FindingDetail, Route},
    event::canonical_json,
    store::Result,
};

pub(crate) const ENGINE_VERSION: i64 = 2;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OutboxKind {
    Alert,
    Correction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutboxItem {
    pub id: String,
    pub finding: String,
    pub revision: i64,
    pub kind: OutboxKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutboxBatch {
    pub items: Vec<OutboxItem>,
    pub has_more: bool,
}

impl Deref for OutboxBatch {
    type Target = [OutboxItem];

    fn deref(&self) -> &Self::Target {
        &self.items
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReconcileSummary {
    pub through_sequence: i64,
    /// Findings evaluated in this write; bounded by what the new input can affect.
    pub assessed: usize,
    pub revised: usize,
    pub queued: usize,
}

/// Stores changed findings and their notification intent with the engine position.
pub(crate) fn persist(
    tx: &Transaction<'_>,
    findings: Vec<Finding>,
    now: DateTime<Utc>,
    through_sequence: i64,
) -> Result<ReconcileSummary> {
    let assessed = findings.len();
    let mut revised = 0;
    let mut queued = 0;
    for finding in findings {
        let previous: Option<(i64, Vec<u8>)> = tx
            .query_row(
                "SELECT revisions.revision, revisions.payload FROM assessment_current AS current
             JOIN assessment_revisions AS revisions
               ON revisions.finding=current.finding AND revisions.revision=current.revision
             WHERE current.finding=?1",
                [&finding.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let payload = canonical_json(&finding)?;
        if previous
            .as_ref()
            .is_some_and(|(_, prior)| *prior == payload)
        {
            continue;
        }
        let revision = previous.as_ref().map_or(1, |(number, _)| number + 1);
        let old = previous
            .as_ref()
            .map(|(_, value)| serde_json::from_slice::<Finding>(value))
            .transpose()?;
        tx.execute(
            "INSERT INTO assessment_revisions(finding,revision,assessed_at,engine_version,payload)
             VALUES (?1,?2,?3,?4,?5)",
            params![
                finding.id,
                revision,
                now.to_rfc3339(),
                ENGINE_VERSION,
                payload
            ],
        )?;
        tx.execute(
            "INSERT INTO assessment_current(finding,revision) VALUES (?1,?2)
             ON CONFLICT(finding) DO UPDATE SET revision=excluded.revision",
            params![finding.id, revision],
        )?;
        revised += 1;
        let pending_before: i64 = tx.query_row(
            "SELECT COUNT(*) FROM notification_outbox WHERE finding=?1 AND state='pending'",
            [&finding.id],
            |row| row.get(0),
        )?;
        let material = old
            .as_ref()
            .is_some_and(|prior| material_change(prior, &finding));
        if pending_before > 0
            && !material
            && old
                .as_ref()
                .is_some_and(|prior| ready_for_alert(prior) == ready_for_alert(&finding))
        {
            continue;
        }
        tx.execute(
            "UPDATE notification_outbox SET state='cancelled' WHERE finding=?1 AND state='pending'",
            [&finding.id],
        )?;
        let delivered: i64 = tx.query_row(
            "SELECT COUNT(*) FROM notification_outbox WHERE finding=?1 AND state='delivered'",
            [&finding.id],
            |row| row.get(0),
        )?;
        let kind = if delivered > 0 {
            material.then_some(OutboxKind::Correction)
        } else if ready_for_alert(&finding) {
            Some(OutboxKind::Alert)
        } else {
            None
        };
        if let Some(kind) = kind {
            let id = format!("{}/{}", finding.id, revision);
            tx.execute(
                "INSERT INTO notification_outbox(id,finding,revision,kind,state,created_at)
                 VALUES (?1,?2,?3,?4,'pending',?5)",
                params![id, finding.id, revision, kind.as_str(), now.to_rfc3339()],
            )?;
            queued += 1;
        }
    }
    tx.execute(
        "UPDATE engine_state SET through_sequence=?1,engine_version=?2,clock=?3 WHERE id=1",
        params![through_sequence, ENGINE_VERSION, now.to_rfc3339()],
    )?;
    Ok(ReconcileSummary {
        through_sequence,
        assessed,
        revised,
        queued,
    })
}

/// Applies log retention to derived data. Findings about pruned events go with
/// them; superseded revisions older than the cutoff go unless a pending notification
/// still needs them. Beyond the horizon Picket keeps no history, including deliveries.
pub(crate) fn prune(tx: &Transaction<'_>, cutoff: DateTime<Utc>) -> Result<usize> {
    let mut orphaned = Vec::new();
    {
        let mut statement = tx.prepare(
            "SELECT revisions.payload FROM assessment_current AS current
             JOIN assessment_revisions AS revisions
               ON revisions.finding=current.finding AND revisions.revision=current.revision",
        )?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let finding: Finding = serde_json::from_slice(&row.get::<_, Vec<u8>>(0)?)?;
            let subject = match &finding.detail {
                FindingDetail::Effect(value) => Some((&value.effect, "effect")),
                FindingDetail::Claim(value) => Some((&value.claim, "claim")),
                FindingDetail::Notice(value) => Some((&value.notice, "notice")),
                FindingDetail::Coverage(_) => None,
            };
            if let Some((key, kind)) = subject {
                let exists: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM events WHERE account=?1 AND source=?2
                       AND resource=?3 AND revision=?4 AND kind=?5)",
                    params![
                        key.account,
                        key.source.as_str(),
                        key.resource,
                        key.revision,
                        kind
                    ],
                    |row| row.get(0),
                )?;
                if !exists {
                    orphaned.push(finding.id);
                }
            }
        }
    }
    for finding in &orphaned {
        tx.execute(
            "DELETE FROM notification_outbox WHERE finding=?1",
            [finding],
        )?;
        tx.execute("DELETE FROM assessment_current WHERE finding=?1", [finding])?;
        tx.execute(
            "DELETE FROM assessment_revisions WHERE finding=?1",
            [finding],
        )?;
    }
    let cutoff = cutoff.to_rfc3339();
    tx.execute(
        "DELETE FROM notification_outbox AS outbox
         WHERE state!='pending' AND created_at<?1
           AND NOT EXISTS (SELECT 1 FROM assessment_current AS current
             WHERE current.finding=outbox.finding AND current.revision=outbox.revision)",
        [&cutoff],
    )?;
    tx.execute(
        "DELETE FROM assessment_revisions AS revisions
         WHERE assessed_at<?1
           AND NOT EXISTS (SELECT 1 FROM assessment_current AS current
             WHERE current.finding=revisions.finding AND current.revision=revisions.revision)
           AND NOT EXISTS (SELECT 1 FROM notification_outbox AS outbox
             WHERE outbox.finding=revisions.finding AND outbox.revision=revisions.revision)",
        [&cutoff],
    )?;
    Ok(orphaned.len())
}

fn ready_for_alert(finding: &Finding) -> bool {
    match &finding.detail {
        FindingDetail::Effect(effect) => {
            finding.route == Route::HighReview
                || (finding.route == Route::MediumReview && !effect.pending)
        }
        FindingDetail::Notice(_) => finding.route == Route::MediumReview,
        FindingDetail::Claim(_) => false,
        FindingDetail::Coverage(_) => finding.route == Route::MediumReview,
    }
}

fn material_change(old: &Finding, new: &Finding) -> bool {
    if old.route != new.route {
        return true;
    }
    match (&old.detail, &new.detail) {
        (FindingDetail::Effect(before), FindingDetail::Effect(after)) => {
            before.actor != after.actor
                || before.request_fit != after.request_fit
                || before.policy_ids != after.policy_ids
                || before.expected_feedback != after.expected_feedback
        }
        (FindingDetail::Claim(_), FindingDetail::Claim(_)) => false,
        (FindingDetail::Notice(_), FindingDetail::Notice(_)) => false,
        (FindingDetail::Coverage(before), FindingDetail::Coverage(after)) => {
            before.evidence.state != after.evidence.state
        }
        _ => true,
    }
}

impl OutboxKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Alert => "alert",
            Self::Correction => "correction",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "alert" => Some(Self::Alert),
            "correction" => Some(Self::Correction),
            _ => None,
        }
    }
}

pub(crate) fn pending(connection: &Connection) -> Result<OutboxBatch> {
    let mut statement = connection.prepare(
        "SELECT id,finding,revision,kind FROM notification_outbox
         WHERE state='pending' ORDER BY created_at,id LIMIT 129",
    )?;
    let mut items = statement
        .query_map([], |row| {
            let value: String = row.get(3)?;
            let kind = OutboxKind::from_str(&value).ok_or_else(|| {
                rusqlite::Error::InvalidColumnType(3, "kind".into(), rusqlite::types::Type::Text)
            })?;
            Ok(OutboxItem {
                id: row.get(0)?,
                finding: row.get(1)?,
                revision: row.get(2)?,
                kind,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let has_more = items.len() > 128;
    items.truncate(128);
    Ok(OutboxBatch { items, has_more })
}
