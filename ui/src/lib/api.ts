import { invoke, isTauri } from "@tauri-apps/api/core";
import { z } from "zod";

// Every IPC response is validated before the renderer uses it, as in runtime.ts.

const time = z.string().min(1);
export const action = z.enum([
  "email_send",
  "calendar_create",
  "calendar_update",
  "calendar_delete",
  "calendar_rsvp",
]);
export type Action = z.infer<typeof action>;
export const source = z.enum(["gmail", "calendar", "messages", "open_claw", "picket"]);
export type Source = z.infer<typeof source>;
export const levels = ["urgent", "review", "summary", "off"] as const;
export const level = z.enum(levels);
export type Level = z.infer<typeof level>;
export const coverageState = z.enum(["healthy", "delayed", "gap", "disabled"]);
export const coverageReason = z.enum([
  "cursor_expired",
  "page_limit",
  "missing_resource",
  "request_failed",
  "auth_unavailable",
  "interrupted",
  "permission_missing",
  "unsupported_format",
  "undecodable",
  "extraction_unavailable",
]);
export type CoverageReason = z.infer<typeof coverageReason>;

const person = z.object({
  address: z.string(),
  name: z.string().nullable(),
  unrequested: z.boolean(),
});
export type Person = z.infer<typeof person>;

export const reason = z.enum([
  "after_revocation",
  "boundary",
  "prohibited_action",
  "approval_missing",
  "extra_target",
  "different_calendar_time",
  "too_many_actions",
  "different_action",
  "outside_window",
  "report_not_observed",
  "report_contradicted",
  "forwarding",
  "app_access",
  "otp",
  "purchase",
  "sign_in_needed",
  "source_gap",
  "source_delayed",
]);
export type Reason = z.infer<typeof reason>;
export const notice = z.enum(["forwarding", "app_access", "purchase", "otp"]);

const summary = z.object({
  id: z.string().startsWith("incident/"),
  severity: z.enum(["review", "urgent"]),
  status: z.enum(["open", "fine", "not_ok"]),
  kind: z.enum(["email", "calendar", "report", "account_warning", "visibility"]),
  reasons: z.array(reason),
  agent: z.string().nullable(),
  occurredAt: time,
  flaggedAt: time,
  subject: z.string().nullable(),
  people: z.array(person),
});
export type IncidentSummary = z.infer<typeof summary>;

const days = z.enum(["every", "weekdays", "weekends"]);
const boundaryKind = z.discriminatedUnion("type", [
  z.object({
    type: z.literal("contact"),
    addresses: z.array(z.string()),
    domains: z.array(z.string()),
    actions: z.array(action),
  }),
  z.object({ type: z.literal("calendar_change"), actions: z.array(action) }),
  z.object({
    type: z.literal("quiet_hours"),
    start_minute: z.number().int(),
    end_minute: z.number().int(),
    days,
    zone: z.string(),
    actions: z.array(action),
  }),
  z.object({
    type: z.literal("burst"),
    count: z.number().int(),
    window_minutes: z.number().int(),
    actions: z.array(action),
  }),
  z.object({ type: z.literal("new_recipient") }),
]);
export type BoundaryKind = z.infer<typeof boundaryKind>;
const boundary = z.object({ id: z.string(), level, since: time, kind: boundaryKind });
export type Boundary = z.infer<typeof boundary>;

