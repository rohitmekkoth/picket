//! Notification planning: which pending outbox items to post now, how they group, and
//! their wording. The desktop app posts the result; this module stays portable so the
//! rate limits and copy are tested without a notification server.

use std::{collections::BTreeMap, fmt::Display};

use chrono::{DateTime, Duration, TimeZone, Utc};

use crate::views::{IncidentSummary, Kind, Person, Reason, Status};

/// Review-level notifications post at most this often, grouped (design: notification behavior).
pub const REVIEW_INTERVAL: Duration = Duration::minutes(10);
/// More urgent alerts than this at once, such as after a long sleep, post as one group.
pub const MAX_SEPARATE: usize = 3;

/// One outbox item ready to post, with the incident it belongs to.
#[derive(Clone, Debug)]
pub struct PendingAlert {
    pub outbox: String,
    pub incident: IncidentSummary,
    /// The queued revision routes to urgent review.
    pub urgent: bool,
    pub correction: bool,
}

/// Where a click on a notification goes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Target {
    Incident(String),
    Review,
}

/// One notification to post, and the outbox items it delivers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Post {
    pub title: String,
    pub body: String,
    pub target: Target,
    /// Offer Review and This was fine; only for one incident that is still open.
    pub actions: bool,
    /// Counted against the review interval.
    pub paced: bool,
    pub outbox: Vec<String>,
}

/// Plans the notifications to post now. Urgent items post at once; review items and
/// corrections post when the review interval has passed since the last paced post.
/// Items that wait stay pending, so nothing is dropped.
pub fn plan<Tz: TimeZone<Offset: Display>>(
    pending: &[PendingAlert],
    last_paced: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Vec<Post> {
    // One incident can have several findings queued; it posts once, as loud as its loudest.
    let mut incidents: BTreeMap<&str, Merged<'_>> = BTreeMap::new();
    for item in pending {
        let merged = incidents
            .entry(item.incident.id.as_str())
            .or_insert_with(|| Merged {
                incident: &item.incident,
                urgent: false,
                correction: true,
                outbox: Vec::new(),
            });
        merged.urgent |= item.urgent && item.incident.status == Status::Open;
        merged.correction &= item.correction;
        merged.outbox.push(item.outbox.clone());
    }
    let (urgent, paced): (Vec<_>, Vec<_>) =
        incidents.into_values().partition(|merged| merged.urgent);
    let mut posts = Vec::new();
    if urgent.len() > MAX_SEPARATE {
        posts.push(group(&urgent, false, "need a look now"));
    } else {
        posts.extend(urgent.iter().map(|merged| single(merged, false, now, zone)));
    }
    let due = last_paced.is_none_or(|at| now - at >= REVIEW_INTERVAL);
    if due {
        match paced.as_slice() {
            [] => {}
            [one] => posts.push(single(one, true, now, zone)),
            many => posts.push(group(many, true, "to review")),
        }
    }
    posts
}

struct Merged<'a> {
    incident: &'a IncidentSummary,
    urgent: bool,
    correction: bool,
    outbox: Vec<String>,
}

