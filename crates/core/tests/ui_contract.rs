//! The desktop UI validates every IPC response against hand-written schemas. This test
//! records the serialized name of each enum variant those schemas must accept, in
//! `ui/src/lib/contract.json`, and fails when the file is out of date. The exhaustive
//! matches make a new variant a compile error here until it is listed.

use picket_core::{
    event::{Action, CoverageReason, CoverageState, Level, NoticeKind, Source},
    views::Reason,
};
use serde::Serialize;
use serde_json::{Value, json};

fn names<T: Serialize>(values: &[T]) -> Value {
    Value::Array(
        values
            .iter()
            .map(|value| serde_json::to_value(value).unwrap_or(Value::Null))
            .collect(),
    )
}

fn contract() -> Value {
    let actions = [
        Action::EmailSend,
        Action::CalendarCreate,
        Action::CalendarUpdate,
        Action::CalendarDelete,
        Action::CalendarRsvp,
    ];
    for action in &actions {
        match action {
            Action::EmailSend
            | Action::CalendarCreate
            | Action::CalendarUpdate
            | Action::CalendarDelete
            | Action::CalendarRsvp => {}
        }
    }
    let sources = [
        Source::Gmail,
        Source::Calendar,
        Source::Messages,
        Source::OpenClaw,
        Source::Picket,
    ];
    for source in &sources {
        match source {
            Source::Gmail
            | Source::Calendar
            | Source::Messages
            | Source::OpenClaw
            | Source::Picket => {}
        }
    }
    let reasons = [
        CoverageReason::CursorExpired,
        CoverageReason::PageLimit,
        CoverageReason::MissingResource,
        CoverageReason::RequestFailed,
        CoverageReason::AuthUnavailable,
        CoverageReason::Interrupted,
        CoverageReason::PermissionMissing,
        CoverageReason::UnsupportedFormat,
        CoverageReason::Undecodable,
        CoverageReason::ExtractionUnavailable,
    ];
    for reason in &reasons {
        match reason {
            CoverageReason::CursorExpired
            | CoverageReason::PageLimit
            | CoverageReason::MissingResource
            | CoverageReason::RequestFailed
            | CoverageReason::AuthUnavailable
            | CoverageReason::Interrupted
            | CoverageReason::PermissionMissing
            | CoverageReason::UnsupportedFormat
            | CoverageReason::Undecodable
            | CoverageReason::ExtractionUnavailable => {}
        }
    }
    let states = [
        CoverageState::Healthy,
        CoverageState::Delayed,
        CoverageState::Gap,
        CoverageState::Disabled,
    ];
    for state in &states {
        match state {
            CoverageState::Healthy
            | CoverageState::Delayed
            | CoverageState::Gap
            | CoverageState::Disabled => {}
        }
    }
    let levels = [Level::Urgent, Level::Review, Level::Summary, Level::Off];
    for level in &levels {
        match level {
            Level::Urgent | Level::Review | Level::Summary | Level::Off => {}
        }
    }
    let notices = [
        NoticeKind::Forwarding,
        NoticeKind::AppAccess,
        NoticeKind::Purchase,
        NoticeKind::Otp,
    ];
    for notice in &notices {
        match notice {
            NoticeKind::Forwarding
            | NoticeKind::AppAccess
            | NoticeKind::Purchase
            | NoticeKind::Otp => {}
        }
    }
    let incident_reasons = [
        Reason::AfterRevocation,
        Reason::Boundary,
        Reason::ProhibitedAction,
        Reason::ApprovalMissing,
        Reason::ExtraTarget,
        Reason::DifferentCalendarTime,
        Reason::TooManyActions,
        Reason::DifferentAction,
        Reason::OutsideWindow,
        Reason::ReportNotObserved,
        Reason::ReportContradicted,
        Reason::Forwarding,
        Reason::AppAccess,
        Reason::Otp,
        Reason::Purchase,
        Reason::SignInNeeded,
        Reason::SourceGap,
        Reason::SourceDelayed,
    ];
    for reason in &incident_reasons {
        match reason {
            Reason::AfterRevocation
            | Reason::Boundary
            | Reason::ProhibitedAction
            | Reason::ApprovalMissing
            | Reason::ExtraTarget
            | Reason::DifferentCalendarTime
            | Reason::TooManyActions
            | Reason::DifferentAction
            | Reason::OutsideWindow
            | Reason::ReportNotObserved
            | Reason::ReportContradicted
            | Reason::Forwarding
            | Reason::AppAccess
            | Reason::Otp
            | Reason::Purchase
            | Reason::SignInNeeded
            | Reason::SourceGap
            | Reason::SourceDelayed => {}
        }
    }
    json!({
        "action": names(&actions),
        "source": names(&sources),
        "coverageReason": names(&reasons),
        "coverageState": names(&states),
        "level": names(&levels),
        "notice": names(&notices),
        "reason": names(&incident_reasons),
    })
}

#[test]
fn the_ui_contract_lists_every_serialized_variant() -> Result<(), Box<dyn std::error::Error>> {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/lib/contract.json");
    let expected = format!("{}\n", serde_json::to_string_pretty(&contract())?);
    if std::env::var_os("PICKET_UPDATE_CONTRACT").is_some() {
        std::fs::write(&path, &expected)?;
    }
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        current, expected,
        "ui/src/lib/contract.json is out of date; rerun with PICKET_UPDATE_CONTRACT=1"
    );
    Ok(())
}
