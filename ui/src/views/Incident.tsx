import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Badge, DesktopOnly, Empty, Failure, Icon } from "../components/ui";
import {
  exportIncident,
  getIncident,
  type IncidentDetail,
  openLink,
  resolveIncident,
  setAgentRevoked,
} from "../lib/api";
import {
  describeBoundary,
  incidentTitle,
  levelLabel,
  people,
  personName,
  quoted,
  reasonExplanation,
  requestedPhrase,
} from "../lib/copy";
import { clock, dateTime, when } from "../lib/format";
import { href } from "../lib/router";
import { listAgents } from "../lib/runtime";

const kindLabel: Record<IncidentDetail["kind"], string> = {
  email: "Email",
  calendar: "Calendar",
  report: "Report",
  account_warning: "Account warning",
  visibility: "What Picket can see",
};

/** One or two sentences that say what happened, in the person's terms. */
function capitalized(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1);
}

function lede(incident: IncidentDetail, agent: string): string {
  const unrequested = incident.effect?.people.filter((person) => person.unrequested) ?? [];
  const asked = incident.request?.message;
  const intro = asked
    ? `You asked ${agent}: ${quoted(asked)}${/[.!?]$/.test(asked) ? "" : "."}`
    : "";
  switch (incident.reasons[0]) {
    case "extra_target":
      return `${intro}${intro ? " " : ""}It also went to ${people(unrequested)}.`;
    case "different_calendar_time":
      if (incident.request?.calendarStart && incident.effect?.calendarStart) {
        return `${intro}${intro ? " " : ""}It booked ${dateTime(incident.effect.calendarStart)} instead of ${dateTime(incident.request.calendarStart)}.`;
      }
      break;
    case "approval_missing":
      return `${intro}${intro ? " " : ""}You asked to approve it first, and hadn’t yet.`;
    case "report_not_observed":
      return `${capitalized(agent)} said it was done, but nothing matching appeared in your account.`;
    default:
      break;
  }
  const reason = incident.reasons[0];
  return reason ? `${intro}${intro ? " " : ""}${reasonExplanation[reason]}` : intro;
}

function RequestColumn({ incident, agent }: { incident: IncidentDetail; agent: string }) {
  const request = incident.request;
  const fit = incident.requestFit?.verdict;
  if (!request) {
    let text = "Picket didn’t compare this with a request.";
    if (fit === "no_request") text = `You didn’t ask ${agent} for this in your iMessage chat.`;
    if (fit === "not_captured")
      text = "Picket couldn’t read your request, so it can’t compare them.";
    if (fit === "ambiguous") text = "Picket couldn’t tell which of your requests this was for.";
    if (fit === "not_assessed")
      text = "Picket can’t tell who did this, so it didn’t look for a request.";
    return (
      <article className="card step">
        <p className="label">1 · You asked</p>
        <p className="muted">{text}</p>
      </article>
    );
  }
  const asked = [...request.targetNames, ...request.targets];
  return (
    <article className="card step">
      <div>
        <p className="label">1 · You asked</p>
        <p className="caption">Your iMessage · {clock(request.at)}</p>
      </div>
      {request.message && <blockquote className="quote">{quoted(request.message)}</blockquote>}
      <dl className="facts">
        {request.actions && request.actions.length > 0 && (
          <>
            <dt>Action</dt>
            <dd>{request.actions.map(requestedPhrase).join(", ")}</dd>
          </>
        )}
        {asked.length > 0 && (
          <>
            <dt>To</dt>
            <dd>{asked.join(", ")}</dd>
          </>
        )}
        {request.calendarStart && (
          <>
            <dt>When</dt>
            <dd>{dateTime(request.calendarStart)}</dd>
          </>
        )}
        {request.prohibitedActions.length > 0 && (
          <>
            <dt>Not to</dt>
            <dd>{request.prohibitedActions.map(requestedPhrase).join(", ")}</dd>
          </>
        )}
        {request.requiresApproval !== null && (
          <>
            <dt>Your OK first</dt>
            <dd>
              {request.requiresApproval
                ? request.approved
                  ? "Asked for, then given"
                  : "Asked for, not given"
                : "Not asked for"}
            </dd>
          </>
        )}
      </dl>
    </article>
  );
}

