import type { Page } from "@playwright/test";

// A synthetic desktop core for browser tests. It answers Picket's IPC commands with
// fixed, made-up data and records decisions, so screens can be exercised end to end.

const minutesAgo = (minutes: number) => new Date(Date.now() - minutes * 60_000).toISOString();

export function fixtures() {
  const sam = { address: "sam@lee.example", name: "Sam Lee", unrequested: false };
  const jordan = { address: "jordan@park.example", name: "Jordan Park", unrequested: true };
  const extra = {
    id: "incident/aaa111",
    severity: "review",
    status: "open",
    kind: "email",
    reasons: ["extra_target"],
    agent: "Instinct",
    occurredAt: minutesAgo(40),
    flaggedAt: minutesAgo(35),
    subject: "Lunch Thursday?",
    people: [sam, jordan],
  };
  const time = {
    id: "incident/bbb222",
    severity: "review",
    status: "open",
    kind: "calendar",
    reasons: ["different_calendar_time"],
    agent: "Instinct",
    occurredAt: minutesAgo(200),
    flaggedAt: minutesAgo(195),
    subject: "Dentist",
    people: [],
  };
  const detail = {
    ...extra,
    findings: ["effect/1", "claim/1"],
    request: {
      at: minutesAgo(43),
      message: "Email Sam about lunch Thursday",
      actions: ["email_send"],
      anyAction: false,
      targets: [],
      targetNames: ["Sam"],
      calendarStart: null,
      prohibitedActions: [],
      requiresApproval: false,
      approved: false,
      maxActions: null,
      validUntil: null,
    },
    reports: [
      {
        at: minutesAgo(39),
        message: "Sent!",
        action: "email_send",
        targets: [],
        targetNames: ["Sam"],
        channel: "user_account",
      },
    ],
    effect: {
      at: minutesAgo(40),
      source: "gmail",
      action: "email_send",
      subject: "Lunch Thursday?",
      calendarStart: null,
      people: [sam, jordan],
      targetsComplete: true,
      thread: "18c2f0a1b2",
    },
    actor: { basis: "reported", agents: ["Instinct"] },
    requestFit: { verdict: "conflicts", conflicts: ["ExtraTarget"] },
    reportStatus: "Matched",
    notice: null,
    visibility: null,
    boundaries: [],
    decision: null,
    history: [
      { at: minutesAgo(43), kind: "requested" },
      { at: minutesAgo(40), kind: "happened" },
      { at: minutesAgo(39), kind: "reported" },
      { at: minutesAgo(35), kind: "flagged" },
    ],
  };
  const item = (id: string, minutes: number, rest: { kind: string } & Record<string, unknown>) => ({
    id,
    at: minutesAgo(minutes),
    action: null,
    subject: null,
    people: [],
    message: null,
    agent: "Instinct",
    verdict: "none",
    incident: null,
    outsideScope: false,
    source: null,
    gapReason: null,
    ...rest,
  });
  const activity = [
    item("e1", 40, {
      kind: "email",
      action: "email_send",
      subject: "Lunch Thursday?",
      people: [sam, jordan],
      verdict: "review",
      incident: extra.id,
      source: "gmail",
    }),
    item("r1", 43, { kind: "request", message: "Email Sam about lunch Thursday" }),
    item("e2", 90, {
      kind: "email",
      action: "email_send",
      subject: "Re: Plans",
      people: [{ address: "priya@shah.example", name: "Priya Shah", unrequested: false }],
      verdict: "as_asked",
      source: "gmail",
    }),
    item("e3", 130, {
      kind: "email",
      action: "email_send",
      subject: "Invoice 0412",
      people: [{ address: "accounts@studio.example", name: null, unrequested: false }],
      agent: null,
      verdict: "not_linked",
      source: "gmail",
    }),
    item("c1", 200, {
      kind: "calendar",
      action: "calendar_create",
      subject: "Dentist",
      verdict: "review",
      incident: time.id,
      source: "calendar",
    }),
    item("p1", 260, {
      kind: "report",
      action: "email_send",
      message: "I sent a note to the landlord from my address",
      verdict: "unclear",
      outsideScope: true,
    }),
    item("g1", 600, { kind: "gap", agent: null, source: "gmail", gapReason: "interrupted" }),
  ];
  const builtIn = [
    ["request.prohibited_action", "review"],
    ["request.approval_missing", "review"],
    ["request.extra_target", "review"],
    ["request.different_time", "review"],
    ["request.too_many", "review"],
    ["request.different_action", "review"],
    ["request.outside_window", "review"],
    ["request.after_revocation", "urgent"],
    ["report.not_observed", "review"],
    ["report.contradicted", "review"],
    ["warning.forwarding", "review"],
    ["warning.app_access", "review"],
    ["warning.otp", "off"],
    ["warning.purchase", "summary"],
    ["visibility.google", "review"],
    ["visibility.messages", "review"],
    ["visibility.catch_up", "summary"],
  ];
  return {
    get_runtime_info: {
      version: "0.1.0",
      platform: "macos",
      architecture: "aarch64",
      monitoring: true,
    },
    google_connected: "monitoring",
    list_agents: {
      agents: [
        {
          id: "instinct",
          name: "Instinct",
          mode: "messages",
          handles: ["+15550104477"],
          enabled: true,
        },
      ],
      messages: "capturing",
    },
    model_status: { status: { state: "ready" }, id: "synthetic-model", bytes: 2_710_000_000 },
    list_incidents: [extra, time],
    get_incident: detail,
    get_overview: {
      counts: { agentActions: 10, asAsked: 7, review: 2, unclear: 1, notLinked: 23 },
      recent: activity.filter((entry) => entry.agent && entry.kind !== "request").slice(0, 5),
      sources: [
        {
          source: "calendar",
          scope: "effects",
          state: "healthy",
          reason: null,
          since: minutesAgo(900),
        },
        {
          source: "gmail",
          scope: "effects",
          state: "healthy",
          reason: null,
          since: minutesAgo(900),
        },
        {
          source: "messages",
          scope: "instinct",
          state: "gap",
          reason: "extraction_unavailable",
          since: minutesAgo(900),
        },
      ],
    },
    list_activity: { items: activity, next: null },
    get_rules: {
      levels: builtIn.map(([id, level]) => ({ id, level, default: level })),
      boundaries: [
        {
          id: "b-1",
          level: "urgent",
          since: minutesAgo(5000),
          kind: {
            type: "contact",
            addresses: [],
            domains: ["bank.example"],
            actions: ["email_send"],
          },
          recent: 0,
        },
        {
          id: "b-2",
          level: "review",
          since: minutesAgo(5000),
          kind: {
            type: "quiet_hours",
            start_minute: 1320,
            end_minute: 420,
            days: "every",
            zone: "America/Los_Angeles",
            actions: ["email_send"],
          },
          recent: 2,
        },
      ],
    },
    preview_boundary: {
      count: 1,
      hits: [
        {
          at: minutesAgo(3000),
          action: "email_send",
          subject: "Statement question",
          people: [{ address: "help@bank.example", name: null, unrequested: false }],
          agent: "Instinct",
        },
      ],
    },
  };
}

