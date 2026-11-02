//! OpenClaw runtime reports. The `picket-openclaw` plugin writes one JSON frame per line
//! to a local socket; this module validates each frame and records reports as tool
//! results. A report is what the runtime says it did. The engine links it to an effect
//! only by an exact provider-resource match, and lost or unparsed reports degrade
//! coverage instead of being guessed.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    agent,
    event::{
        Action, AdapterProvenance, AgentMode, AgentRegistration, Coverage, CoverageReason,
        CoverageState, Event, EventKey, EventPayload, Source, SourceTimeQuality, ToolOutcome,
        ToolResult, normalize_email,
    },
    ledger::Ledger,
    store::{Batch, CollectorCursor, CoverageReport},
};

/// The largest frame the socket reads; a report has a bounded target list and ids only.
pub const MAX_FRAME: usize = 4096;
/// The plugin's heartbeat interval. Three missed heartbeats make reports stale.
pub const HEARTBEAT: Duration = Duration::seconds(60);
const STALE: Duration = Duration::seconds(180);
const MAX_TARGETS: usize = 50;
/// Plugin clocks are the Mac's clock; a time outside this window is not trusted.
const MAX_SKEW: Duration = Duration::minutes(5);
const MAX_AGE: Duration = Duration::days(1);
/// Adapters and versions whose report semantics this module knows.
const ADAPTERS: &[(&str, u16)] = &[("gog", 1)];

