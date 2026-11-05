//! Alert delivery and the menu-bar status. One thread posts pending notifications from
//! the outbox, at least once, and keeps the tray in step with open incidents.
//! Notification actions are handled here in Rust, so they work with the window closed.

use std::{
    sync::{
        Mutex,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread,
    time::Duration,
};

use chrono::{DateTime, Local, Utc};
use picket_core::{
    alerts::{self, Post, Target},
    ledger::Writer,
    views::{IncidentSummary, Severity, Status},
};
use tauri::Manager;
use tauri_plugin_notification::{Action, ActionPerformed, ActionType, NotificationExt};

use crate::{GoogleState, tray};

const TICK: Duration = Duration::from_secs(5);
const ACTION_TYPE: &str = "picket-incident";
/// Pause holds delivery only; collection and assessment continue (plan decision 4).
const PAUSE: chrono::Duration = chrono::Duration::hours(1);
const MAX_ID_BYTES: usize = 512;

#[derive(Default)]
pub struct AlertState {
    paused_until: Mutex<Option<DateTime<Utc>>>,
    wake: Mutex<Option<Sender<()>>>,
}

impl AlertState {
    fn paused_until(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let mut paused = self.paused_until.lock().ok()?;
        if paused.is_some_and(|until| until <= now) {
            *paused = None;
        }
        *paused
    }

    /// Pauses alerts for an hour, or resumes them. Held alerts post on resume.
    pub fn set_paused(&self, paused: bool) {
        if let Ok(mut until) = self.paused_until.lock() {
            *until = paused.then(|| Utc::now() + PAUSE);
        }
        self.wake();
    }

    /// Asks the worker to deliver and refresh now instead of at its next tick.
    pub fn wake(&self) {
        if let Ok(sender) = self.wake.lock()
            && let Some(sender) = sender.as_ref()
        {
            let _ = sender.send(());
        }
    }
}

/// Registers the notification actions and starts the delivery thread.
pub fn start(app: &tauri::AppHandle) -> tauri::Result<()> {
    app.notification()
        .register_action_types(vec![
            ActionType::builder(ACTION_TYPE)
                .actions(vec![
                    Action::builder("review", "Review").build(),
                    Action::builder("fine", "This was fine").build(),
                ])
                .build(),
        ])
        .map_err(|error| tauri::Error::Anyhow(error.into()))?;
    let handle = app.clone();
    app.notification()
        .on_action(move |event| on_action(&handle, event))
        .map_err(|error| tauri::Error::Anyhow(error.into()))?;
    let (sender, receiver) = mpsc::channel();
    if let Ok(mut wake) = app.state::<AlertState>().wake.lock() {
        *wake = Some(sender);
    }
    let handle = app.clone();
    thread::Builder::new()
        .name("picket-alerts".into())
        .spawn(move || run(&handle, &receiver))?;
    Ok(())
}

fn run(app: &tauri::AppHandle, wake: &Receiver<()>) {
    let mut ledger: Option<Writer> = None;
    let mut shown: Option<(Key, tray::View)> = None;
    loop {
        if ledger.is_none() {
            ledger = crate::ledger(app).ok();
        }
        if let Some(writer) = &ledger {
            let now = Utc::now();
            let paused = app.state::<AlertState>().paused_until(now);
            if paused.is_none()
                && let Err(reason) = deliver(app, writer, now)
            {
                tracing::warn!(reason, "alert delivery failed");
            }
            if let Err(reason) = refresh(app, writer, now, paused, &mut shown) {
                tracing::warn!(reason, "menu bar status failed");
            }
        }
        match wake.recv_timeout(TICK) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Posts what the plan allows now and marks it delivered after posting. A crash in
/// between repeats the post on restart; macOS cannot replace a delivered notification.
fn deliver(
    app: &tauri::AppHandle,
    writer: &Writer,
    now: DateTime<Utc>,
) -> Result<(), &'static str> {
    let (pending, last_paced) = writer
        .read(|store| Ok((store.pending_alerts()?, store.last_paced_delivery()?)))
        .map_err(|_| "outbox unavailable")?;
    if !pending.unshowable.is_empty() {
        let ids = pending.unshowable.clone();
        writer
            .read(move |store| {
                for id in &ids {
                    store.cancel_notification(id)?;
                }
                Ok(())
            })
            .map_err(|_| "outbox unavailable")?;
    }
    for post in alerts::plan(&pending.alerts, last_paced, now, &Local) {
        show(app, &post)?;
        let Post { outbox, paced, .. } = post;
        writer
            .read(move |store| {
                for id in &outbox {
                    store.mark_notification_delivered(id, now, paced)?;
                }
                Ok(())
            })
            .map_err(|_| "outbox unavailable")?;
    }
    Ok(())
}

fn show(app: &tauri::AppHandle, post: &Post) -> Result<(), &'static str> {
    let target = match &post.target {
        Target::Incident(id) => id.as_str(),
        Target::Review => "review",
    };
    let mut builder = app
        .notification()
        .builder()
        .title(&post.title)
        .body(&post.body)
        .extra("target", target);
    if post.actions {
        builder = builder.action_type_id(ACTION_TYPE);
    }
    builder.show().map_err(|_| "notification failed")
}

/// What the tray shows depends only on these; while they hold, nothing is recomputed.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Key {
    changes: (i64, i64),
    paused: Option<DateTime<Utc>>,
    google: &'static str,
    day: chrono::NaiveDate,
}

