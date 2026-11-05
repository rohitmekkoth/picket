//! Built-in rules, their default levels, and the user's boundaries. The engine reads
//! the latest recorded rule set; each change is one event, so replay is deterministic.

use std::collections::BTreeSet;

use chrono::{DateTime, Datelike, Duration, Timelike, Utc, Weekday};
use thiserror::Error;

use crate::event::{Action, Boundary, BoundaryKind, Days, Effect, Level, RuleSet};

pub const PROHIBITED_ACTION: &str = "request.prohibited_action";
pub const APPROVAL_MISSING: &str = "request.approval_missing";
pub const EXTRA_TARGET: &str = "request.extra_target";
pub const DIFFERENT_TIME: &str = "request.different_time";
pub const TOO_MANY: &str = "request.too_many";
pub const DIFFERENT_ACTION: &str = "request.different_action";
pub const OUTSIDE_WINDOW: &str = "request.outside_window";
pub const AFTER_REVOCATION: &str = "request.after_revocation";
pub const REPORT_NOT_OBSERVED: &str = "report.not_observed";
pub const REPORT_CONTRADICTED: &str = "report.contradicted";
pub const FORWARDING: &str = "warning.forwarding";
pub const APP_ACCESS: &str = "warning.app_access";
pub const OTP: &str = "warning.otp";
pub const PURCHASE: &str = "warning.purchase";
pub const GOOGLE_ACCESS: &str = "visibility.google";
pub const MESSAGES_ACCESS: &str = "visibility.messages";
pub const CATCH_UP: &str = "visibility.catch_up";

/// Every built-in rule with its default level. Request checks need review, not an
/// immediate alert: the model can misread a request, as the replay showed. Account
/// warnings are indicators from email subjects, not proof of a change.
pub const BUILT_IN: &[(&str, Level)] = &[
    (PROHIBITED_ACTION, Level::Review),
    (APPROVAL_MISSING, Level::Review),
    (EXTRA_TARGET, Level::Review),
    (DIFFERENT_TIME, Level::Review),
    (TOO_MANY, Level::Review),
    (DIFFERENT_ACTION, Level::Review),
    (OUTSIDE_WINDOW, Level::Review),
    (AFTER_REVOCATION, Level::Urgent),
    (REPORT_NOT_OBSERVED, Level::Review),
    (REPORT_CONTRADICTED, Level::Review),
    (FORWARDING, Level::Review),
    (APP_ACCESS, Level::Review),
    (OTP, Level::Off),
    (PURCHASE, Level::Summary),
    (GOOGLE_ACCESS, Level::Review),
    (MESSAGES_ACCESS, Level::Review),
    (CATCH_UP, Level::Summary),
];

const MAX_BOUNDARIES: usize = 64;
const MAX_ENTRIES: usize = 100;
const MAX_ID: usize = 64;
/// A recorded `since` may run slightly ahead of the event that records it.
const CLOCK_SKEW: Duration = Duration::minutes(5);

#[derive(Debug, Error, Eq, PartialEq)]
pub enum RuleError {
    #[error("unknown rule")]
    UnknownRule,
    #[error("too many boundaries")]
    TooManyBoundaries,
    #[error("invalid boundary id")]
    InvalidId,
    #[error("duplicate boundary id")]
    DuplicateId,
    #[error("a boundary cannot start in the future")]
    FutureStart,
    #[error("enter at least one address or domain")]
    NoTargets,
    #[error("too many addresses or domains")]
    TooManyTargets,
    #[error("invalid email address")]
    InvalidAddress,
    #[error("invalid domain")]
    InvalidDomain,
    #[error("choose at least one supported action")]
    InvalidActions,
    #[error("invalid time of day")]
    InvalidTime,
    #[error("unknown time zone")]
    InvalidZone,
    #[error("the count must be between 1 and 1000")]
    InvalidCount,
    #[error("the window must be between 1 minute and 24 hours")]
    InvalidWindow,
}

pub fn default_level(id: &str) -> Option<Level> {
    BUILT_IN
        .iter()
        .find(|(rule, _)| *rule == id)
        .map(|(_, level)| *level)
}

/// The level of a built-in rule in a rule set, or its default.
pub fn level(rules: Option<&RuleSet>, id: &str) -> Level {
    rules
        .and_then(|rules| rules.levels.get(id).copied())
        .or_else(|| default_level(id))
        .unwrap_or(Level::Off)
}

pub fn validate(rules: &RuleSet, recorded_at: DateTime<Utc>) -> Result<(), RuleError> {
    if rules.levels.keys().any(|id| default_level(id).is_none()) {
        return Err(RuleError::UnknownRule);
    }
    if rules.boundaries.len() > MAX_BOUNDARIES {
        return Err(RuleError::TooManyBoundaries);
    }
    let mut ids = BTreeSet::new();
    for boundary in &rules.boundaries {
        validate_boundary(boundary, recorded_at)?;
        if !ids.insert(boundary.id.as_str()) {
            return Err(RuleError::DuplicateId);
        }
    }
    Ok(())
}