const detail = summary.extend({
  findings: z.array(z.string()),
  request: z
    .object({
      at: time,
      message: z.string().nullable(),
      actions: z.array(action).nullable(),
      anyAction: z.boolean(),
      targets: z.array(z.string()),
      targetNames: z.array(z.string()),
      calendarStart: time.nullable(),
      prohibitedActions: z.array(action),
      requiresApproval: z.boolean().nullable(),
      approved: z.boolean(),
      maxActions: z.number().nullable(),
      validUntil: time.nullable(),
    })
    .nullable(),
  reports: z.array(
    z.object({
      at: time,
      message: z.string().nullable(),
      action,
      targets: z.array(z.string()),
      targetNames: z.array(z.string()),
      channel: z.enum(["user_account", "agent_account", "unknown"]),
    }),
  ),
  effect: z
    .object({
      at: time,
      source,
      action,
      subject: z.string().nullable(),
      calendarStart: time.nullable(),
      people: z.array(person),
      targetsComplete: z.boolean(),
      thread: z.string().nullable(),
    })
    .nullable(),
  actor: z.object({
    basis: z.enum(["unknown", "reported", "runtime", "confirmed", "conflicting"]),
    agents: z.array(z.string()),
  }),
  requestFit: z
    .object({
      verdict: z.enum([
        "not_assessed",
        "no_request",
        "not_captured",
        "ambiguous",
        "insufficient",
        "consistent",
        "conflicts",
      ]),
      conflicts: z.array(z.string()),
    })
    .nullable(),
  reportStatus: z.string().nullable(),
  notice: notice.nullable(),
  visibility: z
    .object({
      source,
      capability: z.string(),
      state: coverageState,
      reason: coverageReason.nullable(),
      since: time,
    })
    .nullable(),
  boundaries: z.array(boundary),
  decision: z.object({ expected: z.boolean(), at: time }).nullable(),
  history: z.array(
    z.object({
      at: time,
      kind: z.enum([
        "requested",
        "happened",
        "reported",
        "flagged",
        "revised",
        "marked_fine",
        "marked_not_ok",
      ]),
    }),
  ),
});
export type IncidentDetail = z.infer<typeof detail>;

const item = z.object({
  id: z.string(),
  at: time,
  kind: z.enum(["email", "calendar", "request", "report", "gap", "recovered"]),
  action: action.nullable(),
  subject: z.string().nullable(),
  people: z.array(person),
  message: z.string().nullable(),
  agent: z.string().nullable(),
  verdict: z.enum(["as_asked", "review", "unclear", "not_linked", "none"]),
  incident: z.string().nullable(),
  outsideScope: z.boolean(),
  source: source.nullable(),
  gapReason: coverageReason.nullable(),
});
export type ActivityItem = z.infer<typeof item>;

const overview = z.object({
  counts: z.object({
    agentActions: z.number(),
    asAsked: z.number(),
    review: z.number(),
    unclear: z.number(),
    notLinked: z.number(),
  }),
  recent: z.array(item),
  sources: z.array(
    z.object({
      source,
      scope: z.string(),
      state: coverageState,
      reason: coverageReason.nullable(),
      since: time,
    }),
  ),
});
export type Overview = z.infer<typeof overview>;

const page = z.object({ items: z.array(item), next: z.string().nullable() });
export type ActivityPage = z.infer<typeof page>;

const rules = z.object({
  levels: z.array(z.object({ id: z.string(), level, default: level })),
  boundaries: z.array(boundary.extend({ recent: z.number() })),
});
export type Rules = z.infer<typeof rules>;

const preview = z.object({
  count: z.number(),
  hits: z.array(
    z.object({
      at: time,
      action,
      subject: z.string().nullable(),
      people: z.array(person),
      agent: z.string().nullable(),
    }),
  ),
});
export type Preview = z.infer<typeof preview>;

export type Period = { from: string; to: string };

const desktopOnly = "Open the desktop app to see this.";

export async function getOverview(period: Period): Promise<Overview | null> {
  if (!isTauri()) return null;
  return overview.parse(await invoke<unknown>("get_overview", period));
}

export async function listActivity(
  period: Period,
  who: "everyone" | "agents" | "not_linked",
  result: "any" | "review" | "as_asked" | "unclear",
  before: string | null,
): Promise<ActivityPage | null> {
  if (!isTauri()) return null;
  return page.parse(await invoke<unknown>("list_activity", { ...period, who, result, before }));
}

export async function listIncidents(): Promise<IncidentSummary[] | null> {
  if (!isTauri()) return null;
  return z.array(summary).parse(await invoke<unknown>("list_incidents"));
}