fn refresh(
    app: &tauri::AppHandle,
    writer: &Writer,
    now: DateTime<Utc>,
    paused: Option<DateTime<Utc>>,
    shown: &mut Option<(Key, tray::View)>,
) -> Result<(), &'static str> {
    let google = app
        .state::<GoogleState>()
        .status
        .lock()
        .map_or("unavailable", |status| *status);
    let today = now.with_timezone(&Local).date_naive();
    let key = Key {
        changes: writer
            .read(|store| store.change_marker())
            .map_err(|_| "store unavailable")?,
        paused,
        google,
        day: today,
    };
    if shown.as_ref().is_some_and(|(last, _)| *last == key) {
        return Ok(());
    }
    let midnight = today
        .and_hms_opt(0, 0, 0)
        .and_then(|start| start.and_local_timezone(Local).earliest())
        .map_or(now, |start| start.with_timezone(&Utc));
    let (incidents, actions, agents) = writer
        .read(move |store| {
            let incidents = store.incidents()?;
            let actions = store.overview(midnight, now, now)?.counts.agent_actions;
            let agents = store.agents()?;
            Ok((incidents, actions, agents))
        })
        .map_err(|_| "store unavailable")?;
    // Named like the window does: the one enabled agent, if there is exactly one.
    let watched: Vec<_> = agents.iter().filter(|agent| agent.enabled).collect();
    let agent = match watched.as_slice() {
        [one] => Some(one.name.clone()),
        _ => None,
    };
    let view = view(
        &incidents,
        Today {
            actions,
            agent,
            any_agent: !watched.is_empty(),
        },
        paused,
        google,
    );
    if shown.as_ref().is_none_or(|(_, last)| *last != view) {
        tray::apply(app, &view).map_err(|_| "tray update failed")?;
    }
    *shown = Some((key, view));
    Ok(())
}

struct Today {
    actions: usize,
    agent: Option<String>,
    any_agent: bool,
}

fn view(
    incidents: &[IncidentSummary],
    today: Today,
    paused: Option<DateTime<Utc>>,
    google: &str,
) -> tray::View {
    let open: Vec<_> = incidents
        .iter()
        .filter(|incident| incident.status == Status::Open)
        .collect();
    let urgent = open
        .iter()
        .filter(|incident| incident.severity == Severity::Urgent)
        .count();
    let google_line = match google {
        "disconnected" => Some("Google isn’t connected"),
        "unavailable" => Some("Picket can’t reach Google"),
        _ => None,
    };
    // A pause the person chose greys the icon; otherwise open items outrank a lost
    // Google connection, which has its own line.
    let state = if paused.is_some() {
        tray::IconState::Muted
    } else if urgent > 0 {
        tray::IconState::Urgent
    } else if !open.is_empty() {
        tray::IconState::Review
    } else if google_line.is_some() {
        tray::IconState::Muted
    } else {
        tray::IconState::Quiet
    };
    let mut lines = vec![match (open.len(), urgent) {
        (0, _) => "Nothing needs a look".to_owned(),
        (1, 1) => "1 urgent item".to_owned(),
        (1, _) => "1 item to review".to_owned(),
        (count, 0) => format!("{count} items to review"),
        (count, urgent) => format!("{count} items to review, {urgent} urgent"),
    }];
    if let Some(until) = paused {
        lines.push(format!(
            "Alerts paused until {}",
            until.with_timezone(&Local).format("%-I:%M %p")
        ));
    }
    if let Some(line) = google_line {
        lines.push(line.to_owned());
    }
    if today.any_agent {
        let who = today.agent.unwrap_or_else(|| "Agents".into());
        let plural = if today.actions == 1 { "" } else { "s" };
        lines.push(format!("{who}: {} action{plural} today", today.actions));
    }
    tray::View {
        state,
        lines,
        // Incidents are listed newest first.
        open: open
            .iter()
            .take(3)
            .map(|incident| (incident.id.clone(), alerts::incident_title(incident)))
            .collect(),
        paused: paused.is_some(),
    }
}