function ReportColumn({ incident, agent }: { incident: IncidentDetail; agent: string }) {
  return (
    <article className="card step">
      <p className="label">2 · {capitalized(agent)} said</p>
      {incident.reports.length === 0 ? (
        <p className="muted">{agent} didn’t report this.</p>
      ) : (
        incident.reports.map((report) => (
          <div key={report.at} className="report">
            <p className="caption">
              {agent}’s reply · {clock(report.at)}
            </p>
            {report.message && <blockquote className="quote">{quoted(report.message)}</blockquote>}
            {report.channel === "agent_account" && (
              <p className="muted">
                It says it sent this from its own address, which Picket can’t see.
              </p>
            )}
          </div>
        ))
      )}
    </article>
  );
}

function EffectColumn({ incident }: { incident: IncidentDetail }) {
  const effect = incident.effect;
  const flagged = (effect?.people.some((person) => person.unrequested) ?? false) || !effect;
  return (
    <article className={flagged ? "card step step-flagged" : "card step"}>
      <div>
        <p className="label">3 · What actually happened</p>
        {effect && (
          <p className="caption">
            {effect.source === "gmail" ? "Gmail, Sent" : "Calendar"} · {clock(effect.at)}
          </p>
        )}
      </div>
      {!effect ? (
        <p className="muted">Nothing matching appeared in your Gmail or Calendar.</p>
      ) : (
        <>
          {effect.subject && <p className="effect-subject">{quoted(effect.subject)}</p>}
          {effect.calendarStart && <p>{dateTime(effect.calendarStart)}</p>}
          <ul className="people">
            {effect.people.map((person) => (
              <li key={person.address} className={person.unrequested ? "person miss" : "person"}>
                <Icon name={person.unrequested ? "warning" : "check"} size={16} />
                <span className="person-main">
                  <span className={person.unrequested ? "strong" : undefined}>
                    {personName(person)}
                  </span>
                  {person.name && <span className="caption">{person.address}</span>}
                  {person.unrequested && <span className="miss-label">Not in your request</span>}
                </span>
              </li>
            ))}
          </ul>
          {!effect.targetsComplete && (
            <p className="muted">Picket couldn’t see every recipient of this one.</p>
          )}
          {effect.thread && (
            <button
              type="button"
              className="link-button"
              onClick={() => void openLink("gmail_thread", effect.thread ?? undefined)}
            >
              Open in Gmail
            </button>
          )}
        </>
      )}
    </article>
  );
}

function Certainty({ incident, agent }: { incident: IncidentDetail; agent: string }) {
  const actor = incident.actor;
  const who = actor.agents[0] ?? agent;
  const actorText = {
    reported: `${who}, because it reported doing this`,
    runtime: `${who}, linked by its own record of the action`,
    confirmed: `${who}, as you confirmed`,
    conflicting: `More than one agent reported this: ${actor.agents.join(", ")}`,
    unknown: "Picket can’t tell. It could be you or someone else.",
  }[actor.basis];
  const fit = incident.requestFit?.verdict;
  const requestText = incident.request
    ? "Read from your iMessage chat. Requests made elsewhere, such as WhatsApp, aren’t seen."
    : fit === "not_captured"
      ? "Picket couldn’t read your chat at the time."
      : "No matching request in your iMessage chat.";
  return (
    <section className="card panel" aria-labelledby="sure-heading">
      <h2 id="sure-heading" className="panel-title">
        How sure Picket is
      </h2>
      <dl className="facts facts-wide">
        {incident.effect && (
          <>
            <dt>Who did it</dt>
            <dd>{actorText}</dd>
          </>
        )}
        {(incident.kind === "email" || incident.kind === "calendar") && (
          <>
            <dt>Your request</dt>
            <dd>{requestText}</dd>
          </>
        )}
        {incident.visibility && (
          <>
            <dt>Since</dt>
            <dd>{dateTime(incident.visibility.since)}</dd>
          </>
        )}
        {incident.notice && (
          <>
            <dt>Evidence</dt>
            <dd>An email’s sender and subject. Picket can’t confirm the change itself.</dd>
          </>
        )}
      </dl>
    </section>
  );
}

