import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useAgentName } from "../App";
import { DesktopOnly, Failure, LevelControl } from "../components/ui";
import {
  type Action,
  type BoundaryKind,
  deleteBoundary,
  getRules,
  type Level,
  previewBoundary,
  saveBoundary,
} from "../lib/api";
import { levelHelp, people } from "../lib/copy";
import { when } from "../lib/format";
import { href, navigate } from "../lib/router";

type Template = "contact" | "calendar_change" | "quiet_hours" | "burst" | "new_recipient";
type Sends = "both" | "email" | "invite";

const templates: { id: Template; title: string; text: string }[] = [
  { id: "contact", title: "A person or company", text: "Never contact them" },
  { id: "calendar_change", title: "My calendar", text: "Never delete or move events" },
  { id: "quiet_hours", title: "Quiet hours", text: "Nothing sent overnight" },
  { id: "burst", title: "Bursts", text: "Too much at once" },
  { id: "new_recipient", title: "New people", text: "Someone I don’t write to" },
];

type Form = {
  template: Template;
  sends: Sends;
  targets: string;
  calendar: "delete" | "change" | "create" | "any";
  start: string;
  end: string;
  days: "every" | "weekdays" | "weekends";
  count: number;
  window: number;
};

const initial: Form = {
  template: "contact",
  sends: "both",
  targets: "",
  calendar: "delete",
  start: "22:00",
  end: "07:00",
  days: "every",
  count: 10,
  window: 15,
};

function sendActions(sends: Sends): Action[] {
  if (sends === "email") return ["email_send"];
  if (sends === "invite") return ["calendar_create", "calendar_update"];
  return ["email_send", "calendar_create", "calendar_update"];
}

function minutes(value: string): number {
  const [hours, mins] = value.split(":").map(Number);
  return (hours ?? 0) * 60 + (mins ?? 0);
}

function clockValue(value: number): string {
  return `${String(Math.floor(value / 60)).padStart(2, "0")}:${String(value % 60).padStart(2, "0")}`;
}

/** The boundary a form describes, or null while it is incomplete. */
export function toKind(form: Form): BoundaryKind | null {
  switch (form.template) {
    case "contact": {
      const tokens = form.targets
        .split(/[\s,;]+/)
        .map((token) => token.trim().toLowerCase())
        .filter(Boolean);
      const addresses = tokens.filter((token) => /^[^@]+@[^@]+\.[^@]+$/.test(token));
      const domains = tokens
        .filter((token) => !addresses.includes(token))
        .map((token) => token.replace(/^@/, ""));
      if (tokens.length === 0) return null;
      return { type: "contact", addresses, domains, actions: sendActions(form.sends) };
    }
    case "calendar_change": {
      const actions: Action[] =
        form.calendar === "delete"
          ? ["calendar_delete"]
          : form.calendar === "change"
            ? ["calendar_update"]
            : form.calendar === "create"
              ? ["calendar_create"]
              : ["calendar_create", "calendar_update", "calendar_delete"];
      return { type: "calendar_change", actions };
    }
    case "quiet_hours":
      if (minutes(form.start) === minutes(form.end)) return null;
      return {
        type: "quiet_hours",
        start_minute: minutes(form.start),
        end_minute: minutes(form.end),
        days: form.days,
        zone: "",
        actions: sendActions(form.sends),
      };
    case "burst":
      if (!Number.isInteger(form.count) || form.count < 1 || form.count > 1000) return null;
      return {
        type: "burst",
        count: form.count,
        window_minutes: form.window,
        actions: form.sends === "invite" ? sendActions("invite") : sendActions(form.sends),
      };
    case "new_recipient":
      return { type: "new_recipient" };
  }
}

function fromKind(kind: BoundaryKind): Form {
  const sends = (actions: Action[]): Sends =>
    actions.includes("email_send") ? (actions.length > 1 ? "both" : "email") : "invite";
  switch (kind.type) {
    case "contact":
      return {
        ...initial,
        template: "contact",
        sends: sends(kind.actions),
        targets: [...kind.addresses, ...kind.domains].join(", "),
      };
    case "calendar_change":
      return {
        ...initial,
        template: "calendar_change",
        calendar:
          kind.actions.length > 1
            ? "any"
            : kind.actions[0] === "calendar_delete"
              ? "delete"
              : kind.actions[0] === "calendar_update"
                ? "change"
                : "create",
      };
    case "quiet_hours":
      return {
        ...initial,
        template: "quiet_hours",
        sends: sends(kind.actions),
        start: clockValue(kind.start_minute),
        end: clockValue(kind.end_minute),
        days: kind.days,
      };
    case "burst":
      return {
        ...initial,
        template: "burst",
        sends: sends(kind.actions),
        count: kind.count,
        window: kind.window_minutes,
      };
    case "new_recipient":
      return { ...initial, template: "new_recipient" };
  }
}

