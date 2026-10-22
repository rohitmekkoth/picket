//! Fixed-endpoint, read-only Google HTTP transport.

use std::{
    io::Read,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use picket_core::{calendar::CalendarEvent, google::GmailMessage};
use reqwest::{
    StatusCode, Url,
    blocking::{Client, Response},
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

const MAX_RESPONSE: u64 = 1024 * 1024;
const GMAIL_ROOT: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
const CALENDAR_ROOT: &str = "https://www.googleapis.com/calendar/v3/calendars/primary/events";
const CALENDAR_FIELDS: &str = "nextPageToken,nextSyncToken,items(id,etag,status,updated,summary,start,end,organizer(email,self),creator(email,self),attendees(email,responseStatus,self),recurringEventId)";

#[derive(Debug, Error)]
pub enum HttpError {
    #[error("Google authorization expired")]
    Unauthorized,
    #[error("Google denied the request: {0}")]
    Forbidden(&'static str),
    #[error("Google rate limit exceeded")]
    RateLimited,
    #[error("Google cursor expired")]
    ExpiredCursor,
    #[error("Google resource disappeared during collection")]
    MissingResource,
    #[error("Google request failed or exceeded its limit")]
    Request,
    #[error("Google returned HTTP {0}")]
    Status(u16),
    #[error("Google returned invalid metadata")]
    InvalidResponse,
}

pub struct GoogleHttp {
    client: Client,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailProfile {
    pub email_address: String,
    pub history_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailList {
    #[serde(default)]
    pub messages: Vec<GmailRef>,
    pub next_page_token: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GmailRef {
    pub id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailHistory {
    #[serde(default)]
    pub history: Vec<GmailHistoryRecord>,
    pub history_id: String,
    pub next_page_token: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailHistoryRecord {
    #[serde(default)]
    pub messages_added: Vec<GmailMessageAdded>,
    #[serde(default)]
    pub labels_added: Vec<GmailMessageAdded>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GmailMessageAdded {
    pub message: GmailRef,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarPage {
    #[serde(default)]
    pub items: Vec<CalendarEvent>,
    pub next_page_token: Option<String>,
    pub next_sync_token: Option<String>,
}

impl GoogleHttp {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub fn gmail_profile(&self, token: &str) -> Result<GmailProfile, HttpError> {
        self.get(&format!("{GMAIL_ROOT}/profile"), token, &[], false)
    }

    pub fn gmail_list(
        &self,
        token: &str,
        label: &str,
        page: Option<&str>,
    ) -> Result<GmailList, HttpError> {
        if !matches!(label, "SENT" | "INBOX") {
            return Err(HttpError::Request);
        }
        let mut query = vec![("labelIds", label), ("maxResults", "500")];
        if let Some(page) = page {
            query.push(("pageToken", page));
        }
        self.get(&format!("{GMAIL_ROOT}/messages"), token, &query, false)
    }

    pub fn gmail_message(&self, token: &str, id: &str) -> Result<GmailMessage, HttpError> {
        if id.is_empty() || id.len() > 256 || !id.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err(HttpError::Request);
        }
        // A metadata lookup costs 20 Gmail quota units. Pace baseline imports below
        // the current 6,000-unit per-user minute limit, with room for other clients.
        thread::sleep(Duration::from_millis(300));
        self.get(
            &format!("{GMAIL_ROOT}/messages/{id}"),
            token,
            &[
                ("format", "metadata"),
                ("metadataHeaders", "From"),
                ("metadataHeaders", "To"),
                ("metadataHeaders", "Cc"),
                ("metadataHeaders", "Bcc"),
                ("metadataHeaders", "Subject"),
                ("metadataHeaders", "Date"),
                ("metadataHeaders", "Message-ID"),
                ("metadataHeaders", "In-Reply-To"),
            ],
            false,
        )
    }

    pub fn gmail_history(
        &self,
        token: &str,
        cursor: &str,
        page: Option<&str>,
    ) -> Result<GmailHistory, HttpError> {
        if cursor.is_empty() || cursor.len() > 256 {
            return Err(HttpError::Request);
        }
        let mut query = vec![
            ("startHistoryId", cursor),
            ("historyTypes", "messageAdded"),
            ("historyTypes", "labelAdded"),
            ("maxResults", "500"),
        ];
        if let Some(page) = page {
            query.push(("pageToken", page));
        }
        self.get(&format!("{GMAIL_ROOT}/history"), token, &query, true)
    }

    pub fn calendar_list(
        &self,
        token: &str,
        sync: Option<&str>,
        page: Option<&str>,
        bounds: Option<(&str, &str)>,
    ) -> Result<CalendarPage, HttpError> {
        if sync.is_some() && bounds.is_some() {
            return Err(HttpError::Request);
        }
        let mut query = vec![
            ("showDeleted", "true"),
            ("singleEvents", "true"),
            ("maxResults", "250"),
            ("fields", CALENDAR_FIELDS),
        ];
        if let Some(sync) = sync {
            query.push(("syncToken", sync));
        }
        if let Some(page) = page {
            query.push(("pageToken", page));
        }
        if let Some((from, to)) = bounds {
            query.push(("timeMin", from));
            query.push(("timeMax", to));
        }
        self.get(CALENDAR_ROOT, token, &query, true)
    }

    fn get<T: DeserializeOwned>(
        &self,
        endpoint: &str,
        token: &str,
        query: &[(&str, &str)],
        cursor_request: bool,
    ) -> Result<T, HttpError> {
        let mut url = Url::parse(endpoint).map_err(|_| HttpError::Request)?;
        if url.scheme() != "https"
            || !matches!(
                url.host_str(),
                Some("gmail.googleapis.com" | "www.googleapis.com")
            )
        {
            return Err(HttpError::Request);
        }
        url.query_pairs_mut().extend_pairs(query.iter().copied());
        for attempt in 0..5 {
            let response = self
                .client
                .get(url.clone())
                .bearer_auth(token)
                .send()
                .map_err(|_| HttpError::Request)?;
            match response.status() {
                StatusCode::UNAUTHORIZED => {
                    return Err(HttpError::Unauthorized);
                }
                StatusCode::FORBIDDEN => {
                    let error = forbidden_error(response);
                    if matches!(error, HttpError::RateLimited) {
                        if attempt == 4 {
                            return Err(HttpError::RateLimited);
                        }
                        thread::sleep(retry_delay(attempt));
                        continue;
                    }
                    return Err(error);
                }
                StatusCode::NOT_FOUND | StatusCode::GONE if cursor_request => {
                    return Err(HttpError::ExpiredCursor);
                }
                StatusCode::NOT_FOUND => return Err(HttpError::MissingResource),
                status if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() => {
                    if attempt == 4 {
                        return Err(if status == StatusCode::TOO_MANY_REQUESTS {
                            HttpError::RateLimited
                        } else {
                            HttpError::Status(status.as_u16())
                        });
                    }
                    thread::sleep(retry_delay(attempt));
                    continue;
                }
                status if !status.is_success() => {
                    return Err(HttpError::Status(status.as_u16()));
                }
                _ => {}
            }
            let mut bytes = Vec::new();
            response
                .take(MAX_RESPONSE + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| HttpError::Request)?;
            if bytes.len() as u64 > MAX_RESPONSE {
                return Err(HttpError::Request);
            }
            return serde_json::from_slice(&bytes).map_err(|_| HttpError::InvalidResponse);
        }
        Err(HttpError::Request)
    }
}

fn retry_delay(attempt: usize) -> Duration {
    let jitter = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |time| u64::from(time.subsec_millis()));
    Duration::from_millis((1_u64 << attempt) * 1000 + jitter)
}

fn forbidden_error(response: Response) -> HttpError {
    let Ok(value) = serde_json::from_reader::<_, serde_json::Value>(response.take(8192)) else {
        return HttpError::Forbidden("unspecified");
    };
    classify_forbidden(&value)
}

fn classify_forbidden(value: &serde_json::Value) -> HttpError {
    let reason = value
        .pointer("/error/errors/0/reason")
        .and_then(|item| item.as_str());
    match reason {
        Some("rateLimitExceeded" | "userRateLimitExceeded") => HttpError::RateLimited,
        Some("dailyLimitExceeded") => HttpError::Forbidden("daily limit exceeded"),
        Some("quotaExceeded") => HttpError::Forbidden("quota exceeded"),
        Some("insufficientPermissions") => HttpError::Forbidden("insufficient permissions"),
        Some("accessNotConfigured") => HttpError::Forbidden("API access not configured"),
        _ => HttpError::Forbidden("unspecified"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_sync_rejects_incompatible_bounds() -> Result<(), Box<dyn std::error::Error>> {
        let client = Client::builder().build()?;
        let http = GoogleHttp::new(client);
        assert!(
            http.calendar_list("synthetic", Some("sync"), None, Some(("a", "b")))
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn forbidden_errors_classify_rate_limits_without_leaking_provider_text() {
        let rate = serde_json::json!({"error": {"errors": [{"reason": "rateLimitExceeded"}]}});
        assert!(matches!(classify_forbidden(&rate), HttpError::RateLimited));
        let unexpected = serde_json::json!({"error": {"errors": [{"reason": "private data"}]}});
        assert!(matches!(
            classify_forbidden(&unexpected),
            HttpError::Forbidden("unspecified")
        ));
    }
}