#[derive(Debug, Error, Eq, PartialEq)]
pub enum FrameError {
    #[error("frame exceeds the size limit")]
    TooLarge,
    #[error("frame token is invalid")]
    Token,
    #[error("frame is malformed")]
    Malformed,
    #[error("frame field is out of bounds")]
    Field,
    #[error("adapter version is not supported")]
    Adapter,
    #[error("agent is not a registered OpenClaw agent")]
    Agent,
    #[error("event store failed")]
    Store,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireFrame {
    v: u8,
    token: String,
    agent: String,
    instance: String,
    seq: u64,
    dropped: u64,
    unrecognized: u64,
    adapter: String,
    adapter_version: u16,
    at: DateTime<Utc>,
    #[serde(default)]
    report: Option<WireReport>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireReport {
    #[serde(default)]
    run: Option<String>,
    action: Action,
    #[serde(default)]
    account: Option<String>,
    targets: Vec<String>,
    #[serde(default)]
    resource: Option<String>,
    #[serde(default)]
    revision: Option<String>,
    outcome: ToolOutcome,
}

/// A validated frame. Without a report it is a heartbeat carrying the plugin's counters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    pub agent: String,
    /// One plugin process; counters and sequence restart with a new instance.
    pub instance: String,
    /// Reports this instance has numbered so far; a report carries its own number.
    pub seq: u64,
    /// Reports the plugin dropped because its queue was full or the socket failed.
    pub dropped: u64,
    /// `gog` invocations the adapter saw but could not parse, such as compound commands.
    pub unrecognized: u64,
    pub adapter: String,
    pub adapter_version: u16,
    pub at: DateTime<Utc>,
    pub report: Option<Report>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Report {
    pub run: Option<String>,
    pub action: Action,
    /// The Google account the command named, if it named one.
    pub account: Option<String>,
    pub targets: Vec<String>,
    pub resource: Option<String>,
    pub revision: Option<String>,
    pub outcome: ToolOutcome,
}

/// Validates one frame. The token is compared in constant time before any field is used.
pub fn parse(line: &[u8], token: &str) -> Result<Frame, FrameError> {
    if line.len() > MAX_FRAME {
        return Err(FrameError::TooLarge);
    }
    let wire: WireFrame = serde_json::from_slice(line).map_err(|_| FrameError::Malformed)?;
    if !token_matches(&wire.token, token) {
        return Err(FrameError::Token);
    }
    if wire.v != 1 {
        return Err(FrameError::Malformed);
    }
    if !ADAPTERS.contains(&(wire.adapter.as_str(), wire.adapter_version)) {
        return Err(FrameError::Adapter);
    }
    let instance_valid = (8..=64).contains(&wire.instance.len())
        && wire
            .instance
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    if !agent::valid_id(&wire.agent) || !instance_valid {
        return Err(FrameError::Field);
    }
    let report = wire.report.map(validate_report).transpose()?;
    if report.is_some() && wire.seq == 0 {
        return Err(FrameError::Field);
    }
    Ok(Frame {
        agent: wire.agent,
        instance: wire.instance,
        seq: wire.seq,
        dropped: wire.dropped,
        unrecognized: wire.unrecognized,
        adapter: wire.adapter,
        adapter_version: wire.adapter_version,
        at: wire.at,
        report,
    })
}

fn validate_report(wire: WireReport) -> Result<Report, FrameError> {
    let run_valid = wire.run.as_ref().is_none_or(|run| {
        (1..=128).contains(&run.len())
            && run
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
    });
    let resource_valid = wire.resource.as_ref().is_none_or(|resource| {
        (1..=256).contains(&resource.len())
            && resource
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    });
    let revision_valid = wire.revision.as_ref().is_none_or(|revision| {
        (1..=128).contains(&revision.len()) && revision.bytes().all(|byte| byte.is_ascii_graphic())
    });
    if !run_valid || !resource_valid || !revision_valid || wire.targets.len() > MAX_TARGETS {
        return Err(FrameError::Field);
    }
    let account = wire
        .account
        .map(|value| normalize_email(&value).ok_or(FrameError::Field))
        .transpose()?;
    let targets = wire
        .targets
        .iter()
        .map(|value| normalize_email(value).ok_or(FrameError::Field))
        .collect::<Result<_, _>>()?;
    Ok(Report {
        run: wire.run,
        action: wire.action,
        account,
        targets,
        resource: wire.resource,
        revision: wire.revision,
        outcome: wire.outcome,
    })
}

fn token_matches(given: &str, expected: &str) -> bool {
    let (given, expected) = (given.as_bytes(), expected.as_bytes());
    if given.len() != expected.len() || expected.is_empty() {
        return false;
    }
    given
        .iter()
        .zip(expected)
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

/// The Google account Picket monitors. Reports name accounts by email; events use its id.
#[derive(Clone, Copy, Debug)]
pub struct Monitored<'a> {
    pub id: &'a str,
    pub email: &'a str,
}

#[derive(Default, Serialize, Deserialize)]
struct Cursor {
    agents: BTreeMap<String, AgentState>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct AgentState {
    instance: String,
    seq: u64,
    dropped: u64,
    unrecognized: u64,
    /// Reports about a Google account Picket does not monitor; counted, not recorded.
    other_account: u64,
    /// When Picket last received a frame. Liveness stays in the cursor, not the log.
    seen: Option<DateTime<Utc>>,
}

/// Records one frame: a report becomes a tool result, and the plugin's sequence and
/// counters decide whether any report may have been lost or left unparsed.
pub fn ingest(
    ledger: &mut impl Ledger,
    account: Monitored<'_>,
    agents: &[AgentRegistration],
    frame: Frame,
    now: DateTime<Utc>,
) -> Result<(), FrameError> {
    if !agents
        .iter()
        .any(|item| item.id == frame.agent && item.enabled && item.mode == AgentMode::OpenClaw)
    {
        return Err(FrameError::Agent);
    }
    let mut cursor = load(ledger, account.id)?;
    let state = cursor.agents.entry(frame.agent.clone()).or_default();
    if state.instance != frame.instance {
        // A new plugin process numbers its reports and counts its losses from zero.
        *state = AgentState {
            instance: frame.instance.clone(),
            other_account: state.other_account,
            ..AgentState::default()
        };
    }
    let expected = if frame.report.is_some() {
        state.seq + 1
    } else {
        state.seq
    };
    let lost = frame.seq > expected || frame.dropped > state.dropped;
    let unparsed = frame.unrecognized > state.unrecognized;
    let at = if frame.at > now + MAX_SKEW || frame.at < now - MAX_AGE {
        None
    } else {
        Some(frame.at)
    };
    let mut events = Vec::new();
    if let Some(report) = &frame.report {
        let same_account = report
            .account
            .as_deref()
            .is_none_or(|email| Some(email) == normalize_email(account.email).as_deref());
        if same_account {
            events.push(tool_event(account.id, &frame, report, at, now));
        } else {
            state.other_account += 1;
        }
    }
    state.seq = state.seq.max(frame.seq);
    state.dropped = state.dropped.max(frame.dropped);
    state.unrecognized = state.unrecognized.max(frame.unrecognized);
    state.seen = Some(now);
    let (state, reason) = if lost {
        (CoverageState::Gap, Some(CoverageReason::ReportsLost))
    } else if unparsed {
        (CoverageState::Gap, Some(CoverageReason::UnsupportedFormat))
    } else {
        (CoverageState::Healthy, None)
    };
    let gap_since = (state == CoverageState::Gap).then(|| at.unwrap_or(now));
    let coverage = vec![report(account.id, &frame.agent, state, reason, gap_since)];
    commit(ledger, account.id, &cursor, events, coverage, now)
}

/// Marks registered OpenClaw agents whose plugin never connected, or has gone quiet.
pub fn check_stale(
    ledger: &mut impl Ledger,
    account: &str,
    agents: &[AgentRegistration],
    now: DateTime<Utc>,
) -> Result<(), FrameError> {
    let cursor = load(ledger, account)?;
    let coverage: Vec<_> = agents
        .iter()
        .filter(|item| item.enabled && item.mode == AgentMode::OpenClaw)
        .filter_map(
            |item| match cursor.agents.get(&item.id).and_then(|state| state.seen) {
                None => Some(report(
                    account,
                    &item.id,
                    CoverageState::Disabled,
                    Some(CoverageReason::NotConnected),
                    None,
                )),
                Some(seen) if now - seen > STALE => Some(report(
                    account,
                    &item.id,
                    CoverageState::Gap,
                    Some(CoverageReason::Interrupted),
                    Some(seen),
                )),
                Some(_) => None,
            },
        )
        .collect();
    if coverage.is_empty() {
        return Ok(());
    }
    ledger
        .commit(
            Batch {
                coverage,
                ..Batch::default()
            },
            now,
        )
        .map(|_| ())
        .map_err(|_| FrameError::Store)
}

/// The coverage capability for one OpenClaw agent's runtime reports.
pub fn report_capability(agent: &str) -> String {
    format!("reports_{agent}")
}

fn load(ledger: &impl Ledger, account: &str) -> Result<Cursor, FrameError> {
    match ledger
        .checkpoint(account, &Source::OpenClaw)
        .map_err(|_| FrameError::Store)?
    {
        Some(value) => serde_json::from_str(&value.value).map_err(|_| FrameError::Store),
        None => Ok(Cursor::default()),
    }
}

fn commit(
    ledger: &mut impl Ledger,
    account: &str,
    cursor: &Cursor,
    events: Vec<Event>,
    coverage: Vec<CoverageReport>,
    now: DateTime<Utc>,
) -> Result<(), FrameError> {
    let value = serde_json::to_string(cursor).map_err(|_| FrameError::Store)?;
    ledger
        .commit(
            Batch {
                events,
                cursor: Some(CollectorCursor {
                    account: account.into(),
                    source: Source::OpenClaw,
                    value,
                    updated_at: now,
                }),
                calendar: Vec::new(),
                coverage,
            },
            now,
        )
        .map(|_| ())
        .map_err(|_| FrameError::Store)
}

fn tool_event(
    account: &str,
    frame: &Frame,
    report: &Report,
    at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Event {
    Event {
        schema_version: 1,
        key: EventKey {
            account: account.into(),
            source: Source::OpenClaw,
            resource: format!("{}/{}/{}", frame.agent, frame.instance, frame.seq),
            revision: "0".into(),
        },
        occurred_at: at.unwrap_or(now),
        ingested_at: now,
        source_time_quality: Some(if at.is_some() {
            SourceTimeQuality::Provider
        } else {
            SourceTimeQuality::IngestedFallback
        }),
        adapter: Some(AdapterProvenance {
            name: format!("openclaw-{}", frame.adapter),
            version: frame.adapter_version,
            scope: "after_tool_call".into(),
        }),
        payload: EventPayload::ToolResult(ToolResult {
            agent: frame.agent.clone(),
            task: report.run.clone(),
            action: report.action.clone(),
            targets: report.targets.clone(),
            provider_resource: report.resource.clone(),
            provider_revision: report.revision.clone(),
            outcome: report.outcome.clone(),
        }),
    }
}

fn report(
    account: &str,
    agent: &str,
    state: CoverageState,
    reason: Option<CoverageReason>,
    gap_since: Option<DateTime<Utc>>,
) -> CoverageReport {
    CoverageReport {
        account: account.into(),
        source: Source::OpenClaw,
        coverage: Coverage {
            capability: report_capability(agent),
            state,
            cursor: None,
            gap_since,
            reason,
            partial_pages: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;
    use crate::store::Store;

    const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const ACCOUNT: Monitored<'static> = Monitored {
        id: "account-1",
        email: "Owner@Example.test",
    };

    fn at(minutes: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 2, 9, 0, 0)
            .single()
            .unwrap_or_else(|| panic!("fixed time"))
            + Duration::minutes(minutes)
    }

    fn agent() -> AgentRegistration {
        AgentRegistration {
            id: "openclaw".into(),
            name: "OpenClaw".into(),
            mode: AgentMode::OpenClaw,
            handles: Vec::new(),
            connected_since: None,
            enabled: true,
            region: None,
        }
    }

    fn wire(seq: u64, report: Option<serde_json::Value>) -> serde_json::Value {
        let mut frame = json!({
            "v": 1,
            "token": TOKEN,
            "agent": "openclaw",
            "instance": "instance-a",
            "seq": seq,
            "dropped": 0,
            "unrecognized": 0,
            "adapter": "gog",
            "adapter_version": 1,
            "at": at(0),
        });
        if let Some(report) = report {
            frame["report"] = report;
        }
        frame
    }

    fn send() -> serde_json::Value {
        json!({
            "run": "run-1",
            "action": "email_send",
            "targets": ["Sam@Example.test"],
            "resource": "18f0a1b2c3d4e5f6",
            "outcome": "succeeded",
        })
    }

    fn frame(value: &serde_json::Value) -> Result<Frame, FrameError> {
        parse(value.to_string().as_bytes(), TOKEN)
    }

    fn ingest_at(
        store: &mut Store,
        value: serde_json::Value,
        minutes: i64,
    ) -> Result<(), FrameError> {
        ingest(store, ACCOUNT, &[agent()], frame(&value)?, at(minutes))
    }

    fn tools(store: &Store) -> Vec<(EventKey, ToolResult)> {
        store
            .events()
            .unwrap_or_else(|error| panic!("{error}"))
            .into_iter()
            .filter_map(|row| match row.event.payload {
                EventPayload::ToolResult(tool) => Some((row.event.key, tool)),
                _ => None,
            })
            .collect()
    }

    fn coverage(store: &Store) -> Vec<Coverage> {
        store
            .events()
            .unwrap_or_else(|error| panic!("{error}"))
            .into_iter()
            .filter_map(|row| match row.event.payload {
                EventPayload::Coverage(value) if row.event.key.source == Source::OpenClaw => {
                    Some(value)
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn frames_need_the_token_bounded_fields_and_a_known_adapter() {
        assert!(frame(&wire(1, Some(send()))).is_ok());
        let mut wrong = wire(0, None);
        wrong["token"] = json!(TOKEN.replace('a', "b"));
        assert_eq!(frame(&wrong), Err(FrameError::Token));
        assert_eq!(
            parse(wire(0, None).to_string().as_bytes(), ""),
            Err(FrameError::Token)
        );
        let mut unknown = wire(0, None);
        unknown["body"] = json!("raw tool output");
        assert_eq!(
            frame(&unknown),
            Err(FrameError::Malformed),
            "no extra fields"
        );
        let mut adapter = wire(0, None);
        adapter["adapter_version"] = json!(2);
        assert_eq!(frame(&adapter), Err(FrameError::Adapter));
        let mut resource = send();
        resource["resource"] = json!("https://evil.example/x");
        assert_eq!(frame(&wire(1, Some(resource))), Err(FrameError::Field));
        let mut targets = send();
        targets["targets"] = json!(vec!["a@example.test"; MAX_TARGETS + 1]);
        assert_eq!(frame(&wire(1, Some(targets))), Err(FrameError::Field));
        let mut address = send();
        address["targets"] = json!(["not an address"]);
        assert_eq!(frame(&wire(1, Some(address))), Err(FrameError::Field));
        assert_eq!(
            frame(&wire(0, Some(send()))),
            Err(FrameError::Field),
            "reports start at 1"
        );
        assert_eq!(
            parse(&vec![b' '; MAX_FRAME + 1], TOKEN),
            Err(FrameError::TooLarge)
        );
    }

    #[test]
    fn the_plugins_contract_example_parses_and_records() -> Result<(), Box<dyn std::error::Error>> {
        let example = include_bytes!("../../../plugins/openclaw/contract/report-frame.json");
        let line: serde_json::Value = serde_json::from_slice(example)?;
        let parsed = parse(line.to_string().as_bytes(), TOKEN)?;
        assert_eq!(
            parsed.report.as_ref().map(|item| &item.action),
            Some(&Action::EmailSend)
        );
        let mut store = Store::memory()?;
        ingest(&mut store, ACCOUNT, &[agent()], parsed, at(1))?;
        assert_eq!(tools(&store).len(), 1);
        Ok(())
    }

    #[test]
    fn a_report_becomes_a_tool_result_with_normalized_targets()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut store = Store::memory()?;
        ingest_at(&mut store, wire(1, Some(send())), 0)?;
        let recorded = tools(&store);
        assert_eq!(recorded.len(), 1);
        let (key, tool) = &recorded[0];
        assert_eq!(key.account, "account-1");
        assert_eq!(key.resource, "openclaw/instance-a/1");
        assert_eq!(tool.task.as_deref(), Some("run-1"));
        assert_eq!(tool.targets, vec!["sam@example.test".to_owned()]);
        assert_eq!(tool.provider_resource.as_deref(), Some("18f0a1b2c3d4e5f6"));
        assert_eq!(tool.outcome, ToolOutcome::Succeeded);
        assert_eq!(
            coverage(&store).last().map(|item| &item.state),
            Some(&CoverageState::Healthy)
        );
        ingest_at(&mut store, wire(1, Some(send())), 1)?;
        assert_eq!(tools(&store).len(), 1, "a resent report is recorded once");
        Ok(())
    }

    #[test]
    fn an_ingested_report_links_the_observed_send_and_nothing_else()
    -> Result<(), Box<dyn std::error::Error>> {
        let sent = |resource: &str| Event {
            schema_version: 1,
            key: EventKey {
                account: "account-1".into(),
                source: Source::Gmail,
                resource: resource.into(),
                revision: "sent".into(),
            },
            occurred_at: at(0),
            ingested_at: at(0),
            source_time_quality: Some(SourceTimeQuality::Provider),
            adapter: None,
            payload: EventPayload::Effect(crate::event::Effect {
                action: Action::EmailSend,
                targets: vec!["sam@example.test".into()],
                targets_complete: true,
                calendar_start: None,
                origin: crate::event::Origin::Live,
                external_change: false,
                target_names: BTreeMap::new(),
                email: None,
                calendar: None,
            }),
        };
        let mut store = Store::memory()?;
        store.append(&sent("18f0a1b2c3d4e5f6"))?;
        store.append(&sent("18f0a1b2c3d4ffff"))?;
        ingest_at(&mut store, wire(1, Some(send())), 1)?;
        let events: Vec<_> = store.events()?.into_iter().map(|row| row.event).collect();
        let actors: Vec<_> = crate::engine::assess(&events, at(2))
            .into_iter()
            .map(|item| item.actor)
            .collect();
        assert_eq!(
            actors,
            vec![
                crate::engine::ActorEvidence::RuntimeLinked("openclaw".into()),
                crate::engine::ActorEvidence::Unknown,
            ]
        );
        Ok(())
    }

    #[test]
    fn sequence_gaps_and_drops_are_recorded_as_lost_reports()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut store = Store::memory()?;
        ingest_at(&mut store, wire(0, None), 0)?;
        ingest_at(&mut store, wire(2, Some(send())), 1)?;
        let states: Vec<_> = coverage(&store)
            .into_iter()
            .map(|item| item.reason)
            .collect();
        assert_eq!(states, vec![None, Some(CoverageReason::ReportsLost)]);
        ingest_at(&mut store, wire(2, None), 2)?;
        assert_eq!(
            coverage(&store).last().map(|item| &item.state),
            Some(&CoverageState::Healthy)
        );
        let mut dropped = wire(2, None);
        dropped["dropped"] = json!(1);
        ingest_at(&mut store, dropped, 3)?;
        assert_eq!(
            coverage(&store).last().and_then(|item| item.reason.clone()),
            Some(CoverageReason::ReportsLost)
        );
        let mut heartbeat = wire(3, None);
        heartbeat["dropped"] = json!(1);
        ingest_at(&mut store, heartbeat, 4)?;
        assert_eq!(
            coverage(&store).last().and_then(|item| item.reason.clone()),
            Some(CoverageReason::ReportsLost),
            "a heartbeat ahead of the last report means a report never arrived"
        );
        Ok(())
    }

    #[test]
    fn unparsed_commands_and_other_accounts_are_counted_not_recorded()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut store = Store::memory()?;
        let mut unparsed = wire(0, None);
        unparsed["unrecognized"] = json!(1);
        ingest_at(&mut store, unparsed, 0)?;
        assert_eq!(
            coverage(&store).last().and_then(|item| item.reason.clone()),
            Some(CoverageReason::UnsupportedFormat)
        );
        let mut other = send();
        other["account"] = json!("someone-else@example.test");
        let mut frame = wire(1, Some(other));
        frame["unrecognized"] = json!(1);
        ingest_at(&mut store, frame, 1)?;
        assert!(tools(&store).is_empty());
        let mut same = send();
        same["account"] = json!("owner@example.test");
        let mut frame = wire(2, Some(same));
        frame["unrecognized"] = json!(1);
        ingest_at(&mut store, frame, 2)?;
        assert_eq!(
            tools(&store).len(),
            1,
            "the monitored account matches case-insensitively"
        );
        Ok(())
    }

    #[test]
    fn a_new_plugin_instance_restarts_its_counters_without_a_false_loss()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut store = Store::memory()?;
        let mut first = wire(5, None);
        first["dropped"] = json!(2);
        ingest_at(&mut store, first, 0)?;
        let mut restarted = wire(1, Some(send()));
        restarted["instance"] = json!("instance-b");
        ingest_at(&mut store, restarted, 1)?;
        assert_eq!(
            coverage(&store).last().map(|item| &item.state),
            Some(&CoverageState::Healthy)
        );
        assert_eq!(tools(&store)[0].0.resource, "openclaw/instance-b/1");
        Ok(())
    }

    #[test]
    fn frames_for_unregistered_agents_are_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let mut store = Store::memory()?;
        let mut messages = agent();
        messages.mode = AgentMode::Messages;
        let result = ingest(
            &mut store,
            ACCOUNT,
            &[messages],
            frame(&wire(0, None))?,
            at(0),
        );
        assert_eq!(result, Err(FrameError::Agent));
        assert!(coverage(&store).is_empty());
        Ok(())
    }

    #[test]
    fn untrusted_times_fall_back_to_receipt_time() -> Result<(), Box<dyn std::error::Error>> {
        let mut store = Store::memory()?;
        let mut future = wire(1, Some(send()));
        future["at"] = json!(at(60));
        ingest_at(&mut store, future, 0)?;
        let row = store
            .events()?
            .into_iter()
            .find(|row| matches!(row.event.payload, EventPayload::ToolResult(_)))
            .ok_or("tool result")?;
        assert_eq!(row.event.occurred_at, at(0));
        assert_eq!(
            row.event.source_time_quality,
            Some(SourceTimeQuality::IngestedFallback)
        );
        Ok(())
    }

    #[test]
    fn quiet_or_never_connected_plugins_degrade_coverage() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut store = Store::memory()?;
        check_stale(&mut store, ACCOUNT.id, &[agent()], at(0))?;
        assert_eq!(
            coverage(&store).last().and_then(|item| item.reason.clone()),
            Some(CoverageReason::NotConnected)
        );
        ingest_at(&mut store, wire(0, None), 1)?;
        check_stale(&mut store, ACCOUNT.id, &[agent()], at(3))?;
        assert_eq!(
            coverage(&store).last().map(|item| &item.state),
            Some(&CoverageState::Healthy)
        );
        check_stale(&mut store, ACCOUNT.id, &[agent()], at(5))?;
        let last = coverage(&store).pop().ok_or("coverage")?;
        assert_eq!(last.reason, Some(CoverageReason::Interrupted));
        assert_eq!(last.gap_since, Some(at(1)));
        Ok(())
    }
}