fn single<Tz: TimeZone<Offset: Display>>(
    merged: &Merged<'_>,
    paced: bool,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Post {
    let incident = merged.incident;
    let open = incident.status == Status::Open;
    let (title, body) = if merged.correction && !open {
        (
            "An earlier alert no longer needs a look".to_owned(),
            incident_title(incident),
        )
    } else if merged.correction {
        (
            format!("Updated: {}", incident_title(incident)),
            body(incident, now, zone),
        )
    } else {
        (incident_title(incident), body(incident, now, zone))
    };
    Post {
        title,
        body,
        target: Target::Incident(incident.id.clone()),
        actions: open,
        paced,
        outbox: merged.outbox.clone(),
    }
}

fn group(items: &[Merged<'_>], paced: bool, what: &str) -> Post {
    let first = items
        .first()
        .map(|merged| incident_title(merged.incident))
        .unwrap_or_default();
    let others = items.len().saturating_sub(1);
    Post {
        title: format!("{} items {what}", items.len()),
        body: format!("{first}, and {others} more"),
        target: Target::Review,
        actions: false,
        paced,
        outbox: items
            .iter()
            .flat_map(|merged| merged.outbox.iter().cloned())
            .collect(),
    }
}

/// The headline for an incident. Mirrors `incidentTitle` in `ui/src/lib/copy.ts`; the
/// UI contract test checks both give the same words.
pub fn incident_title(summary: &IncidentSummary) -> String {
    let agent = summary.agent.as_deref().unwrap_or("Someone");
    let calendar = summary.kind == Kind::Calendar;
    let Some(reason) = summary.reasons.first() else {
        return "Something needs a look".into();
    };
    match reason {
        Reason::AfterRevocation => format!("{agent} acted after you marked it revoked"),
        Reason::Boundary if calendar => "A calendar change crossed one of your boundaries".into(),
        Reason::Boundary => "An email crossed one of your boundaries".into(),
        Reason::ProhibitedAction => format!("{agent} did something you told it not to"),
        Reason::ApprovalMissing => format!("{agent} went ahead without your OK"),
        Reason::ExtraTarget => format!(
            "{agent} {} someone you didn’t mention",
            if calendar { "invited" } else { "emailed" }
        ),
        Reason::DifferentCalendarTime => format!("{agent} booked a different time than you asked"),
        Reason::TooManyActions => format!("{agent} did it more times than you asked"),
        Reason::DifferentAction => format!("{agent} did a different kind of thing than you asked"),
        Reason::OutsideWindow => format!("{agent} acted outside the time you gave"),
        Reason::ReportNotObserved => format!("{agent} said it did something Picket didn’t see"),
        Reason::ReportContradicted => {
            format!("{agent} described it differently than it happened")
        }
        Reason::Forwarding => "Mail forwarding may have been turned on".into(),
        Reason::AppAccess => "A new app may have access to your Google account".into(),
        Reason::Otp => "A sign-in or verification code arrived".into(),
        Reason::Purchase => "A purchase or order confirmation arrived".into(),
        Reason::SignInNeeded => "Picket needs you to sign in to Google again".into(),
        Reason::SourceGap => "Picket couldn’t see everything for a while".into(),
        Reason::SourceDelayed => "Picket is catching up".into(),
    }
}

/// Who and what, then when: "To Sam Lee · “Plan” · 2:41 PM". Bounded by the
/// collectors' own limits on subjects and recipients.
fn body<Tz: TimeZone<Offset: Display>>(
    summary: &IncidentSummary,
    now: DateTime<Utc>,
    zone: &Tz,
) -> String {
    let mut parts = Vec::new();
    let names = people(&summary.people);
    match summary.kind {
        Kind::Email | Kind::Calendar => {
            if !names.is_empty() {
                parts.push(if summary.kind == Kind::Email {
                    format!("To {names}")
                } else {
                    format!("With {names}")
                });
            }
            if let Some(subject) = &summary.subject {
                parts.push(format!("“{subject}”"));
            }
        }
        Kind::Report | Kind::AccountWarning | Kind::Visibility => {}
    }
    parts.push(when(summary.occurred_at, now, zone));
    parts.join(" · ")
}

/// "2:41 PM" today, otherwise "Oct 28, 2:41 PM", in the Mac's time zone.
fn when<Tz: TimeZone<Offset: Display>>(at: DateTime<Utc>, now: DateTime<Utc>, zone: &Tz) -> String {
    let local = at.with_timezone(zone);
    if local.date_naive() == now.with_timezone(zone).date_naive() {
        local.format("%-I:%M %p").to_string()
    } else {
        local.format("%b %-d, %-I:%M %p").to_string()
    }
}

/// "Sam Lee and Jordan Park", "Sam, Jo and 2 others". Mirrors `people` in copy.ts.
fn people(list: &[Person]) -> String {
    let names: Vec<&str> = list
        .iter()
        .map(|person| person.name.as_deref().unwrap_or(&person.address))
        .filter(|name| !name.is_empty())
        .collect();
    match names.as_slice() {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [first, second] => format!("{first} and {second}"),
        [first, second, rest @ ..] => format!(
            "{first}, {second} and {} other{}",
            rest.len(),
            if rest.len() == 1 { "" } else { "s" }
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::Severity;

    fn at(minute: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000 + minute * 60, 0).unwrap_or_default()
    }

    fn incident(id: &str, reason: Reason) -> IncidentSummary {
        IncidentSummary {
            id: format!("incident/{id}"),
            severity: Severity::Review,
            status: Status::Open,
            kind: Kind::Email,
            reasons: vec![reason],
            agent: Some("Instinct".into()),
            occurred_at: at(0),
            flagged_at: at(0),
            subject: Some("Synthetic plan".into()),
            people: vec![Person {
                address: "sam@example.test".into(),
                name: Some("Sam Lee".into()),
                unrequested: true,
            }],
        }
    }

    fn alert(id: &str, urgent: bool) -> PendingAlert {
        PendingAlert {
            outbox: format!("finding/{id}/1"),
            incident: incident(id, Reason::ExtraTarget),
            urgent,
            correction: false,
        }
    }

    #[test]
    fn urgent_alerts_post_at_once_and_review_alerts_wait_for_the_interval() {
        let pending = [alert("a", true), alert("b", false)];
        let posts = plan(&pending, Some(at(5)), at(9), &Utc);
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].target, Target::Incident("incident/a".into()));
        assert!(!posts[0].paced && posts[0].actions);

        let posts = plan(&pending[1..], Some(at(5)), at(15), &Utc);
        assert_eq!(posts.len(), 1);
        assert!(posts[0].paced);
        assert_eq!(
            posts[0].title,
            "Instinct emailed someone you didn’t mention"
        );
        assert_eq!(posts[0].body, "To Sam Lee · “Synthetic plan” · 8:00 AM");
    }

    #[test]
    fn several_review_alerts_post_as_one_group_that_opens_the_review_list() {
        let pending = [alert("a", false), alert("b", false), alert("c", false)];
        let posts = plan(&pending, None, at(0), &Utc);
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].title, "3 items to review");
        assert_eq!(
            posts[0].body,
            "Instinct emailed someone you didn’t mention, and 2 more"
        );
        assert_eq!(posts[0].target, Target::Review);
        assert!(!posts[0].actions);
        assert_eq!(posts[0].outbox.len(), 3);
    }

    #[test]
    fn an_urgent_backlog_posts_as_one_group() {
        let pending: Vec<_> = ["a", "b", "c", "d"]
            .iter()
            .map(|id| alert(id, true))
            .collect();
        let posts = plan(&pending, Some(at(0)), at(1), &Utc);
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].title, "4 items need a look now");
        assert!(!posts[0].paced);
    }

    #[test]
    fn findings_of_one_incident_post_once() {
        let mut second = alert("a", true);
        second.outbox = "finding/a-claim/1".into();
        let posts = plan(&[alert("a", false), second], Some(at(0)), at(1), &Utc);
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].outbox.len(), 2);
        assert!(!posts[0].paced);
    }

    #[test]
    fn a_correction_says_what_changed() {
        let mut cleared = alert("a", false);
        cleared.correction = true;
        cleared.incident.status = Status::Fine;
        let posts = plan(&[cleared], None, at(0), &Utc);
        assert_eq!(posts[0].title, "An earlier alert no longer needs a look");
        assert_eq!(posts[0].body, "Instinct emailed someone you didn’t mention");
        assert!(!posts[0].actions);

        let mut raised = alert("b", true);
        raised.correction = true;
        let posts = plan(&[raised], None, at(0), &Utc);
        assert_eq!(
            posts[0].title,
            "Updated: Instinct emailed someone you didn’t mention"
        );
        assert!(posts[0].actions);
    }

    #[test]
    fn earlier_days_show_the_date() {
        let mut item = alert("a", true);
        item.incident.people.clear();
        item.incident.subject = None;
        let posts = plan(&[item], None, at(60 * 24 * 2), &Utc);
        assert_eq!(posts[0].body, "Jan 15, 8:00 AM");
    }

    #[test]
    fn people_lists_stay_short() {
        let person = |name: &str| Person {
            address: format!("{name}@example.test"),
            name: Some(name.into()),
            unrequested: false,
        };
        let list: Vec<_> = ["Sam", "Jo", "Ari", "Kim"].map(person).into();
        assert_eq!(people(&list[..2]), "Sam and Jo");
        assert_eq!(people(&list[..3]), "Sam, Jo and 1 other");
        assert_eq!(people(&list), "Sam, Jo and 2 others");
    }
}