fn validate_boundary(boundary: &Boundary, recorded_at: DateTime<Utc>) -> Result<(), RuleError> {
    let id_ok = !boundary.id.is_empty()
        && boundary.id.len() <= MAX_ID
        && boundary
            .id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if !id_ok {
        return Err(RuleError::InvalidId);
    }
    if boundary.since > recorded_at + CLOCK_SKEW {
        return Err(RuleError::FutureStart);
    }
    let sends = [
        Action::EmailSend,
        Action::CalendarCreate,
        Action::CalendarUpdate,
    ];
    let calendar = [
        Action::CalendarCreate,
        Action::CalendarUpdate,
        Action::CalendarDelete,
    ];
    let actions_ok = |actions: &[Action], allowed: &[Action]| {
        !actions.is_empty()
            && actions.len() <= allowed.len()
            && actions.iter().all(|action| allowed.contains(action))
    };
    match &boundary.kind {
        BoundaryKind::Contact {
            addresses,
            domains,
            actions,
        } => {
            if addresses.is_empty() && domains.is_empty() {
                return Err(RuleError::NoTargets);
            }
            if addresses.len() > MAX_ENTRIES || domains.len() > MAX_ENTRIES {
                return Err(RuleError::TooManyTargets);
            }
            if !addresses.iter().all(|address| valid_address(address)) {
                return Err(RuleError::InvalidAddress);
            }
            if !domains.iter().all(|domain| valid_domain(domain)) {
                return Err(RuleError::InvalidDomain);
            }
            if !actions_ok(actions, &sends) {
                return Err(RuleError::InvalidActions);
            }
        }
        BoundaryKind::CalendarChange { actions } => {
            if !actions_ok(actions, &calendar) {
                return Err(RuleError::InvalidActions);
            }
        }
        BoundaryKind::QuietHours {
            start_minute,
            end_minute,
            zone,
            actions,
            ..
        } => {
            if *start_minute >= 1440 || *end_minute >= 1440 || start_minute == end_minute {
                return Err(RuleError::InvalidTime);
            }
            if zone.parse::<chrono_tz::Tz>().is_err() {
                return Err(RuleError::InvalidZone);
            }
            if !actions_ok(actions, &sends) {
                return Err(RuleError::InvalidActions);
            }
        }
        BoundaryKind::Burst {
            count,
            window_minutes,
            actions,
        } => {
            if !(1..=1000).contains(count) {
                return Err(RuleError::InvalidCount);
            }
            if !(1..=1440).contains(window_minutes) {
                return Err(RuleError::InvalidWindow);
            }
            if !actions_ok(
                actions,
                &[sends.as_slice(), &[Action::CalendarDelete]].concat(),
            ) {
                return Err(RuleError::InvalidActions);
            }
        }
        BoundaryKind::NewRecipient => {}
    }
    Ok(())
}

/// Addresses are stored as collectors normalize them: trimmed and lower case.
pub fn normalize_address(value: &str) -> String {
    value.trim().to_lowercase()
}

/// Domains are stored without a leading `@`, lower case.
pub fn normalize_domain(value: &str) -> String {
    value.trim().trim_start_matches('@').to_lowercase()
}

fn valid_address(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && value.len() <= 254
        && value == normalize_address(value)
        && !value
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
        && valid_domain(domain)
}

fn valid_domain(value: &str) -> bool {
    value.len() <= 253
        && value.contains('.')
        && !value.starts_with('.')
        && !value.ends_with('.')
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.' || byte == b'-'
        })
}

