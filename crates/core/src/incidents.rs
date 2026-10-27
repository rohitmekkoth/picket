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