function useDebounced<T>(value: T, delay = 300): T {
  const [current, setCurrent] = useState(value);
  useEffect(() => {
    const timer = window.setTimeout(() => setCurrent(value), delay);
    return () => window.clearTimeout(timer);
  }, [value, delay]);
  return current;
}

function SendsSelect({
  form,
  set,
  label,
}: {
  form: Form;
  set: (form: Form) => void;
  label: string;
}) {
  return (
    <select
      aria-label={label}
      value={form.sends}
      onChange={(event) => set({ ...form, sends: event.target.value as Sends })}
    >
      <option value="both">an email or invite</option>
      <option value="email">an email</option>
      <option value="invite">an invite</option>
    </select>
  );
}

function Sentence({ form, set }: { form: Form; set: (form: Form) => void }) {
  switch (form.template) {
    case "contact":
      return (
        <div className="sentence">
          <SendsSelect form={form} set={set} label="What" />
          <span>goes to</span>
          <input
            aria-label="People or companies"
            placeholder="bank.com or sam@example.com"
            value={form.targets}
            autoComplete="off"
            onChange={(event) => set({ ...form, targets: event.target.value })}
          />
        </div>
      );
    case "calendar_change":
      return (
        <div className="sentence">
          <span>a calendar event is</span>
          <select
            aria-label="Change"
            value={form.calendar}
            onChange={(event) => set({ ...form, calendar: event.target.value as Form["calendar"] })}
          >
            <option value="delete">deleted</option>
            <option value="change">moved or changed</option>
            <option value="create">created</option>
            <option value="any">changed in any way</option>
          </select>
        </div>
      );
    case "quiet_hours":
      return (
        <div className="sentence">
          <SendsSelect form={form} set={set} label="What" />
          <span>is sent between</span>
          <input
            type="time"
            aria-label="From"
            value={form.start}
            onChange={(event) => set({ ...form, start: event.target.value })}
          />
          <span>and</span>
          <input
            type="time"
            aria-label="Until"
            value={form.end}
            onChange={(event) => set({ ...form, end: event.target.value })}
          />
          <select
            aria-label="Days"
            value={form.days}
            onChange={(event) => set({ ...form, days: event.target.value as Form["days"] })}
          >
            <option value="every">every day</option>
            <option value="weekdays">on weekdays</option>
            <option value="weekends">on weekends</option>
          </select>
        </div>
      );
    case "burst":
      return (
        <div className="sentence">
          <span>more than</span>
          <input
            type="number"
            className="short"
            aria-label="How many"
            min={1}
            max={1000}
            value={form.count}
            onChange={(event) => set({ ...form, count: Number(event.target.value) })}
          />
          <select
            aria-label="Of what"
            value={form.sends}
            onChange={(event) => set({ ...form, sends: event.target.value as Sends })}
          >
            <option value="email">emails</option>
            <option value="invite">invites</option>
            <option value="both">emails or invites</option>
          </select>
          <span>go out within</span>
          <select
            aria-label="Window"
            value={form.window}
            onChange={(event) => set({ ...form, window: Number(event.target.value) })}
          >
            <option value={5}>5 minutes</option>
            <option value={15}>15 minutes</option>
            <option value={60}>1 hour</option>
          </select>
        </div>
      );
    case "new_recipient":
      return (
        <div className="sentence">
          <span>an email goes to someone you’ve never exchanged email with</span>
        </div>
      );
  }
}

