import { useQuery } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { Icon, type IconName, Mark } from "./components/ui";
import { listIncidents } from "./lib/api";
import { href, type Route, useRoute } from "./lib/router";
import { listAgents } from "./lib/runtime";
import { Activity } from "./views/Activity";
import { BoundaryEditor } from "./views/BoundaryEditor";
import { Incident } from "./views/Incident";
import { NotificationTest } from "./views/NotificationTest";
import { Overview } from "./views/Overview";
import { Review } from "./views/Review";
import { Rules } from "./views/Rules";
import { Settings } from "./views/Settings";

const sections: { page: Route["page"]; label: string; icon: IconName }[] = [
  { page: "overview", label: "Overview", icon: "home" },
  { page: "review", label: "Review", icon: "review" },
  { page: "activity", label: "Activity", icon: "activity" },
  { page: "rules", label: "Rules", icon: "rules" },
  { page: "settings", label: "Settings", icon: "settings" },
];

/** Which sidebar entry a page belongs to. */
function section(route: Route): Route["page"] {
  if (route.page === "incident") return "review";
  if (route.page === "boundary") return "rules";
  if (route.page === "notification-test") return "settings";
  return route.page;
}

/** The agent Picket watches, for wording such as "Instinct did 10 things". */
export function useAgentName(): string | null {
  const agents = useQuery({ queryKey: ["agents"], queryFn: listAgents, refetchInterval: 5000 });
  const watched = agents.data?.agents.filter((agent) => agent.enabled) ?? [];
  return watched.length === 1 ? (watched[0]?.name ?? null) : null;
}

function Sidebar({ current }: { current: Route["page"] }) {
  const incidents = useQuery({
    queryKey: ["incidents"],
    queryFn: listIncidents,
    refetchInterval: 5000,
  });
  const agents = useQuery({ queryKey: ["agents"], queryFn: listAgents, refetchInterval: 5000 });
  const open = incidents.data?.filter((incident) => incident.status === "open").length ?? 0;
  const watched = agents.data?.agents.filter((agent) => agent.enabled) ?? [];
  return (
    <nav className="sidebar" aria-label="Picket">
      <a className="brand" href={href({ page: "overview" })} aria-label="Picket overview">
        <Mark />
        <span>Picket</span>
      </a>
      <ul className="nav-list">
        {sections.map((item) => (
          <li key={item.page}>
            <a
              className={current === item.page ? "nav on" : "nav"}
              aria-current={current === item.page ? "page" : undefined}
              href={href({ page: item.page } as Route)}
            >
              <Icon name={item.icon} />
              <span>{item.label}</span>
              {item.page === "review" && open > 0 && (
                <span className="count">
                  {open}
                  <span className="visually-hidden"> open</span>
                </span>
              )}
            </a>
          </li>
        ))}
      </ul>
      {watched.length > 0 && (
        <div className="watching">
          <p className="label">Watching</p>
          {watched.map((agent) => (
            <div key={agent.id} className="agent-chip">
              <span className="avatar" aria-hidden="true">
                {agent.name.slice(0, 2)}
              </span>
              <span>
                <span className="agent-name">{agent.name}</span>
                <span className="agent-mode">
                  {agent.mode === "messages" ? "iMessage agent" : "Effects only"}
                </span>
              </span>
            </div>
          ))}
        </div>
      )}
    </nav>
  );
}

function Page({ route }: { route: Route }): ReactNode {
  switch (route.page) {
    case "overview":
      return <Overview />;
    case "review":
      return <Review />;
    case "incident":
      return <Incident id={route.id} />;
    case "activity":
      return <Activity />;
    case "rules":
      return <Rules />;
    case "boundary":
      return <BoundaryEditor id={route.id} />;
    case "settings":
      return <Settings />;
    case "notification-test":
      return <NotificationTest action={route.action} />;
  }
}

export function App() {
  const route = useRoute();
  return (
    <div className="app">
      <Sidebar current={section(route)} />
      <main className="content" id="content">
        <div className="page">
          <Page route={route} />
        </div>
      </main>
    </div>
  );
}
