import type {
  Action,
  ActivityItem,
  BoundaryKind,
  IncidentSummary,
  Level,
  Person,
  Reason,
} from "./api";
import { minutesToTime } from "./format";

// All user-facing wording lives here, so screens describe the same thing the same way.

export function agentName(summary: { agent: string | null }): string {
  return summary.agent ?? "An agent";
}

/** The headline for an incident, from its most specific reason. */
export function incidentTitle(summary: IncidentSummary): string {
  const agent = summary.agent ?? "Someone";
  const reason = summary.reasons[0];
  switch (reason) {
    case "after_revocation":
      return `${agent} acted after you marked it revoked`;
    case "boundary":
      return summary.kind === "calendar"
        ? "A calendar change crossed one of your boundaries"
        : "An email crossed one of your boundaries";
    case "prohibited_action":
      return `${agent} did something you told it not to`;
    case "approval_missing":
      return `${agent} went ahead without your OK`;
    case "extra_target":
      return `${agent} ${summary.kind === "calendar" ? "invited" : "emailed"} someone you didn’t mention`;
    case "different_calendar_time":
      return `${agent} booked a different time than you asked`;
    case "too_many_actions":
      return `${agent} did it more times than you asked`;
    case "different_action":
      return `${agent} did a different kind of thing than you asked`;
    case "outside_window":
      return `${agent} acted outside the time you gave`;
    case "report_not_observed":
      return `${agent} said it did something Picket didn’t see`;
    case "report_contradicted":
      return `${agent} described it differently than it happened`;
    case "forwarding":
      return "Mail forwarding may have been turned on";
    case "app_access":
      return "A new app may have access to your Google account";
    case "otp":
      return "A sign-in or verification code arrived";
    case "purchase":
      return "A purchase or order confirmation arrived";
    case "sign_in_needed":
      return "Picket needs you to sign in to Google again";
    case "source_gap":
      return "Picket couldn’t see everything for a while";
    case "source_delayed":
      return "Picket is catching up";
  }
  return "Something needs a look";
}

export const reasonExplanation: Record<Reason, string> = {
  after_revocation: "You marked this agent revoked before it acted.",
  boundary: "It crossed a boundary you set.",
  prohibited_action: "Your request said not to do this.",
  approval_missing: "You asked to approve it first, and you hadn’t.",
  extra_target: "Someone received it who wasn’t named in your request.",
  different_calendar_time: "The time differs from the one you asked for.",
  too_many_actions: "It happened more times than you asked for.",
  different_action: "It was a different kind of action from the one you asked for.",
  outside_window: "It happened outside the time you gave.",
  report_not_observed: "The agent reported it, but nothing matching appeared in your account.",
  report_contradicted: "What the agent reported doesn’t match what Picket saw.",
  forwarding: "An email that looks like a forwarding notice arrived.",
  app_access: "An email that looks like an app access notice arrived.",
  otp: "A verification code arrived.",
  purchase: "An order or receipt email arrived.",
  sign_in_needed: "Google stopped accepting Picket’s sign-in.",
  source_gap: "Some activity may be missing for this period.",
  source_delayed: "Picket fell behind and is checking what happened meanwhile.",
};

export function actionPhrase(action: Action | null): string {
  switch (action) {
    case "email_send":
      return "Sent an email";
    case "calendar_create":
      return "Created an event";
    case "calendar_update":
      return "Changed an event";
    case "calendar_delete":
      return "Deleted an event";
    case "calendar_rsvp":
      return "Replied to an invite";
    case null:
      return "Did something";
  }
}

export function requestedPhrase(action: Action): string {
  switch (action) {
    case "email_send":
      return "Send an email";
    case "calendar_create":
      return "Create an event";
    case "calendar_update":
      return "Change an event";
    case "calendar_delete":
      return "Delete an event";
    case "calendar_rsvp":
      return "Reply to an invite";
  }
}

export function personName(person: Person): string {
  return person.name ?? person.address;
}