export function BoundaryEditor({ id }: { id: string | null }) {
  const client = useQueryClient();
  const agent = useAgentName() ?? "your agent";
  const rules = useQuery({ queryKey: ["rules"], queryFn: getRules });
  const existing = id ? rules.data?.boundaries.find((item) => item.id === id) : undefined;
  const [form, setForm] = useState<Form>(initial);
  const [level, setLevel] = useState<Level>("urgent");
  const [loaded, setLoaded] = useState(id === null);
  const [error, setError] = useState<unknown>(null);
  useEffect(() => {
    if (existing && !loaded) {
      setForm(fromKind(existing.kind));
      setLevel(existing.level);
      setLoaded(true);
    }
  }, [existing, loaded]);
  const kind = toKind(form);
  const pending = useDebounced(kind);
  const preview = useQuery({
    queryKey: ["preview", pending],
    queryFn: () => (pending ? previewBoundary(pending) : Promise.resolve(null)),
    enabled: pending !== null,
    placeholderData: keepPreviousData,
  });
  const save = useMutation({
    mutationFn: () => {
      if (!kind) throw new Error("Finish the sentence first.");
      return saveBoundary(id, level, kind);
    },
    onSuccess: async () => {
      await client.invalidateQueries();
      navigate({ page: "rules" });
    },
    onError: setError,
  });
  const remove = useMutation({
    mutationFn: () => (id ? deleteBoundary(id) : Promise.resolve()),
    onSuccess: async () => {
      await client.invalidateQueries();
      navigate({ page: "rules" });
    },
    onError: setError,
  });
  if (rules.isSuccess && rules.data === null) return <DesktopOnly />;
  const count = preview.data?.count ?? 0;
  return (
    <div className="stack stack-narrow">
      <header>
        <h1>{id ? "Edit boundary" : "Add a boundary"}</h1>
        <p className="lede">A line that should never be crossed, whoever does it.</p>
      </header>
      <section aria-labelledby="start-heading">
        <h2 id="start-heading" className="label section-label">
          Start from
        </h2>
        <div className="templates">
          {templates.map((template) => (
            <button
              key={template.id}
              type="button"
              className={form.template === template.id ? "template on" : "template"}
              aria-pressed={form.template === template.id}
              onClick={() => setForm({ ...form, template: template.id })}
            >
              <span className="strong">{template.title}</span>
              <span className="caption">{template.text}</span>
            </button>
          ))}
        </div>
      </section>
      <section aria-labelledby="when-heading">
        <h2 id="when-heading" className="label section-label">
          Flag it when
        </h2>
        <Sentence form={form} set={setForm} />
      </section>
      <section className="loudness" aria-labelledby="loud-heading">
        <div>
          <h2 id="loud-heading" className="label">
            How loud
          </h2>
          <p className="caption">{levelHelp[level]}</p>
        </div>
        <LevelControl label="How loud" value={level} onChange={setLevel} allowOff={false} />
      </section>
      <section className="card panel" aria-live="polite" aria-label="What it would have flagged">
        <div className="panel-head">
          <p className="strong">
            {kind === null
              ? "Finish the sentence to see what it would catch"
              : count === 0
                ? "Would have flagged nothing"
                : `Would have flagged ${count}`}
          </p>
          <span className="caption">Checked against your last 30 days</span>
        </div>
        {kind !== null && count > 0 && (
          <ul className="plain-list">
            {preview.data?.hits.map((hit) => (
              <li key={`${hit.at}-${hit.subject ?? ""}`} className="activity-row">
                <span className="row-time">{when(hit.at)}</span>
                <span className="row-main">
                  {hit.subject ? `“${hit.subject}”` : people(hit.people) || "An action"}
                  {hit.subject && hit.people.length > 0 && ` to ${people(hit.people)}`}
                </span>
                <span className="caption">{hit.agent ?? "Not linked to an agent"}</span>
              </li>
            ))}
          </ul>
        )}
      </section>
      <p className="caption">
        Boundaries also catch things you do yourself, unless {agent} reported doing them. You can
        mark any alert as fine.
      </p>
      {error !== null && <Failure error={error} />}
      <div className="footer-actions">
        {id && (
          <button
            type="button"
            className="button danger-text"
            disabled={remove.isPending}
            onClick={() => remove.mutate()}
          >
            Remove boundary
          </button>
        )}
        <span className="spacer" />
        <a className="button" href={href({ page: "rules" })}>
          Cancel
        </a>
        <button
          type="button"
          className="button primary"
          disabled={kind === null || save.isPending}
          onClick={() => save.mutate()}
        >
          {id ? "Save" : "Add boundary"}
        </button>
      </div>
    </div>
  );
}
