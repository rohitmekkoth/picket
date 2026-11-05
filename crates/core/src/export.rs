//! Exports of the person's own data: one incident as a Markdown report, and everything
//! Picket keeps as JSON. The desktop app writes them only where the person chooses; the
//! content is never logged.

use std::{fmt::Display, io::Write};

use chrono::{DateTime, TimeZone, Utc};
use serde_json::json;

use crate::{
    alerts::{incident_title, reason_explanation},
    event::Action,
    store::{Result, Store},
    views::{ActorBasis, HistoryKind, IncidentDetail, Severity, Status},
};

/// The incident as a self-contained Markdown report.
pub fn incident_markdown<Tz: TimeZone<Offset: Display>>(
    incident: &IncidentDetail,
    now: DateTime<Utc>,
    zone: &Tz,
    version: &str,
) -> String {
    let summary = &incident.summary;
    let agent = summary.agent.as_deref().unwrap_or("your agent");
    // A report is kept and shared, so every time carries its date.
    let at = |time: DateTime<Utc>| {
        time.with_timezone(zone)
            .format("%b %-d, %Y, %-I:%M %p")
            .to_string()
    };
    let mut out = format!("# {}\n\n", incident_title(summary));
    out += &format!(
        "- Severity: {}\n",
        match summary.severity {
            Severity::Urgent => "Urgent",
            Severity::Review => "Review",
        }
    );
    out += &format!(
        "- Status: {}\n",
        match summary.status {
            Status::Open => "Open",
            Status::Fine => "Marked as fine",
            Status::NotOk => "Marked as not OK",
        }
    );
    out += &format!("- Happened: {}\n", at(summary.occurred_at));
    out += &format!("- Flagged: {}\n", at(summary.flagged_at));
    let who = incident.actor.agents.first().map_or(agent, String::as_str);
    out += &format!(
        "- Who did it: {}\n",
        match incident.actor.basis {
            ActorBasis::Reported => format!("{who}, because it reported doing this"),
            ActorBasis::Runtime => format!("{who}, linked by its own record of the action"),
            ActorBasis::Confirmed => format!("{who}, as you confirmed"),
            ActorBasis::Conflicting => format!(
                "More than one agent reported this: {}",
                incident.actor.agents.join(", ")
            ),
            ActorBasis::Unknown => "Picket can’t tell. It could be you or someone else.".into(),
        }
    );

    out += "\n## Why this was flagged\n\n";
    for reason in &summary.reasons {
        out += &format!("- {}\n", reason_explanation(*reason));
    }

    out += "\n## What you asked\n\n";
    match &incident.request {
        Some(request) => {
            out += &format!(
                "{}: {}\n",
                at(request.at),
                request.message.as_deref().map_or(
                    "a request Picket read but did not keep the words of".into(),
                    quote
                )
            );
        }
        None => out += "No request was found in your iMessage chat with the agent.\n",
    }

    if !incident.reports.is_empty() {
        out += &format!("\n## What {agent} reported\n\n");
        for report in &incident.reports {
            out += &format!(
                "- {}: {}\n",
                at(report.at),
                report
                    .message
                    .as_deref()
                    .map_or_else(|| action_phrase(&report.action).into(), quote)
            );
        }
    }

    if let Some(effect) = &incident.effect {
        out += "\n## What happened\n\n";
        let mut line = format!("- {}: {}", at(effect.at), action_phrase(&effect.action));
        let names: Vec<String> = effect
            .people
            .iter()
            .map(|person| match &person.name {
                Some(name) => format!("{name} <{}>", person.address),
                None => person.address.clone(),
            })
            .collect();
        if !names.is_empty() {
            let joiner = if effect.action == Action::EmailSend {
                "to"
            } else {
                "with"
            };
            line += &format!(" {joiner} {}", names.join(", "));
        }
        if let Some(subject) = &effect.subject {
            line += &format!(", {}", quote(subject));
        }
        if let Some(start) = effect.calendar_start {
            line += &format!(", starting {}", at(start));
        }
        out += &line;
        out += "\n";
        let unrequested: Vec<_> = effect
            .people
            .iter()
            .filter(|person| person.unrequested)
            .map(|person| person.address.as_str())
            .collect();
        if !unrequested.is_empty() {
            out += &format!("- Not in your request: {}\n", unrequested.join(", "));
        }
    }

    if !incident.history.is_empty() {
        out += "\n## History\n\n";
        for item in &incident.history {
            let what = match item.kind {
                HistoryKind::Requested => format!("You asked {agent}"),
                HistoryKind::Happened => "It happened in your account".into(),
                HistoryKind::Reported => format!("{agent} reported it"),
                HistoryKind::Flagged => "Picket flagged it".into(),
                HistoryKind::Revised => "Picket revised its assessment after new evidence".into(),
                HistoryKind::MarkedFine => "You marked it as fine".into(),
                HistoryKind::MarkedNotOk => "You said it wasn’t OK".into(),
            };
            out += &format!("- {}: {what}\n", at(item.at));
        }
    }

    out += &format!(
        "\n---\n\nExported by Picket {version} on {}. Picket reports what it could see in Gmail, \
         Calendar and your iMessage chat with the agent; it cannot see requests made elsewhere.\n",
        now.with_timezone(zone).format("%b %-d, %Y")
    );
    out
}

fn quote(text: &str) -> String {
    format!("“{text}”")
}

fn action_phrase(action: &Action) -> &'static str {
    match action {
        Action::EmailSend => "Sent an email",
        Action::CalendarCreate => "Created an event",
        Action::CalendarUpdate => "Changed an event",
        Action::CalendarDelete => "Deleted an event",
        Action::CalendarRsvp => "Replied to an invite",
    }
}

/// Everything Picket keeps, as one JSON document: settings, every retained event in log
/// order with its hash, and the current findings. Written as it is read, so a large log
/// is not held in memory twice.
pub fn write_all(
    store: &Store,
    out: &mut impl Write,
    now: DateTime<Utc>,
    version: &str,
) -> Result<()> {
    let header = json!({
        "format": "picket-export",
        "formatVersion": 1,
        "picketVersion": version,
        "exportedAt": now,
        "settings": store.settings()?,
    });
    let header = serde_json::to_string(&header)?;
    // Open the header object again to append the two arrays.
    let open = header
        .strip_suffix('}')
        .ok_or(crate::store::StoreError::Invalid("export"))?;
    write!(out, "{open},\"events\":[").map_err(io)?;
    for (index, row) in store.events()?.iter().enumerate() {
        if index > 0 {
            out.write_all(b",").map_err(io)?;
        }
        serde_json::to_writer(
            &mut *out,
            &json!({ "sequence": row.sequence, "hash": hex(&row.hash), "event": row.event }),
        )?;
    }
    out.write_all(b"],\"findings\":[").map_err(io)?;
    for (index, (revision, finding)) in store.current_findings()?.iter().enumerate() {
        if index > 0 {
            out.write_all(b",").map_err(io)?;
        }
        serde_json::to_writer(
            &mut *out,
            &json!({ "revision": revision, "finding": finding }),
        )?;
    }
    out.write_all(b"]}\n").map_err(io)?;
    out.flush().map_err(io)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn io(_: std::io::Error) -> crate::store::StoreError {
    crate::store::StoreError::Invalid("export could not be written")
}