/** A short list of names: "Sam Lee and Jordan Park", "Sam, Jo and 2 others". */
export function people(list: Person[]): string {
  const names = list.map(personName).filter((name) => name.length > 0);
  if (names.length === 0) return "";
  const [first = "", second = ""] = names;
  if (names.length === 1) return first;
  if (names.length === 2) return `${first} and ${second}`;
  return `${first}, ${second} and ${names.length - 2} other${names.length === 3 ? "" : "s"}`;
}

export function quoted(text: string | null): string {
  return text ? `“${text}”` : "";
}

/** One line describing an activity item, for lists. */
export function activityLine(item: ActivityItem): string {
  const to = people(item.people);
  switch (item.kind) {
    case "email":
      if (item.subject) return `Sent ${quoted(item.subject)}${to ? ` to ${to}` : ""}`;
      return to ? `Sent an email to ${to}` : "Sent an email";
    case "calendar": {
      const what = item.subject ? quoted(item.subject) : "an event";
      if (item.action === "calendar_delete") return `Deleted ${what}`;
      if (item.action === "calendar_update") return `Changed ${what}`;
      if (item.action === "calendar_rsvp") return `Replied to ${what}`;
      return `Created ${what}`;
    }
    case "request":
      return item.message
        ? `You asked ${item.agent ?? "your agent"}: ${quoted(item.message)}`
        : `You asked ${item.agent ?? "your agent"} to do something`;
    case "report":
      return item.message
        ? `${item.agent ?? "Your agent"} said: ${quoted(item.message)}`
        : `${item.agent ?? "Your agent"} reported doing something`;
    case "gap":
      return `Picket lost sight of ${sourceName(item.source)}`;
    case "recovered":
      return `Picket can see ${sourceName(item.source)} again`;
  }
}

export function sourceName(source: string | null): string {
  switch (source) {
    case "gmail":
      return "Gmail";
    case "calendar":
      return "Calendar";
    case "messages":
      return "your chat";
    default:
      return "a source";
  }
}

export const verdictLabel: Record<ActivityItem["verdict"], string> = {
  as_asked: "As asked",
  review: "Review",
  unclear: "Couldn’t tell",
  not_linked: "",
  none: "",
};

export const levelLabel: Record<Level, string> = {
  urgent: "Urgent",
  review: "Review",
  summary: "Summary",
  off: "Off",
};

/** Short descriptions for the Rules legend. */
export const levelShort: Record<Level, string> = {
  urgent: "Notifies you right away",
  review: "Notifies you, grouped",
  summary: "Daily summary only",
  off: "Recorded, never flagged",
};

export const levelHelp: Record<Level, string> = {
  urgent: "Notifies you right away",
  review: "Notifies you, at most every 10 minutes",
  summary: "Only in your daily summary",
  off: "Recorded, never flagged",
};

export type RuleCopy = { id: string; name: string; example: string };

