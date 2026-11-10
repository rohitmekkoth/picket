//! The person's preferences. They are not evidence, so they live in one validated row
//! beside the log rather than in it (plan decision 5), and a wipe removes them with it.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

pub const DEFAULT_SUMMARY_MINUTES: u16 = 21 * 60;
pub const DEFAULT_RETENTION_DAYS: u16 = 180;
/// The retention choices the Settings screen offers.
pub const RETENTION_CHOICES: [u16; 4] = [30, 90, 180, 365];

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Minutes after local midnight for the daily summary; `None` turns it off.
    pub summary_minutes: Option<u16>,
    pub retention_days: u16,
    pub alerts_paused_until: Option<DateTime<Utc>>,
    /// The local day the last daily summary covered.
    pub last_summary_day: Option<NaiveDate>,
    /// The setup wizard was finished or closed; it does not open by itself again.
    pub setup_completed: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            summary_minutes: Some(DEFAULT_SUMMARY_MINUTES),
            retention_days: DEFAULT_RETENTION_DAYS,
            alerts_paused_until: None,
            last_summary_day: None,
            setup_completed: false,
        }
    }
}

/// A change from the Settings screen. Absent fields stay as they are.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatch {
    /// `Some(None)` turns the summary off.
    #[serde(default, with = "double_option")]
    pub summary_minutes: Option<Option<u16>>,
    pub retention_days: Option<u16>,
}

impl Settings {
    /// Applies a patch, or says which field is invalid.
    pub fn apply(&self, patch: &SettingsPatch) -> Result<Self, &'static str> {
        let mut next = self.clone();
        if let Some(minutes) = patch.summary_minutes {
            if minutes.is_some_and(|value| value >= 24 * 60) {
                return Err("invalid summary time");
            }
            next.summary_minutes = minutes;
        }
        if let Some(days) = patch.retention_days {
            if !RETENTION_CHOICES.contains(&days) {
                return Err("invalid retention");
            }
            next.retention_days = days;
        }
        Ok(next)
    }
}

/// Tells a missing field from an explicit `null`.
mod double_option {
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        Option::<T>::deserialize(deserializer).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(json: &str) -> SettingsPatch {
        serde_json::from_str(json).unwrap_or_default()
    }

    #[test]
    fn patches_change_only_what_they_name() {
        let current = Settings::default();
        let next = current.apply(&patch(r#"{"retentionDays":90}"#));
        assert_eq!(next.as_ref().map(|value| value.retention_days), Ok(90));
        assert_eq!(
            next.map(|value| value.summary_minutes),
            Ok(Some(DEFAULT_SUMMARY_MINUTES))
        );
        let off = current.apply(&patch(r#"{"summaryMinutes":null}"#));
        assert_eq!(off.map(|value| value.summary_minutes), Ok(None));
    }

    #[test]
    fn invalid_values_are_rejected() {
        let current = Settings::default();
        assert!(current.apply(&patch(r#"{"summaryMinutes":1440}"#)).is_err());
        assert!(current.apply(&patch(r#"{"retentionDays":7}"#)).is_err());
        assert!(serde_json::from_str::<SettingsPatch>(r#"{"other":1}"#).is_err());
    }
}