function Why({ incident }: { incident: IncidentDetail }) {
  return (
    <section className="card panel" aria-labelledby="why-heading">
      <h2 id="why-heading" className="panel-title">
        Why Picket flagged this
      </h2>
      <ul className="why">
        {incident.reasons
          .filter((reason) => reason !== "boundary")
          .map((reason) => (
            <li key={reason}>{reasonExplanation[reason]}</li>
          ))}
        {incident.boundaries.map((boundary) => (
          <li key={boundary.id}>
            Your boundary: {describeBoundary(boundary.kind)} ({levelLabel[boundary.level]})
          </li>
        ))}
      </ul>
      <div className="why-footer">
        <span className="muted">Change how loudly Picket tells you about this.</span>
        <a className="quiet-link" href={href({ page: "rules" })}>
          Rules
        </a>
      </div>
    </section>
  );
}

const historyText: Record<IncidentDetail["history"][number]["kind"], (agent: string) => string> = {
  requested: (agent) => `You asked ${agent}`,
  happened: () => "It happened in your account",
  reported: (agent) => `${agent} reported it`,
  flagged: () => "Picket flagged it",
  revised: () => "Picket revised its assessment after new evidence",
  marked_fine: () => "You marked it as fine",
  marked_not_ok: () => "You said it wasn’t OK",
};

/** Saves the incident as a Markdown report, whatever its status. */
function ExportReport({ id }: { id: string }) {
  const save = useMutation({ mutationFn: () => exportIncident(id) });
  return (
    <div className="page-footer">
      <span className="muted" role="status">
        {save.data ? "Report saved." : "Keep or share a copy of what happened."}
      </span>
      <button
        type="button"
        className="button small"
        disabled={save.isPending}
        onClick={() => save.mutate()}
      >
        {save.isPending ? "Saving…" : "Export report…"}
      </button>
      {save.isError && <Failure error={save.error} />}
    </div>
  );
}

function NextSteps({
  incident,
  agentId,
  agent,
}: {
  incident: IncidentDetail;
  agentId: string | null;
  agent: string;
}) {
  const client = useQueryClient();
  const revoke = useMutation({
    mutationFn: () => (agentId ? setAgentRevoked(agentId, true) : Promise.resolve()),
    onSuccess: () => client.invalidateQueries(),
  });
  return (
    <ul className="checklist">
      {incident.effect?.thread && (
        <li>
          <span>
            <span className="strong">Check the email</span>
            <span className="caption">See exactly what was sent, and to whom.</span>
          </span>
          <button
            type="button"
            className="button small"
            onClick={() => void openLink("gmail_thread", incident.effect?.thread ?? undefined)}
          >
            Open in Gmail
          </button>
        </li>
      )}
      <li>
        <span>
          <span className="strong">Review {agent}’s Google access</span>
          <span className="caption">
            Remove it in your Google account if you no longer trust it.
          </span>
        </span>
        <button
          type="button"
          className="button small"
          onClick={() => void openLink("google_access")}
        >
          Open Google settings
        </button>
      </li>
      {agentId && (
        <li>
          <span>
            <span className="strong">Mark {agent} as revoked</span>
            <span className="caption">Anything {agent} does after now is flagged as urgent.</span>
          </span>
          <button
            type="button"
            className="button small"
            disabled={revoke.isPending || revoke.isSuccess}
            onClick={() => revoke.mutate()}
          >
            {revoke.isSuccess ? "Marked revoked" : "Mark revoked"}
          </button>
        </li>
      )}
    </ul>
  );
}