/// Whether an effect crosses a boundary that needs no other evidence. Bursts and new
/// recipients depend on other events and are checked by the engine.
pub(crate) fn crosses(kind: &BoundaryKind, effect: &Effect, at: DateTime<Utc>) -> bool {
    match kind {
        BoundaryKind::Contact {
            addresses,
            domains,
            actions,
        } => {
            actions.contains(&effect.action)
                && effect.targets.iter().any(|target| {
                    addresses.contains(target)
                        || target.rsplit_once('@').is_some_and(|(_, domain)| {
                            domains.iter().any(|blocked| {
                                domain == blocked || domain.ends_with(&format!(".{blocked}"))
                            })
                        })
                })
        }
        BoundaryKind::CalendarChange { actions } => actions.contains(&effect.action),
        BoundaryKind::QuietHours {
            start_minute,
            end_minute,
            days,
            zone,
            actions,
        } => {
            if !actions.contains(&effect.action) {
                return false;
            }
            let Ok(zone) = zone.parse::<chrono_tz::Tz>() else {
                return false;
            };
            let local = at.with_timezone(&zone);
            let minute = (local.hour() * 60 + local.minute()) as u16;
            let crosses_midnight = end_minute < start_minute;
            let inside = if crosses_midnight {
                minute >= *start_minute || minute < *end_minute
            } else {
                minute >= *start_minute && minute < *end_minute
            };
            // A night that crosses midnight belongs to the day it started.
            let day = if crosses_midnight && minute < *end_minute {
                local.weekday().pred()
            } else {
                local.weekday()
            };
            let weekend = matches!(day, Weekday::Sat | Weekday::Sun);
            inside
                && match days {
                    Days::Every => true,
                    Days::Weekdays => !weekend,
                    Days::Weekends => weekend,
                }
        }
        BoundaryKind::Burst { .. } | BoundaryKind::NewRecipient => false,
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::event::Origin;

    fn email(to: &str) -> Effect {
        Effect {
            action: Action::EmailSend,
            targets: vec![to.into()],
            targets_complete: true,
            calendar_start: None,
            origin: Origin::Live,
            external_change: false,
            target_names: Default::default(),
            email: None,
            calendar: None,
        }
    }

    fn quiet(days: Days) -> BoundaryKind {
        BoundaryKind::QuietHours {
            start_minute: 22 * 60,
            end_minute: 7 * 60,
            days,
            zone: "America/Los_Angeles".into(),
            actions: vec![Action::EmailSend],
        }
    }

    #[test]
    fn contact_boundaries_match_addresses_domains_and_subdomains_only() {
        let kind = BoundaryKind::Contact {
            addresses: vec!["sam@lee.example".into()],
            domains: vec!["bank.example".into()],
            actions: vec![Action::EmailSend],
        };
        let at = Utc::now();
        assert!(crosses(&kind, &email("sam@lee.example"), at));
        assert!(crosses(&kind, &email("help@bank.example"), at));
        assert!(crosses(&kind, &email("help@mail.bank.example"), at));
        assert!(!crosses(&kind, &email("help@notbank.example"), at));
        assert!(!crosses(&kind, &email("other@lee.example"), at));
    }

    #[test]
    fn quiet_hours_follow_the_local_clock_across_midnight_and_daylight_time() {
        // 23:30 Pacific on a Friday in winter and in summer.
        let winter = Utc.with_ymd_and_hms(2026, 1, 10, 7, 30, 0).single();
        let summer = Utc.with_ymd_and_hms(2026, 7, 11, 6, 30, 0).single();
        let noon = Utc.with_ymd_and_hms(2026, 7, 10, 19, 0, 0).single();
        // 06:30 Pacific on a Saturday belongs to Friday night.
        let early = Utc.with_ymd_and_hms(2026, 7, 11, 13, 30, 0).single();
        let effect = email("a@b.example");
        for at in [winter, summer, early].into_iter().flatten() {
            assert!(crosses(&quiet(Days::Every), &effect, at));
            assert!(crosses(&quiet(Days::Weekdays), &effect, at));
            assert!(!crosses(&quiet(Days::Weekends), &effect, at));
        }
        assert!(noon.is_some_and(|at| !crosses(&quiet(Days::Every), &effect, at)));
    }

    #[test]
    fn rule_sets_are_validated_strictly() {
        let at = Utc::now();
        let boundary = |kind| RuleSet {
            levels: Default::default(),
            boundaries: vec![Boundary {
                id: "b-1".into(),
                level: Level::Urgent,
                since: at,
                kind,
            }],
        };
        assert_eq!(validate(&boundary(quiet(Days::Every)), at), Ok(()));
        let bad_zone = BoundaryKind::QuietHours {
            start_minute: 0,
            end_minute: 60,
            days: Days::Every,
            zone: "Mars/Olympus".into(),
            actions: vec![Action::EmailSend],
        };
        assert_eq!(
            validate(&boundary(bad_zone), at),
            Err(RuleError::InvalidZone)
        );
        let upper = BoundaryKind::Contact {
            addresses: vec!["Sam@Lee.example".into()],
            domains: vec![],
            actions: vec![Action::EmailSend],
        };
        assert_eq!(
            validate(&boundary(upper), at),
            Err(RuleError::InvalidAddress)
        );
        let delete_contact = BoundaryKind::Contact {
            addresses: vec![],
            domains: vec!["bank.example".into()],
            actions: vec![Action::CalendarDelete],
        };
        assert_eq!(
            validate(&boundary(delete_contact), at),
            Err(RuleError::InvalidActions)
        );
        let mut levels = RuleSet::default();
        levels.levels.insert("request.unknown".into(), Level::Off);
        assert_eq!(validate(&levels, at), Err(RuleError::UnknownRule));
        assert_eq!(level(None, OTP), Level::Off);
        assert_eq!(level(None, AFTER_REVOCATION), Level::Urgent);
    }
}
