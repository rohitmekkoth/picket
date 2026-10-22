//! Privacy-bounded conversion of Google wire responses into portable observations.

use chrono::{DateTime, Utc};
use mail_parser::MessageParser;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::event::{
    Action, AdapterProvenance, ContactObserved, Effect, EmailMetadata, Event, EventKey,
    EventPayload, NoticeKind, Origin, Source, SourceTimeQuality,
};

#[derive(Debug, Error)]
pub enum GoogleWireError {
    #[error("invalid or incomplete Gmail metadata")]
    InvalidGmail,
    #[error("Gmail date is outside the supported range")]
    InvalidDate,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailMessage {
    pub id: String,
    pub thread_id: String,
    pub history_id: String,
    pub internal_date: String,
    #[serde(default)]
    pub label_ids: Vec<String>,
    pub payload: GmailPayload,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GmailPayload {
    #[serde(default)]
    pub headers: Vec<GmailHeader>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GmailHeader {
    pub name: String,
    pub value: String,
}

fn gmail_adapter() -> AdapterProvenance {
    AdapterProvenance {
        name: "google.gmail.metadata".into(),
        version: 1,
        scope: "https://www.googleapis.com/auth/gmail.metadata".into(),
    }
}

impl GmailMessage {
    pub fn observations(
        &self,
        account: &str,
        origin: Origin,
        ingested_at: DateTime<Utc>,
    ) -> Result<Vec<Event>, GoogleWireError> {
        if !valid_id(&self.id)
            || !valid_id(&self.thread_id)
            || !valid_id(&self.history_id)
            || self.payload.headers.len() > 64
            || self.label_ids.len() > 64
            || self.label_ids.iter().any(|label| label.len() > 256)
            || self
                .payload
                .headers
                .iter()
                .any(|header| header.name.len() > 128 || header.value.len() > 8192)
        {
            return Err(GoogleWireError::InvalidGmail);
        }
        let milliseconds: i64 = self
            .internal_date
            .parse()
            .map_err(|_| GoogleWireError::InvalidDate)?;
        let occurred_at =
            DateTime::from_timestamp_millis(milliseconds).ok_or(GoogleWireError::InvalidDate)?;
        let mut events = Vec::new();
        let sender = self.addresses("From")?;
        let subject = self.header("Subject").unwrap_or_default();
        let mut labels = self.label_ids.clone();
        labels.sort();
        labels.dedup();

        if labels.iter().any(|label| label == "SENT") {
            let mut recipients = Vec::new();
            for name in ["To", "Cc", "Bcc"] {
                recipients.extend(self.addresses(name)?);
            }
            recipients.sort();
            recipients.dedup();
            let subject_hash = format!("{:x}", Sha256::digest(subject.as_bytes()));
            events.push(Event {
                schema_version: 1,
                key: EventKey {
                    account: account.into(),
                    source: Source::Gmail,
                    resource: self.id.clone(),
                    revision: "sent".into(),
                },
                occurred_at,
                ingested_at,
                source_time_quality: Some(SourceTimeQuality::Provider),
                adapter: Some(gmail_adapter()),
                payload: EventPayload::Effect(Effect {
                    action: Action::EmailSend,
                    targets_complete: !recipients.is_empty(),
                    targets: recipients,
                    calendar_start: None,
                    origin: origin.clone(),
                    external_change: false,
                    email: Some(EmailMetadata {
                        thread_id: self.thread_id.clone(),
                        sender: sender.first().cloned(),
                        subject_hash,
                        subject_snippet: subject.chars().take(80).collect(),
                        labels: vec!["SENT".into()],
                    }),
                    calendar: None,
                }),
            });
        }

        if labels.iter().any(|label| label == "INBOX") {
            if let Some(address) = sender.first() {
                events.push(Event {
                    schema_version: 1,
                    key: EventKey {
                        account: account.into(),
                        source: Source::Gmail,
                        resource: self.id.clone(),
                        revision: "inbox-contact".into(),
                    },
                    occurred_at,
                    ingested_at,
                    source_time_quality: Some(SourceTimeQuality::Provider),
                    adapter: Some(gmail_adapter()),
                    payload: EventPayload::ContactObserved(ContactObserved {
                        address: address.clone(),
                        origin: origin.clone(),
                    }),
                });
            }
            if let Some(kind) = notice_kind(subject) {
                events.push(Event {
                    schema_version: 1,
                    key: EventKey {
                        account: account.into(),
                        source: Source::Gmail,
                        resource: self.id.clone(),
                        revision: format!("notice/{kind:?}").to_lowercase(),
                    },
                    occurred_at,
                    ingested_at,
                    source_time_quality: Some(SourceTimeQuality::Provider),
                    adapter: Some(gmail_adapter()),
                    payload: EventPayload::Notice { kind, origin },
                });
            }
        }
        Ok(events)
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.payload
            .headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.as_str())
    }

    fn addresses(&self, name: &str) -> Result<Vec<String>, GoogleWireError> {
        let Some(value) = self.header(name) else {
            return Ok(Vec::new());
        };
        if value.len() > 8192 || value.chars().any(char::is_control) {
            return Err(GoogleWireError::InvalidGmail);
        }
        let raw = format!("{name}: {value}\r\n\r\n");
        let parsed = MessageParser::new()
            .with_address_headers()
            .parse_headers(raw.as_bytes())
            .ok_or(GoogleWireError::InvalidGmail)?;
        let parsed = parsed
            .headers()
            .first()
            .and_then(|header| header.value().as_address())
            .ok_or(GoogleWireError::InvalidGmail)?;
        let mut addresses = Vec::new();
        for item in parsed.iter() {
            let address = item.address().ok_or(GoogleWireError::InvalidGmail)?;
            addresses.push(normalize_address(address)?);
        }
        Ok(addresses)
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn normalize_address(value: &str) -> Result<String, GoogleWireError> {
    let value = value.trim().to_lowercase();
    if value.is_empty()
        || value.len() > 320
        || !value.contains('@')
        || value.contains(char::is_whitespace)
    {
        return Err(GoogleWireError::InvalidGmail);
    }
    Ok(value)
}

fn notice_kind(subject: &str) -> Option<NoticeKind> {
    let subject = subject.to_lowercase();
    if ["order confirmation", "your order", "receipt"]
        .iter()
        .any(|needle| subject.contains(needle))
    {
        Some(NoticeKind::Purchase)
    } else if subject.contains("forwarding") {
        Some(NoticeKind::Forwarding)
    } else if subject.contains("app access") || subject.contains("third-party access") {
        Some(NoticeKind::AppAccess)
    } else if subject.contains("verification code") || subject.contains("one-time code") {
        Some(NoticeKind::Otp)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gmail_metadata_is_minimized_and_actor_unknown() -> Result<(), Box<dyn std::error::Error>> {
        let message: GmailMessage = serde_json::from_str(
            r#"{"id":"abc123","threadId":"thread123","historyId":"55","internalDate":"1760000000000","labelIds":["SENT"],"payload":{"headers":[{"name":"From","value":"Me <me@example.test>"},{"name":"To","value":"Person <Person@example.test>"},{"name":"Subject","value":"Private synthetic subject"}]}}"#,
        )?;
        let events = message.observations("account-1", Origin::Live, Utc::now())?;
        assert_eq!(events.len(), 1);
        let EventPayload::Effect(effect) = &events[0].payload else {
            return Err("expected effect".into());
        };
        assert_eq!(effect.targets, vec!["person@example.test"]);
        assert_eq!(
            effect
                .email
                .as_ref()
                .map(|email| email.subject_snippet.as_str()),
            Some("Private synthetic subject")
        );
        assert!(!serde_json::to_string(&events)?.contains("body"));
        Ok(())
    }

    #[test]
    fn inbox_produces_contact_and_qualified_notice_without_body()
    -> Result<(), Box<dyn std::error::Error>> {
        let message: GmailMessage = serde_json::from_str(
            r#"{"id":"abc124","threadId":"thread124","historyId":"56","internalDate":"1760000000000","labelIds":["INBOX"],"payload":{"headers":[{"name":"From","value":"Store <sales@example.test>"},{"name":"Subject","value":"Your order receipt"}]}}"#,
        )?;
        let events = message.observations("account-1", Origin::Live, Utc::now())?;
        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0].payload,
            EventPayload::ContactObserved(_)
        ));
        assert!(matches!(
            events[1].payload,
            EventPayload::Notice {
                kind: NoticeKind::Purchase,
                ..
            }
        ));
        assert!(!serde_json::to_string(&events)?.contains("Your order receipt"));
        Ok(())
    }
}