export function Incident({ id }: { id: string }) {
  const client = useQueryClient();
  const [error, setError] = useState<unknown>(null);
  const incident = useQuery({
    queryKey: ["incident", id],
    queryFn: () => getIncident(id),
    refetchInterval: 10_000,
  });
  const agents = useQuery({ queryKey: ["agents"], queryFn: listAgents });
  const decide = useMutation({
    mutationFn: (decision: "fine" | "not_ok" | "undo") => resolveIncident(id, decision),
    onSuccess: () => {
      setError(null);
      return client.invalidateQueries();
    },
    onError: setError,
  });
  if (incident.isSuccess && incident.data === null) return <DesktopOnly />;
  if (incident.isError) {
    return (
      <Empty title="This item isn’t available">
        It may have been removed with old history.{" "}
        <a href={href({ page: "review" })}>Back to Review</a>
      </Empty>
    );
  }
  const data = incident.data;
  if (!data) return null;
  // With one watched agent, name it even when Picket can't tell who acted.
  const watched = agents.data?.agents.filter((agent) => agent.enabled) ?? [];
  const agentName =
    data.agent ??
    data.actor.agents[0] ??
    (watched.length === 1 ? watched[0]?.name : undefined) ??
    "your agent";
  const agentId = agents.data?.agents.find((agent) => agent.name === agentName)?.id ?? null;
  const open = data.status === "open";
  return (
    <div className="stack">
      <a className="back" href={href({ page: "review" })}>
        <Icon name="back" size={14} />
        Review
      </a>
      <header className="incident-header">
        <div className="meta">
          {data.status === "fine" ? (
            <Badge tone="ok">Marked as fine</Badge>
          ) : data.status === "not_ok" ? (
            <Badge tone="muted">You said this wasn’t OK</Badge>
          ) : (
            <Badge tone={data.severity === "urgent" ? "urgent" : "review"}>
              {data.severity === "urgent" ? "Urgent" : "Review"}
            </Badge>
          )}
          <span>{kindLabel[data.kind]}</span>
          <span aria-hidden="true">·</span>
          <span>{when(data.occurredAt)}</span>
        </div>
        <h1>{incidentTitle(data)}</h1>
        <p className="lede">{lede(data, agentName)}</p>
      </header>

      {(data.kind === "email" || data.kind === "calendar" || data.kind === "report") && (
        <section className="story" aria-label="Request, report and what happened">
          <RequestColumn incident={data} agent={agentName} />
          <ReportColumn incident={data} agent={agentName} />
          <EffectColumn incident={data} />
        </section>
      )}

      {error !== null && <Failure error={error} />}
      {open ? (
        <section className="card decision" aria-labelledby="ok-heading">
          <div>
            <h2 id="ok-heading">Was this OK?</h2>
            <p className="muted">
              Your answer applies to this item only. Your rules stay as they are.
            </p>
          </div>
          <div className="actions">
            <button
              type="button"
              className="button"
              disabled={decide.isPending}
              onClick={() => decide.mutate("fine")}
            >
              This was fine
            </button>
            <button
              type="button"
              className="button primary"
              disabled={decide.isPending}
              onClick={() => decide.mutate("not_ok")}
            >
              This wasn’t OK
            </button>
          </div>
        </section>
      ) : data.status === "fine" ? (
        <section className="card decision decision-done">
          <Icon name="check" />
          <p>
            Marked as fine. Picket won’t treat this as approved next time; to allow it, change your
            rules.
          </p>
          <button type="button" className="button small" onClick={() => decide.mutate("undo")}>
            Undo
          </button>
        </section>
      ) : (
        <section className="card panel" aria-labelledby="next-heading">
          <div className="panel-head">
            <h2 id="next-heading" className="panel-title">
              What you can do now
            </h2>
            <button type="button" className="button small" onClick={() => decide.mutate("undo")}>
              Undo
            </button>
          </div>
          <p className="muted">
            Picket only watches. It can’t undo anything or change {agentName}’s access for you.
          </p>
          <NextSteps incident={data} agentId={agentId} agent={agentName} />
        </section>
      )}

      <div className="columns">
        <Why incident={data} />
        <Certainty incident={data} agent={agentName} />
      </div>

      <section aria-labelledby="history-heading">
        <h2 id="history-heading" className="label section-label">
          History
        </h2>
        <ol className="history">
          {data.history.map((item) => (
            <li key={`${item.kind}-${item.at}`}>
              <span className="row-time">{when(item.at)}</span>
              <span>{historyText[item.kind](agentName)}</span>
            </li>
          ))}
        </ol>
      </section>
      <ExportReport id={data.id} />
    </div>
  );
}