fn on_action(app: &tauri::AppHandle, event: &ActionPerformed) {
    let Some(target) = event
        .notification()
        .and_then(|notification| notification.extra().get("target"))
        .and_then(serde_json::Value::as_str)
    else {
        return;
    };
    let result = match (event.action_id(), target) {
        ("tap" | "review", "review") => open(app, "#/review"),
        ("tap" | "review", id) => incident_id(id).and_then(|id| open_incident(app, id)),
        ("fine", id) => incident_id(id).and_then(|id| mark_fine(app, id)),
        _ => Ok(()),
    };
    if let Err(reason) = result {
        tracing::warn!(
            action = event.action_id(),
            reason,
            "notification action failed"
        );
    }
}

/// Opens the window on one incident.
pub fn open_incident(app: &tauri::AppHandle, id: &str) -> Result<(), &'static str> {
    let id = incident_id(id)?;
    open(app, &format!("#/incident/{}", &id["incident/".len()..]))
}

fn open(app: &tauri::AppHandle, hash: &str) -> Result<(), &'static str> {
    crate::show_window(app).map_err(|_| "window unavailable")?;
    let window = app.get_webview_window("main").ok_or("window unavailable")?;
    let literal = serde_json::to_string(hash).map_err(|_| "invalid route")?;
    window
        .eval(format!("window.location.hash = {literal}"))
        .map_err(|_| "window unavailable")
}

/// "This was fine" from a notification: the same decision as in the incident view.
fn mark_fine(app: &tauri::AppHandle, id: &str) -> Result<(), &'static str> {
    let id = id.to_owned();
    let found = crate::ledger(app)?
        .read(move |store| store.decide(&id, Some(true), Utc::now()))
        .map_err(|_| "decision could not be saved")?;
    app.state::<AlertState>().wake();
    if found {
        Ok(())
    } else {
        Err("incident not found")
    }
}

/// Incident ids come back from the notification server and the menu, so they are
/// checked as strictly as ids from the webview.
pub fn incident_id(id: &str) -> Result<&str, &'static str> {
    let hash = id.strip_prefix("incident/").ok_or("invalid incident id")?;
    let valid = !hash.is_empty()
        && id.len() <= MAX_ID_BYTES
        && hash.bytes().all(|byte| byte.is_ascii_hexdigit());
    if valid {
        Ok(id)
    } else {
        Err("invalid incident id")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use picket_core::views::Kind;

    fn incident(id: &str, severity: Severity, status: Status) -> IncidentSummary {
        IncidentSummary {
            id: format!("incident/{id}"),
            severity,
            status,
            kind: Kind::Email,
            reasons: vec![picket_core::views::Reason::ExtraTarget],
            agent: Some("Instinct".into()),
            occurred_at: DateTime::UNIX_EPOCH,
            flagged_at: DateTime::UNIX_EPOCH,
            subject: None,
            people: vec![],
        }
    }

    fn today(actions: usize) -> Today {
        Today {
            actions,
            agent: Some("Instinct".into()),
            any_agent: true,
        }
    }

    #[test]
    fn the_icon_follows_the_loudest_open_item_and_lists_three() {
        let incidents = [
            incident("a1", Severity::Review, Status::Open),
            incident("b2", Severity::Urgent, Status::Open),
            incident("c3", Severity::Urgent, Status::Fine),
            incident("d4", Severity::Review, Status::Open),
            incident("e5", Severity::Review, Status::Open),
        ];
        let view = view(&incidents, today(1), None, "monitoring");
        assert_eq!(view.state, tray::IconState::Urgent);
        assert_eq!(view.lines[0], "4 items to review, 1 urgent");
        assert_eq!(view.lines[1], "Instinct: 1 action today");
        assert_eq!(view.open.len(), 3);
        assert_eq!(view.open[0].0, "incident/a1");
        assert_eq!(view.open[1].0, "incident/b2");
    }

    #[test]
    fn a_pause_greys_the_icon_and_a_lost_connection_has_its_own_line() {
        let open = [incident("a1", Severity::Review, Status::Open)];
        let paused = Some(DateTime::UNIX_EPOCH);
        let view = view(&open, today(0), paused, "monitoring");
        assert_eq!(view.state, tray::IconState::Muted);
        assert!(view.paused);

        let view = view_quiet("disconnected");
        assert_eq!(view.state, tray::IconState::Muted);
        assert_eq!(view.lines[1], "Google isn’t connected");

        let view = super::view(&open, today(2), None, "disconnected");
        assert_eq!(view.state, tray::IconState::Review);
        assert_eq!(view.lines[2], "Instinct: 2 actions today");
        assert_eq!(view_quiet("monitoring").state, tray::IconState::Quiet);
    }

    fn view_quiet(google: &str) -> tray::View {
        view(&[], today(0), None, google)
    }

    #[test]
    fn incident_ids_are_checked() {
        assert!(incident_id("incident/0af9").is_ok());
        assert!(incident_id("incident/").is_err());
        assert!(incident_id("incident/'; alert(1)").is_err());
        assert!(incident_id("finding/0af9").is_err());
    }
}