export type Fixtures = ReturnType<typeof fixtures>;

export type Call = { cmd: string; args: Record<string, unknown> };

type Incident = { id: string; status: string; decision?: unknown };
type State = Record<string, unknown> & {
  list_incidents: Incident[];
  get_incident: Incident;
  get_rules: { levels: { id: string; level: string }[] };
};

type DesktopWindow = Window & {
  isTauri?: boolean;
  __picketCalls?: Call[];
  __TAURI_INTERNALS__?: unknown;
};

/** Makes the page believe it runs in the desktop app, answering IPC from fixtures. */
export async function mockDesktop(page: Page, overrides: Partial<Record<string, unknown>> = {}) {
  const data = { ...fixtures(), ...overrides };
  await page.addInitScript((responses: Record<string, unknown>) => {
    const state = structuredClone(responses) as State;
    const calls: Call[] = [];
    const target = window as DesktopWindow;
    target.__picketCalls = calls;
    target.isTauri = true;
    target.__TAURI_INTERNALS__ = {
      transformCallback: () => 0,
      invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
        calls.push({ cmd, args });
        if (cmd === "resolve_incident") {
          const decision = String(args.decision);
          const status = { fine: "fine", not_ok: "not_ok", undo: "open" }[decision] ?? "open";
          for (const incident of state.list_incidents) {
            if (incident.id === args.id) incident.status = status;
          }
          if (state.get_incident.id === args.id) {
            state.get_incident.status = status;
            state.get_incident.decision =
              decision === "undo"
                ? null
                : { expected: decision === "fine", at: new Date().toISOString() };
          }
          return null;
        }
        if (cmd === "set_rule_level") {
          for (const rule of state.get_rules.levels) {
            if (rule.id === args.rule) rule.level = String(args.level);
          }
          return null;
        }
        if (cmd === "save_boundary") return args.id ?? "b-3";
        if (cmd in state) return structuredClone(state[cmd]);
        return null;
      },
    };
  }, data);
}

/** The IPC calls the page made, in order. */
export async function calls(page: Page): Promise<Call[]> {
  return page.evaluate(() => (window as DesktopWindow).__picketCalls ?? []);
}
