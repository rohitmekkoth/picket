use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use picket_core::{
    settings::{DEFAULT_RETENTION_DAYS, SettingsPatch},
    store::Store,
};

fn at(day: i64) -> DateTime<Utc> {
    Utc.timestamp_nanos((1_700_000_000 + day * 86_400) * 1_000_000_000)
}

#[test]
fn settings_persist_and_reject_invalid_values() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("settings.db");
    let mut store = Store::open(&path)?;
    let defaults = store.settings()?;
    assert_eq!(defaults.retention_days, DEFAULT_RETENTION_DAYS);
    assert_eq!(defaults.summary_minutes, Some(21 * 60));

    let patch: SettingsPatch =
        serde_json::from_str(r#"{"summaryMinutes":null,"retentionDays":90}"#)?;
    store.update_settings(&patch, at(0))?;
    store.set_alerts_paused_until(Some(at(1)))?;
    let day = NaiveDate::from_ymd_opt(2026, 11, 5).ok_or("date")?;
    store.set_last_summary_day(day)?;
    assert!(!store.settings()?.setup_completed);
    store.set_setup_completed(at(0))?;
    let invalid: SettingsPatch = serde_json::from_str(r#"{"retentionDays":2}"#)?;
    assert!(store.update_settings(&invalid, at(0)).is_err());
    drop(store);

    let reopened = Store::open(&path)?;
    let settings = reopened.settings()?;
    assert_eq!(settings.summary_minutes, None);
    assert_eq!(settings.retention_days, 90);
    assert_eq!(settings.alerts_paused_until, Some(at(1)));
    assert_eq!(settings.last_summary_day, Some(day));
    assert!(settings.setup_completed);
    Ok(())
}
