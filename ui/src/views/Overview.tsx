import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useAgentName } from "../App";
import { Badge, DesktopOnly, Empty, Failure, Icon, Segmented } from "../components/ui";
import {
  type Overview as Data,
  getOverview,
  type IncidentSummary,
  listIncidents,
} from "../lib/api";
import { activityLine, incidentTitle, people, quoted, verdictLabel } from "../lib/copy";
import { longDate, period, type Range, rangeWords, when } from "../lib/format";
import { href } from "../lib/router";
import { getModel, listAgents } from "../lib/runtime";

const ranges: { value: Range; label: string }[] = [
  { value: "today", label: "Today" },
  { value: "7d", label: "7 days" },
  { value: "30d", label: "30 days" },
];

function plural(count: number, one: string, many = `${one}s`): string {
  return `${count} ${count === 1 ? one : many}`;
}

export function headline(open: number): string {
  if (open === 0) return "All quiet";
  return `${plural(open, "thing")} ${open === 1 ? "needs" : "need"} a look`;
}

export function sentence(agent: string, counts: Data["counts"], range: Range): string {
  if (counts.agentActions === 0) {
    return `${agent} hasn’t done anything Picket could see ${rangeWords[range]}.`;
  }
  const first = `${agent} did ${plural(counts.agentActions, "thing")} ${rangeWords[range]}.`;
  const details: string[] = [];
  if (counts.asAsked > 0) details.push(`${counts.asAsked} matched what you asked`);
  if (counts.unclear > 0) details.push(`for ${counts.unclear} Picket couldn’t tell`);
  if (details.length === 0) return first;
  const rest = details.join(", and ");
  return `${first} ${rest.charAt(0).toUpperCase()}${rest.slice(1)}.`;
}

export function ReviewRow({ incident }: { incident: IncidentSummary }) {
  const subject = incident.subject ? quoted(incident.subject) : "";
  const to = people(incident.people);
  const detail = [subject, to && `to ${to}`].filter(Boolean).join(" ");
  return (
    <a className="row-link" href={href({ page: "incident", id: incident.id })}>
      {incident.status === "fine" ? (
        <Badge tone="ok">Fine</Badge>
      ) : incident.status === "not_ok" ? (
        <Badge tone="muted">Not OK</Badge>
      ) : (
        <Badge tone={incident.severity === "urgent" ? "urgent" : "review"}>
          {incident.severity === "urgent" ? "Urgent" : "Review"}
        </Badge>
      )}
      <span className="row-main">
        <span className="row-title">{incidentTitle(incident)}</span>
        {detail && <span className="row-detail">{detail}</span>}
      </span>
      <span className="row-time end">{when(incident.occurredAt)}</span>
      <Icon name="chevron" size={16} />
    </a>
  );
}

function sourceLabel(source: string, scope: string, agent: string | null): string {
  if (source === "gmail") return "Gmail";
  if (source === "calendar") return "Calendar";
  if (source === "messages") return `Your chat with ${agent ?? scope}`;
  return source;
}