export async function getIncident(id: string): Promise<IncidentDetail | null> {
  if (!isTauri()) return null;
  return detail.parse(await invoke<unknown>("get_incident", { id }));
}

export async function resolveIncident(
  id: string,
  decision: "fine" | "not_ok" | "undo",
): Promise<void> {
  if (!isTauri()) throw new Error(desktopOnly);
  await invoke("resolve_incident", { id, decision });
}

export async function setAgentRevoked(id: string, revoked: boolean): Promise<void> {
  if (!isTauri()) throw new Error(desktopOnly);
  await invoke("set_agent_revoked", { id, revoked });
}

export async function getRules(): Promise<Rules | null> {
  if (!isTauri()) return null;
  return rules.parse(await invoke<unknown>("get_rules"));
}

export async function setRuleLevel(rule: string, value: Level): Promise<void> {
  if (!isTauri()) throw new Error(desktopOnly);
  await invoke("set_rule_level", { rule, level: value });
}

export async function previewBoundary(kind: BoundaryKind): Promise<Preview | null> {
  if (!isTauri()) return null;
  return preview.parse(await invoke<unknown>("preview_boundary", { kind }));
}

export async function saveBoundary(
  id: string | null,
  value: Level,
  kind: BoundaryKind,
): Promise<string> {
  if (!isTauri()) throw new Error(desktopOnly);
  return z.string().parse(await invoke<unknown>("save_boundary", { id, level: value, kind }));
}

export async function deleteBoundary(id: string): Promise<void> {
  if (!isTauri()) throw new Error(desktopOnly);
  await invoke("delete_boundary", { id });
}

export type LinkTarget =
  | "gmail_thread"
  | "google_access"
  | "notification_settings"
  | "full_disk_access";

export async function openLink(target: LinkTarget, thread?: string): Promise<void> {
  if (!isTauri()) throw new Error(desktopOnly);
  await invoke("open_link", { target, thread: thread ?? null });
}

const settings = z.object({
  summaryMinutes: z.number().int().min(0).max(1439).nullable(),
  retentionDays: z.number().int().positive(),
  alertsPausedUntil: time.nullable(),
  lastSummaryDay: z.string().nullable(),
  startAtLogin: z.boolean(),
});
export type Settings = z.infer<typeof settings>;
export type SettingsPatch = { summaryMinutes?: number | null; retentionDays?: number };

export async function getSettings(): Promise<Settings | null> {
  if (!isTauri()) return null;
  return settings.parse(await invoke<unknown>("get_settings"));
}

export async function updateSettings(patch: SettingsPatch): Promise<Settings> {
  if (!isTauri()) throw new Error(desktopOnly);
  return settings.parse(await invoke<unknown>("update_settings", { patch }));
}

export async function setStartAtLogin(enabled: boolean): Promise<boolean> {
  if (!isTauri()) throw new Error(desktopOnly);
  return z.boolean().parse(await invoke<unknown>("set_start_at_login", { enabled }));
}

const verification = z.object({
  valid: z.boolean(),
  throughSequence: z.number().int().nullable(),
  retainedEvents: z.number().int().nullable(),
  brokenAt: z.number().int().nullable(),
});
export type Verification = z.infer<typeof verification>;

export async function verifyLog(): Promise<Verification> {
  if (!isTauri()) throw new Error(desktopOnly);
  return verification.parse(await invoke<unknown>("verify_log"));
}

/** Opens a save panel; resolves false when the person cancels. */
export async function exportIncident(id: string): Promise<boolean> {
  if (!isTauri()) throw new Error(desktopOnly);
  return z.boolean().parse(await invoke<unknown>("export_incident", { id }));
}

export async function exportAll(): Promise<boolean> {
  if (!isTauri()) throw new Error(desktopOnly);
  return z.boolean().parse(await invoke<unknown>("export_all"));
}

/** Deletes everything Picket keeps on this Mac, opens Google's access page and quits. */
export async function wipeAll(): Promise<void> {
  if (!isTauri()) throw new Error(desktopOnly);
  await invoke("wipe_all");
}