export const ruleGroups: {
  title: string;
  note: string;
  groups: { label?: string; rules: RuleCopy[] }[];
}[] = [
  {
    title: "When your agent doesn’t do what you asked",
    note: "Picket compares what you ask in iMessage with what then happens in Gmail and Calendar. These apply only when Picket can tell your agent did it.",
    groups: [
      {
        label: "What it did",
        rules: [
          {
            id: "request.prohibited_action",
            name: "Did something you told it not to",
            example: "You said “don’t send it yet”, and it sent it.",
          },
          {
            id: "request.approval_missing",
            name: "Went ahead without your OK",
            example: "You asked to see a draft first, and it sent the email.",
          },
          {
            id: "request.extra_target",
            name: "Contacted someone you didn’t mention",
            example: "You asked it to email Sam, and it also emailed Jordan.",
          },
          {
            id: "request.different_time",
            name: "Picked a different time",
            example: "You asked for 2:00 PM, and it booked 3:00 PM.",
          },
          {
            id: "request.too_many",
            name: "Did it more times than you asked",
            example: "You asked for one invite, and it sent three.",
          },
          {
            id: "request.different_action",
            name: "Did a different kind of thing",
            example: "You asked it to move a meeting, and it deleted it.",
          },
          {
            id: "request.outside_window",
            name: "Acted outside the time you gave",
            example: "You said “tomorrow morning”, and it sent tonight.",
          },
          {
            id: "request.after_revocation",
            name: "Acted after you marked it revoked",
            example: "You marked it revoked in Picket, and it sent an email afterwards.",
          },
        ],
      },
      {
        label: "What it said",
        rules: [
          {
            id: "report.not_observed",
            name: "Said it did something Picket didn’t see",
            example: "It said “Sent”, but no email appeared in Gmail.",
          },
          {
            id: "report.contradicted",
            name: "Described it differently than it happened",
            example: "It said “invited Sam”, but the invite went to Jordan.",
          },
        ],
      },
    ],
  },
  {
    title: "Account warnings",
    note: "Picket spots these from emails sent by Google and stores. It can’t confirm the change happened, so check before you act.",
    groups: [
      {
        rules: [
          {
            id: "warning.forwarding",
            name: "Mail forwarding turned on",
            example: "Someone could be copying your email elsewhere.",
          },
          {
            id: "warning.app_access",
            name: "A new app got access to your Google account",
            example: "Google emails you when an app is granted access.",
          },
          {
            id: "warning.otp",
            name: "Sign-in or verification codes",
            example: "A code arrived that you may not have asked for.",
          },
          {
            id: "warning.purchase",
            name: "Purchases and order confirmations",
            example: "A receipt or order email arrived.",
          },
        ],
      },
    ],
  },
  {
    title: "When Picket can’t see everything",
    note: "While Picket can’t see a source, it can’t warn you about what happens there.",
    groups: [
      {
        rules: [
          {
            id: "visibility.google",
            name: "Lost access to Gmail or Calendar",
            example: "For example, your Google sign-in expired.",
          },
          {
            id: "visibility.messages",
            name: "Can’t read your chat with your agent",
            example: "Usually Full Disk Access was turned off, or the model isn’t ready.",
          },
          {
            id: "visibility.catch_up",
            name: "Catching up after your Mac was asleep or offline",
            example: "Picket checks what happened meanwhile and says what it couldn’t see.",
          },
        ],
      },
    ],
  },
];

function sendsPhrase(actions: Action[]): string {
  const email = actions.includes("email_send");
  const invites = actions.includes("calendar_create") || actions.includes("calendar_update");
  if (email && invites) return "email or invite";
  return email ? "email" : "invite";
}

function list(values: string[]): string {
  if (values.length <= 1) return values[0] ?? "";
  return `${values.slice(0, -1).join(", ")} or ${values[values.length - 1]}`;
}

/** A boundary as the sentence the user built. */
export function describeBoundary(kind: BoundaryKind): string {
  switch (kind.type) {
    case "contact": {
      const who = [...kind.addresses, ...kind.domains.map((domain) => `anyone at ${domain}`)];
      return `Never ${sendsPhrase(kind.actions)} ${list(who)}`;
    }
    case "calendar_change": {
      const verbs = kind.actions.map((item) =>
        item === "calendar_delete"
          ? "delete"
          : item === "calendar_update"
            ? "move or change"
            : "create",
      );
      return `Never ${list(verbs)} calendar events`;
    }
    case "quiet_hours": {
      const days =
        kind.days === "weekdays" ? " on weekdays" : kind.days === "weekends" ? " on weekends" : "";
      return `No ${sendsPhrase(kind.actions)}s between ${minutesToTime(kind.start_minute)} and ${minutesToTime(kind.end_minute)}${days}`;
    }
    case "burst":
      return `No more than ${kind.count} ${kind.actions.includes("email_send") && kind.actions.length === 1 ? "emails" : "actions"} within ${kind.window_minutes} minutes`;
    case "new_recipient":
      return "Emails to people you’ve never exchanged email with";
  }
}