function Sources({ data, agent }: { data: Data; agent: string | null }) {
  const model = useQuery({ queryKey: ["model"], queryFn: getModel, refetchInterval: 5000 });
  const agents = useQuery({ queryKey: ["agents"], queryFn: listAgents, refetchInterval: 5000 });
  const rows = data.sources.map((source) => {
    const label = sourceLabel(source.source, source.scope, agent);
    let state: "ok" | "warn" | "bad" = "ok";
    let text = "Watching";
    if (source.reason === "auth_unavailable") {
      state = "bad";
      text = "Sign in again";
    } else if (source.reason === "permission_missing") {
      state = "bad";
      text = "Needs Full Disk Access";
    } else if (source.reason === "extraction_unavailable") {
      state = "warn";
      text = "Waiting for the on-device model";
    } else if (source.reason === "undecodable" || source.reason === "unsupported_format") {
      state = "warn";
      text = "Some messages couldn’t be read";
    } else if (source.state === "delayed") {
      state = "warn";
      text = "Catching up";
    } else if (source.state === "gap") {
      state = "bad";
      text = `Missing since ${when(source.since)}`;
    } else if (source.state === "disabled") {
      state = "bad";
      text = "Off";
    }
    return { key: `${source.source}/${source.scope}`, label, state, text };
  });
  if (agents.data?.messages === "permission_missing") {
    rows.push({
      key: "messages-access",
      label: `Your chat with ${agent ?? "your agent"}`,
      state: "bad",
      text: "Needs Full Disk Access",
    });
  }
  if (model.data) {
    const ready = model.data.status.state === "ready";
    rows.push({
      key: "model",
      label: "On-device model",
      state: ready ? "ok" : "warn",
      text: ready
        ? "Ready"
        : model.data.status.state === "downloading"
          ? "Downloading"
          : "Not downloaded",
    });
  }
  const rank = (key: string) =>
    ["gmail", "calendar", "messages", "messages-access", "model"].findIndex((prefix) =>
      key.startsWith(prefix),
    );
  rows.sort((left, right) => rank(left.key) - rank(right.key));
  return (
    <section className="card panel" aria-labelledby="sees-heading">
      <h2 id="sees-heading" className="panel-title">
        What Picket can see
      </h2>
      {rows.length === 0 ? (
        <p className="muted">Nothing is connected yet. Set up Google in Settings.</p>
      ) : (
        <ul className="plain-list">
          {rows.map((row) => (
            <li key={row.key} className="source-row">
              <span className={`status-dot status-${row.state}`} aria-hidden="true" />
              <span className="source-name">{row.label}</span>
              <span className="muted">{row.text}</span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

export function Overview() {
  const [range, setRange] = useState<Range>("today");
  const agent = useAgentName();
  const overview = useQuery({
    queryKey: ["overview", range],
    queryFn: () => getOverview(period(range)),
    refetchInterval: 10_000,
  });
  const incidents = useQuery({
    queryKey: ["incidents"],
    queryFn: listIncidents,
    refetchInterval: 5000,
  });
  if (overview.isSuccess && overview.data === null) return <DesktopOnly />;
  const open = incidents.data?.filter((incident) => incident.status === "open") ?? [];
  const data = overview.data;
  const name = agent ?? "Your agent";
  return (
    <div className="stack">
      <header className="page-header">
        <div>
          <p className="eyebrow">{longDate()}</p>
          <h1>{headline(open.length)}</h1>
          {data && <p className="lede">{sentence(name, data.counts, range)}</p>}
        </div>
        <Segmented label="Period" value={range} options={ranges} onChange={setRange} />
      </header>

      {overview.isError && <Failure error={overview.error} />}
      {open.length > 0 && (
        <section aria-labelledby="review-heading">
          <h2 id="review-heading" className="label section-label">
            Needs your review
          </h2>
          <div className="card list">
            {open.slice(0, 5).map((incident) => (
              <ReviewRow key={incident.id} incident={incident} />
            ))}
          </div>
          {open.length > 5 && (
            <a className="more" href={href({ page: "review" })}>
              See all {open.length}
            </a>
          )}
        </section>
      )}

      {data && (
        <>
          <section className="tiles" aria-label="At a glance">
            <div className="card tile">
              <span className="tile-number tone-ok">{data.counts.asAsked}</span>
              <span className="tile-label">As you asked</span>
            </div>
            <div className="card tile">
              <span className="tile-number tone-review">{data.counts.review}</span>
              <span className="tile-label">Didn’t match</span>
            </div>
            <div className="card tile">
              <span className="tile-number">{data.counts.unclear}</span>
              <span className="tile-label">Couldn’t tell</span>
            </div>
            <div className="card tile">
              <span className="tile-number tone-muted">{data.counts.notLinked}</span>
              <span className="tile-label">Not linked to {name}</span>
            </div>
          </section>

          <div className="columns">
            <section className="card panel wide" aria-labelledby="recent-heading">
              <div className="panel-head">
                <h2 id="recent-heading" className="panel-title">
                  {name} {range === "today" ? "today" : "lately"}
                </h2>
                <a className="quiet-link" href={href({ page: "activity" })}>
                  All activity
                </a>
              </div>
              {data.recent.length === 0 ? (
                <Empty title="Nothing yet">
                  When {name} sends an email or changes your calendar, it appears here.
                </Empty>
              ) : (
                <ul className="plain-list">
                  {data.recent.map((item) => (
                    <li key={item.id} className="activity-row">
                      <span className="row-time">{when(item.at)}</span>
                      <span className="row-main">{activityLine(item)}</span>
                      {verdictLabel[item.verdict] && (
                        <Badge
                          tone={
                            item.verdict === "review"
                              ? "review"
                              : item.verdict === "as_asked"
                                ? "ok"
                                : "muted"
                          }
                        >
                          {verdictLabel[item.verdict]}
                        </Badge>
                      )}
                    </li>
                  ))}
                </ul>
              )}
            </section>
            <Sources data={data} agent={agent} />
          </div>
        </>
      )}
    </div>
  );
}
