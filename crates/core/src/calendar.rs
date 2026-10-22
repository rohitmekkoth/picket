//! Primary Calendar wire normalization and revision classification.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::event::{
    Action, AdapterProvenance, CalendarMetadata, Effect, Event, EventKey, EventPayload, Origin,
    Source, SourceTimeQuality,
};

#[derive(Debug, Error)]
#[error("invalid or incomplete Calendar event")]
pub struct CalendarError;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEvent {
    pub id: String,
    pub etag: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub updated: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub start: Option<CalendarWhen>,
    #[serde(default)]
    pub end: Option<CalendarWhen>,
    #[serde(default)]
    pub organizer: Option<CalendarPerson>,
    #[serde(default)]
    pub creator: Option<CalendarPerson>,
    #[serde(default)]
    pub attendees: Vec<CalendarAttendee>,
    #[serde(default)]
    pub recurring_event_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarWhen {
    #[serde(default)]
    pub date_time: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CalendarPerson {
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default, rename = "self")]
    pub is_self: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarAttendee {
    pub email: String,
    #[serde(default)]
    pub response_status: Option<String>,
    #[serde(default, rename = "self")]
    pub is_self: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CalendarState {
    pub etag: String,
    pub deleted: bool,
    pub metadata: CalendarMetadata,
    pub start: Option<DateTime<Utc>>,
    pub external_change: bool,
}

impl CalendarEvent {
    pub fn observation(
        &self,
        account: &str,
        origin: Origin,
        ingested_at: DateTime<Utc>,
        previous: Option<&CalendarState>,
    ) -> Result<(Option<Event>, CalendarState), CalendarError> {
        if self.id.is_empty()
            || self.id.len() > 512
            || self.id.chars().any(char::is_control)
            || self.etag.is_empty()
            || self.etag.len() > 512
            || self.etag.chars().any(char::is_control)
            || self.attendees.len() > 500
            || self
                .recurring_event_id
                .as_ref()
                .is_some_and(|id| id.len() > 512 || id.chars().any(char::is_control))
        {
            return Err(CalendarError);
        }
        let deleted = self.status.as_deref() == Some("cancelled");
        let start = self
            .start
            .as_ref()
            .map(CalendarWhen::parse)
            .transpose()?
            .flatten()
            .or_else(|| previous.and_then(|state| state.start));
        let end = self
            .end
            .as_ref()
            .map(CalendarWhen::parse)
            .transpose()?
            .flatten()
            .or_else(|| previous.and_then(|state| state.metadata.end));
        if !deleted && self.start.is_none() {
            return Err(CalendarError);
        }
        let mut attendees = self
            .attendees
            .iter()
            .map(|attendee| normalize_address(&attendee.email))
            .collect::<Result<Vec<_>, _>>()?;
        attendees.sort();
        attendees.dedup();
        let self_rsvp = self
            .attendees
            .iter()
            .find(|attendee| attendee.is_self)
            .and_then(|attendee| attendee.response_status.clone())
            .or_else(|| {
                deleted
                    .then(|| previous.and_then(|state| state.metadata.self_rsvp.clone()))
                    .flatten()
            });
        let organizer = person_email(self.organizer.as_ref())?
            .or_else(|| previous.and_then(|state| state.metadata.organizer.clone()));
        let creator = person_email(self.creator.as_ref())?
            .or_else(|| previous.and_then(|state| state.metadata.creator.clone()));
        let metadata = CalendarMetadata {
            end,
            summary: self
                .summary
                .as_ref()
                .map(|value| value.chars().filter(|c| !c.is_control()).take(80).collect())
                .or_else(|| previous.and_then(|state| state.metadata.summary.clone())),
            organizer,
            creator,
            attendees: if attendees.is_empty() && deleted {
                previous.map_or_else(Vec::new, |state| state.metadata.attendees.clone())
            } else {
                attendees
            },
            self_rsvp,
            all_day: self.start.as_ref().map_or_else(
                || previous.is_some_and(|state| state.metadata.all_day),
                |when| when.date.is_some(),
            ),
            recurring_event_id: self
                .recurring_event_id
                .clone()
                .or_else(|| previous.and_then(|state| state.metadata.recurring_event_id.clone())),
        };
        let state = CalendarState {
            etag: self.etag.clone(),
            deleted,
            metadata,
            start,
            external_change: self.organizer.as_ref().map_or_else(
                || previous.is_some_and(|state| state.external_change),
                |person| !person.is_self,
            ),
        };
        if previous.is_some_and(|previous| previous.etag == self.etag) {
            return if previous == Some(&state) {
                Ok((None, state))
            } else {
                Err(CalendarError)
            };
        }
        if previous.is_none() && deleted {
            return Ok((None, state));
        }
        let action = if deleted {
            Action::CalendarDelete
        } else if let Some(previous) = previous {
            let mut prior_metadata = previous.metadata.clone();
            prior_metadata.self_rsvp = state.metadata.self_rsvp.clone();
            if previous.metadata.self_rsvp != state.metadata.self_rsvp
                && previous.start == state.start
                && previous.external_change == state.external_change
                && prior_metadata == state.metadata
            {
                Action::CalendarRsvp
            } else {
                Action::CalendarUpdate
            }
        } else {
            Action::CalendarCreate
        };
        let provider_time = self
            .updated
            .as_deref()
            .map(DateTime::parse_from_rfc3339)
            .transpose()
            .map_err(|_| CalendarError)?
            .map(|value| value.with_timezone(&Utc));
        let has_provider_time = provider_time.is_some();
        let occurred_at = provider_time.unwrap_or(ingested_at);
        let event = Event {
            schema_version: 1,
            key: EventKey {
                account: account.into(),
                source: Source::Calendar,
                resource: self.id.clone(),
                revision: self.etag.clone(),
            },
            occurred_at,
            ingested_at,
            source_time_quality: Some(if has_provider_time {
                SourceTimeQuality::Provider
            } else {
                SourceTimeQuality::IngestedFallback
            }),
            adapter: Some(AdapterProvenance {
                name: "google.calendar.events".into(),
                version: 1,
                scope: "https://www.googleapis.com/auth/calendar.events.readonly".into(),
            }),
            payload: EventPayload::Effect(Effect {
                action,
                targets: state.metadata.attendees.clone(),
                targets_complete: !self.attendees.is_empty() || (deleted && previous.is_some()),
                calendar_start: state.start,
                origin,
                external_change: state.external_change,
                email: None,
                calendar: Some(state.metadata.clone()),
            }),
        };
        Ok((Some(event), state))
    }
}

impl CalendarWhen {
    fn parse(&self) -> Result<Option<DateTime<Utc>>, CalendarError> {
        if let Some(value) = &self.date_time {
            Ok(Some(
                DateTime::parse_from_rfc3339(value)
                    .map_err(|_| CalendarError)?
                    .with_timezone(&Utc),
            ))
        } else if self
            .date
            .as_ref()
            .is_some_and(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok())
        {
            Ok(None)
        } else {
            Err(CalendarError)
        }
    }
}

fn person_email(person: Option<&CalendarPerson>) -> Result<Option<String>, CalendarError> {
    person
        .and_then(|person| person.email.as_deref())
        .map(normalize_address)
        .transpose()
}

fn normalize_address(address: &str) -> Result<String, CalendarError> {
    let address = address.trim().to_lowercase();
    if address.is_empty()
        || address.len() > 320
        || !address.contains('@')
        || address.contains(char::is_whitespace)
    {
        return Err(CalendarError);
    }
    Ok(address)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(json: &str) -> Result<CalendarEvent, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn revisions_distinguish_create_rsvp_and_delete() -> Result<(), Box<dyn std::error::Error>> {
        let first = event(
            r#"{"id":"event_1","etag":"rev1","updated":"2026-10-22T10:00:00Z","summary":"Synthetic meeting","start":{"dateTime":"2026-10-23T09:00:00-07:00"},"end":{"dateTime":"2026-10-23T10:00:00-07:00"},"organizer":{"email":"host@example.test","self":false},"attendees":[{"email":"me@example.test","self":true,"responseStatus":"needsAction"}]}"#,
        )?;
        let (created, state) = first.observation("account-1", Origin::Live, Utc::now(), None)?;
        let created = created.ok_or("missing creation")?;
        let EventPayload::Effect(effect) = &created.payload else {
            return Err("expected effect".into());
        };
        assert_eq!(effect.action, Action::CalendarCreate);
        assert_eq!(
            created.source_time_quality,
            Some(SourceTimeQuality::Provider)
        );
        assert!(effect.external_change);
        assert_eq!(effect.targets, vec!["me@example.test"]);
        assert_eq!(
            effect.calendar_start.map(|date| date.to_rfc3339()),
            Some("2026-10-23T16:00:00+00:00".into())
        );
        assert!(
            first
                .observation("account-1", Origin::Live, Utc::now(), Some(&state))?
                .0
                .is_none()
        );

        let revised = event(
            r#"{"id":"event_1","etag":"rev2","updated":"2026-10-22T10:01:00Z","summary":"Synthetic meeting","start":{"dateTime":"2026-10-23T09:00:00-07:00"},"end":{"dateTime":"2026-10-23T10:00:00-07:00"},"organizer":{"email":"host@example.test","self":false},"attendees":[{"email":"me@example.test","self":true,"responseStatus":"accepted"}]}"#,
        )?;
        let (rsvp, state) =
            revised.observation("account-1", Origin::Live, Utc::now(), Some(&state))?;
        let rsvp = rsvp.ok_or("missing RSVP")?;
        assert!(matches!(
            rsvp.payload,
            EventPayload::Effect(Effect {
                action: Action::CalendarRsvp,
                ..
            })
        ));

        let cancelled = event(r#"{"id":"event_1","etag":"rev3","status":"cancelled"}"#)?;
        let (deleted, _) =
            cancelled.observation("account-1", Origin::Live, Utc::now(), Some(&state))?;
        let deleted = deleted.ok_or("missing deletion")?;
        assert_eq!(
            deleted.source_time_quality,
            Some(SourceTimeQuality::IngestedFallback)
        );
        assert!(matches!(
            deleted.payload,
            EventPayload::Effect(Effect {
                action: Action::CalendarDelete,
                ..
            })
        ));
        Ok(())
    }

    #[test]
    fn all_day_event_keeps_time_unknown() -> Result<(), Box<dyn std::error::Error>> {
        let all_day = event(
            r#"{"id":"all_day","etag":"rev1","start":{"date":"2026-10-23"},"end":{"date":"2026-10-24"}}"#,
        )?;
        let (observed, state) =
            all_day.observation("account-1", Origin::Baseline, Utc::now(), None)?;
        assert!(observed.is_some());
        assert!(state.metadata.all_day);
        assert!(state.start.is_none());
        Ok(())
    }

    #[test]
    fn malformed_provider_time_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
        let malformed = event(
            r#"{"id":"event_1","etag":"rev1","updated":"bad","start":{"dateTime":"2026-10-23T09:00:00Z"}}"#,
        )?;
        assert!(
            malformed
                .observation("account-1", Origin::Live, Utc::now(), None)
                .is_err()
        );
        Ok(())
    }
}
