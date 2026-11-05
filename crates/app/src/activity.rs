//! Overview and Activity commands. The renderer picks a period in the user's local
//! time and sends it as instants; Rust bounds it and runs the query on the writer.

use chrono::{DateTime, Duration, Utc};
use picket_core::activity::{ActivityPage, Filter, OverviewView, Query, Who};

const MAX_SPAN_DAYS: i64 = 400;
const MAX_CURSOR_BYTES: usize = 1024;

fn period(from: &str, to: &str) -> Result<(DateTime<Utc>, DateTime<Utc>), String> {
    let parse = |value: &str| {
        DateTime::parse_from_rfc3339(value)
            .map(|time| time.with_timezone(&Utc))
            .map_err(|_| "invalid period".to_string())
    };
    let (from, to) = (parse(from)?, parse(to)?);
    if from >= to || to - from > Duration::days(MAX_SPAN_DAYS) {
        return Err("invalid period".into());
    }
    Ok((from, to))
}

#[tauri::command]
pub fn get_overview(
    app: tauri::AppHandle,
    from: String,
    to: String,
) -> Result<OverviewView, String> {
    let (from, to) = period(&from, &to)?;
    crate::ledger(&app)?
        .read(move |store| store.overview(from, to, Utc::now()))
        .map_err(|_| "overview unavailable".to_string())
}

#[tauri::command]
pub fn list_activity(
    app: tauri::AppHandle,
    from: String,
    to: String,
    who: String,
    result: String,
    before: Option<String>,
) -> Result<ActivityPage, String> {
    let (from, to) = period(&from, &to)?;
    let who = match who.as_str() {
        "everyone" => Who::Everyone,
        "agents" => Who::Agents,
        "not_linked" => Who::NotLinked,
        _ => return Err("invalid filter".into()),
    };
    let result = match result.as_str() {
        "any" => Filter::Any,
        "review" => Filter::Review,
        "as_asked" => Filter::AsAsked,
        "unclear" => Filter::Unclear,
        _ => return Err("invalid filter".into()),
    };
    if before
        .as_ref()
        .is_some_and(|cursor| cursor.len() > MAX_CURSOR_BYTES)
    {
        return Err("invalid page".into());
    }
    crate::ledger(&app)?
        .read(move |store| {
            store.activity(
                &Query {
                    from,
                    to,
                    who,
                    result,
                    before: before.as_deref(),
                    limit: 50,
                },
                Utc::now(),
            )
        })
        .map_err(|_| "activity unavailable".to_string())
}
