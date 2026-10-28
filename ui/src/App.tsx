import { useQuery } from "@tanstack/react-query";
import { type FormEvent, useEffect, useState } from "react";
import {
  type AgentMode,
  type Agents,
  connectGoogle,
  getGoogleStatus,
  getRuntimeInfo,
  listAgents,
  registerAgent,
} from "./lib/runtime";

const messagesStatus: Record<Agents["messages"], string> = {
  inactive: "No iMessage agent is registered.",
  waiting: "Connect Google first. Messages are recorded with the monitored account.",
  capturing: "Capturing the registered conversations.",
  permission_missing:
    "Grant Full Disk Access to Picket in System Settings, Privacy & Security. Capture starts automatically.",
  unsupported: "This Messages database format is not supported, so capture is off.",
  unavailable: "Messages could not be read. Picket will retry.",
};

export function App() {
  const [notificationAction, setNotificationAction] = useState(window.location.hash);
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [connecting, setConnecting] = useState(false);
  const [connectionError, setConnectionError] = useState("");
  const [agentName, setAgentName] = useState("");
  const [agentHandle, setAgentHandle] = useState("");
  const [agentMode, setAgentMode] = useState<AgentMode>("messages");
  const [agentError, setAgentError] = useState("");
  const [savingAgent, setSavingAgent] = useState(false);
  useEffect(() => {
    const onHashChange = () => setNotificationAction(window.location.hash);
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, []);
  // The query is the single typed boundary between the renderer and Rust shell.
  const runtime = useQuery({ queryKey: ["runtime"], queryFn: getRuntimeInfo });
  const google = useQuery({
    queryKey: ["google-status"],
    queryFn: getGoogleStatus,
    refetchInterval: 5000,
  });
  const agents = useQuery({ queryKey: ["agents"], queryFn: listAgents, refetchInterval: 5000 });
  async function onAddAgent(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setAgentError("");
    setSavingAgent(true);
    try {
      await registerAgent(agentName.trim(), agentMode, agentHandle.trim());
      setAgentName("");
      setAgentHandle("");
      await agents.refetch();
    } catch (error) {
      setAgentError(error instanceof Error ? error.message : String(error));
    } finally {
      setSavingAgent(false);
    }
  }
  async function onConnect(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setConnectionError("");
    setConnecting(true);
    try {
      await connectGoogle(clientId.trim(), clientSecret);
      setClientSecret("");
      await Promise.all([google.refetch(), runtime.refetch()]);
    } catch (error) {
      setConnectionError(error instanceof Error ? error.message : String(error));
    } finally {
      setConnecting(false);
    }
  }
  let runtimeHeading = "Checking runtime";
  let runtimeDetail = "Waiting for the desktop runtime response.";

  if (runtime.isError) {
    runtimeHeading = "Core connection unavailable";
    runtimeDetail = "The runtime check failed. Retry or inspect the development console.";
  } else if (runtime.data) {
    runtimeHeading = "Desktop core connected";
    runtimeDetail = `v${runtime.data.version} / ${runtime.data.platform} / ${runtime.data.architecture}`;
  } else if (!runtime.isPending) {
    runtimeHeading = "Browser preview";
    runtimeDetail = "Launch the desktop app to check the Rust core connection.";
  }

  return (
    <div className="shell">
      <header className="header">
        <a className="brand" href="#/" aria-label="Picket home">
          <span className="brand-mark" aria-hidden="true">
            p
          </span>
          Picket
        </a>
        <span className="stage">Google observation</span>
      </header>
      <main>
        {notificationAction.startsWith("#/notification-test/") && (
          <section className="runtime notification-result" aria-label="Notification test result">
            <div>
              <p className="eyebrow">Notification test</p>
              <h2>
                {notificationAction.endsWith("/expected")
                  ? "This was expected action received"
                  : "Review action received"}
              </h2>
              <p role="status">
                The native notification opened this window and selected this view.
              </p>
            </div>
          </section>
        )}
        <p className="eyebrow">An independent record</p>
        <h1>
          Your agents act.
          <br />
          Picket keeps watch.
        </h1>
        <p className="intro">
          A private, local record of what you asked, what your agent claimed, and what actually
          changed.
        </p>
        <section className="layers" aria-label="The three layers of an agent action">
          <article>
            <span className="number">01</span>
            <h2>Request</h2>
            <p>What you asked your agent to do.</p>
          </article>
          <article>
            <span className="number">02</span>
            <h2>Claim</h2>
            <p>What your agent said it did.</p>
          </article>
          <article>
            <span className="number">03</span>
            <h2>Effect</h2>
            <p>What changed in your account.</p>
          </article>
        </section>
        <section className="google-panel" aria-labelledby="google-heading">
          <div>
            <p className="eyebrow">Read only sources</p>
            <h2 id="google-heading">Gmail and primary Calendar</h2>
            <p>
              Picket reads Gmail message metadata and primary Calendar events. It stores
              observations locally. It does not read email bodies or attachments, or change your
              account.
            </p>
          </div>
          {google.data === "monitoring" || google.data === "starting" ? (
            <p className="connection-state" role="status">
              Connection: {google.data}
            </p>
          ) : google.data ? (
            <form className="connect-form" onSubmit={(event) => void onConnect(event)}>
              {google.data !== "disconnected" && (
                <p role="status">Connection: {google.data}. Sign in again to recover.</p>
              )}
              <label htmlFor="client-id">Google Desktop OAuth client ID</label>
              <input
                id="client-id"
                autoComplete="off"
                value={clientId}
                onChange={(event) => setClientId(event.target.value)}
                required
              />
              <label htmlFor="client-secret">Client secret, if supplied</label>
              <input
                id="client-secret"
                type="password"
                autoComplete="off"
                value={clientSecret}
                onChange={(event) => setClientSecret(event.target.value)}
              />
              <button type="submit" disabled={connecting}>
                {connecting
                  ? "Waiting for Google..."
                  : google.data === "disconnected"
                    ? "Connect Google"
                    : "Reconnect Google"}
              </button>
              {connectionError && (
                <p className="form-error" role="alert">
                  {connectionError}
                </p>
              )}
            </form>
          ) : (
            <p className="connection-state">Open the desktop app to connect Google.</p>
          )}
        </section>
        <section className="google-panel" aria-labelledby="agents-heading">
          <div>
            <p className="eyebrow">Agents</p>
            <h2 id="agents-heading">Registered agents</h2>
            <p>
              For an iMessage agent, Picket reads only your one-to-one conversation with it, read
              only, and stores direction, time and a short snippet, never the full message. This
              needs Full Disk Access. For an agent you reach elsewhere, such as Meta Muse, Picket
              sees only what changes in Gmail and Calendar.
            </p>
          </div>
          {agents.data ? (
            <form className="connect-form" onSubmit={(event) => void onAddAgent(event)}>
              <p role="status">{messagesStatus[agents.data.messages]}</p>
              {agents.data.agents.length > 0 && (
                <ul className="agent-list" aria-label="Registered agents">
                  {agents.data.agents.map((agent) => (
                    <li key={agent.id}>
                      <span>{agent.name}</span>
                      <span>{agent.handles.join(", ")}</span>
                    </li>
                  ))}
                </ul>
              )}
              <label htmlFor="agent-name">Agent name</label>
              <input
                id="agent-name"
                autoComplete="off"
                value={agentName}
                onChange={(event) => setAgentName(event.target.value)}
                required
              />
              <label htmlFor="agent-mode">How you reach it</label>
              <select
                id="agent-mode"
                value={agentMode}
                onChange={(event) => setAgentMode(event.target.value as AgentMode)}
              >
                <option value="messages">iMessage</option>
                <option value="effects_only">Elsewhere (Gmail and Calendar effects only)</option>
              </select>
              {agentMode === "messages" && (
                <>
                  <label htmlFor="agent-handle">Phone number or Apple ID email</label>
                  <input
                    id="agent-handle"
                    autoComplete="off"
                    value={agentHandle}
                    onChange={(event) => setAgentHandle(event.target.value)}
                    required
                  />
                </>
              )}
              <button type="submit" disabled={savingAgent}>
                {savingAgent ? "Saving..." : "Add agent"}
              </button>
              {agentError && (
                <p className="form-error" role="alert">
                  {agentError}
                </p>
              )}
            </form>
          ) : (
            <p className="connection-state">Open the desktop app to add agents.</p>
          )}
        </section>
        <section className="runtime" aria-labelledby="runtime-heading">
          <div>
            <p className="eyebrow">Workspace status</p>
            <h2 id="runtime-heading">{runtimeHeading}</h2>
            <p role="status">{runtimeDetail}</p>
          </div>
          <button
            type="button"
            onClick={() => void runtime.refetch()}
            disabled={runtime.isFetching}
          >
            Check connection
          </button>
        </section>
        <p className="disclosure">
          Monitoring covers these two Google sources while your Mac is awake. Coverage gaps and
          connection problems limit what Picket can verify. A message marked sent does not prove
          delivery or identify the app that sent it. Initial Gmail history is limited to 50 messages
          each from Sent and Inbox; older contact context may be incomplete.
        </p>
      </main>
      <footer>
        <span>Built to stay on your Mac.</span>
        <span>Local first. Read only. Independent.</span>
      </footer>
    </div>
  );
}
