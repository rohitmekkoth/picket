import { useInfiniteQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useAgentName } from "../App";
import { Badge, DesktopOnly, Empty, Icon, type IconName, Segmented } from "../components/ui";
import { type ActivityItem, listActivity } from "../lib/api";
import { activityLine, verdictLabel } from "../lib/copy";
import { clock, dayHeading, period } from "../lib/format";
import { href } from "../lib/router";

type Who = "everyone" | "agents" | "not_linked";
type Result = "any" | "review" | "as_asked" | "unclear";

const icons: Record<ActivityItem["kind"], IconName> = {
  email: "mail",
  calendar: "calendar",
  request: "chat",
  report: "chat",
  gap: "moon",
  recovered: "check",
};

function detail(item: ActivityItem, agent: string): string {
  if (item.kind === "request") return "Your chat";
  if (item.kind === "report") {
    return item.outsideScope
      ? `From ${item.agent ?? agent}’s own address, which Picket can’t see`
      : `${item.agent ?? agent}’s reply`;
  }
  if (item.kind === "gap" || item.kind === "recovered") {
    return item.gapReason === "auth_unavailable"
      ? "Google sign-in needed"
      : "Picket will say what it couldn’t see";
  }
  if (item.agent) return `${item.agent}, by its report`;
  return "Not linked to an agent";
}

function Row({ item, agent }: { item: ActivityItem; agent: string }) {
  const tone = item.verdict === "review" ? "review" : item.verdict === "as_asked" ? "ok" : "muted";
  const body = (
    <>
      <span className="row-time">{clock(item.at)}</span>
      <span className="icon-tile" aria-hidden="true">
        <Icon name={icons[item.kind]} size={16} />
      </span>
      <span className="row-main">
        <span>{activityLine(item)}</span>
        <span className="row-detail">{detail(item, agent)}</span>
      </span>
      {verdictLabel[item.verdict] && <Badge tone={tone}>{verdictLabel[item.verdict]}</Badge>}
    </>
  );
  if (item.incident) {
    return (
      <li>
        <a className="row-link" href={href({ page: "incident", id: item.incident })}>
          {body}
          <Icon name="chevron" size={16} />
        </a>
      </li>
    );
  }
  return (
    <li className={item.kind === "gap" ? "row gap-row" : "row"}>
      {body}
      <span className="chevron-space" aria-hidden="true" />
    </li>
  );
}

export function Activity() {
  const agent = useAgentName() ?? "your agent";
  const [who, setWho] = useState<Who>("everyone");
  const [result, setResult] = useState<Result>("any");
  // The list covers the last 30 days; the period is fixed when the screen opens.
  const [span] = useState(() => period("30d"));
  const activity = useInfiniteQuery({
    queryKey: ["activity", who, result, span.from],
    queryFn: ({ pageParam }) => listActivity(span, who, result, pageParam),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last?.next ?? null,
  });
  if (activity.isSuccess && activity.data.pages[0] === null) return <DesktopOnly />;
  const items = activity.data?.pages.flatMap((page) => page?.items ?? []) ?? [];
  const days: { heading: string; items: ActivityItem[] }[] = [];
  for (const item of items) {
    const heading = dayHeading(item.at);
    const last = days[days.length - 1];
    if (last?.heading === heading) last.items.push(item);
    else days.push({ heading, items: [item] });
  }
  return (
    <div className="stack">
      <header className="page-header">
        <div>
          <h1>Activity</h1>
          <p className="lede">
            Everything Picket saw in Gmail, Calendar and your chat with {agent}, for the last 30
            days.
          </p>
        </div>
      </header>
      <div className="filters">
        <Segmented
          label="Who"
          value={who}
          options={[
            { value: "everyone", label: "Everyone" },
            { value: "agents", label: agent.charAt(0).toUpperCase() + agent.slice(1) },
            { value: "not_linked", label: "Not linked" },
          ]}
          onChange={setWho}
        />
        <Segmented
          label="Result"
          value={result}
          options={[
            { value: "any", label: "Any result" },
            { value: "review", label: "Review" },
            { value: "as_asked", label: "As asked" },
            { value: "unclear", label: "Couldn’t tell" },
          ]}
          onChange={setResult}
        />
      </div>
      {activity.isSuccess && items.length === 0 ? (
        <Empty title="Nothing to show">
          {who === "everyone" && result === "any"
            ? "Activity appears here as Picket sees it."
            : "Nothing matches these filters."}
        </Empty>
      ) : (
        days.map((day) => (
          <section key={day.heading} aria-label={day.heading}>
            <h2 className="label section-label">{day.heading}</h2>
            <ul className="card list plain-list">
              {day.items.map((item) => (
                <Row key={item.id} item={item} agent={agent} />
              ))}
            </ul>
          </section>
        ))
      )}
      {activity.hasNextPage && (
        <button
          type="button"
          className="button more-button"
          disabled={activity.isFetchingNextPage}
          onClick={() => void activity.fetchNextPage()}
        >
          {activity.isFetchingNextPage ? "Loading" : "Show more"}
        </button>
      )}
    </div>
  );
}
