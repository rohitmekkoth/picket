import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { DesktopOnly, Failure } from "../components/ui";
import { openLink, setAgentRevoked } from "../lib/api";
import {
  type AgentMode,
  type Agents,
  connectGoogle,
  downloadModel,
  getGoogleStatus,
  getModel,
  getRuntimeInfo,
  listAgents,
  type Model,
  registerAgent,
} from "../lib/runtime";

const messagesStatus: Record<Agents["messages"], string> = {
  inactive: "No iMessage agent is registered.",
  waiting: "Connect Google first. Messages are recorded with the monitored account.",
  capturing: "Reading your conversation with the agent.",
  permission_missing:
    "Picket needs Full Disk Access to read your chat. Turn it on in System Settings; Picket notices on its own.",
  unsupported: "This Messages database format is not supported, so reading it is off.",
  unavailable: "Messages could not be read. Picket will retry.",
};

const googleStatus: Record<string, string> = {
  monitoring: "Connected",
  starting: "Connecting",
  degraded: "Having trouble reaching Google",
  unavailable: "Sign in again",
  disconnected: "Not connected",
};

function gigabytes(bytes: number): string {
  return `${(bytes / 1e9).toFixed(1)} GB`;
}

function modelText(model: Model): string {
  switch (model.status.state) {
    case "absent":
      return "Not downloaded. Until it is, Picket watches Gmail and Calendar but can’t read what you asked.";
    case "downloading":
      return model.status.total > 0
        ? `Downloading ${gigabytes(model.status.received)} of ${gigabytes(model.status.total)}.`
        : "Starting download.";
    case "ready":
      return "Ready. It runs only on this Mac, loads when there are messages to read, and unloads when idle.";
    case "failed":
      return `Download failed: ${model.status.reason}. Try again.`;
  }
}

function GoogleSection() {
  const client = useQueryClient();
  const google = useQuery({
    queryKey: ["google-status"],
    queryFn: getGoogleStatus,
    refetchInterval: 5000,
  });
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const connect = useMutation({
    mutationFn: () => connectGoogle(clientId.trim(), clientSecret),
    onSuccess: () => {
      setClientSecret("");
      return client.invalidateQueries();
    },
  });
  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    connect.mutate();
  }
  const status = google.data ?? "disconnected";
  const connected = status === "monitoring" || status === "starting";
  return (
    <section className="settings-group" aria-labelledby="google-heading">
      <h2 id="google-heading">Google account</h2>
      <div className="card list">
        <div className="setting">
          <span className="setting-text">
            <span className="strong">Gmail and your main calendar</span>
            <span className="caption">
              Who your emails are to and from, when, and their subject, plus your calendar events.
              Picket can’t read message bodies, or send, change or delete anything.
            </span>
          </span>
          <span className={connected ? "state state-ok" : "state"} role="status">
            {googleStatus[status] ?? status}
          </span>
        </div>
        {!connected && (
          <form className="setting setting-form" onSubmit={onSubmit}>
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
            <div>
              <button type="submit" className="button primary" disabled={connect.isPending}>
                {connect.isPending
                  ? "Waiting for Google…"
                  : status === "disconnected"
                    ? "Connect Google"
                    : "Sign in again"}
              </button>
            </div>
            {connect.isError && <Failure error={connect.error} />}
          </form>
        )}
      </div>
    </section>
  );
}

function AgentSection() {
  const client = useQueryClient();
  const agents = useQuery({ queryKey: ["agents"], queryFn: listAgents, refetchInterval: 5000 });
  const [name, setName] = useState("");
  const [handle, setHandle] = useState("");
  const [mode, setMode] = useState<AgentMode>("messages");
  const [adding, setAdding] = useState(false);
  const add = useMutation({
    mutationFn: () => registerAgent(name.trim(), mode, handle.trim()),
    onSuccess: () => {
      setName("");
      setHandle("");
      setAdding(false);
      return client.invalidateQueries();
    },
  });
  const revoke = useMutation({
    mutationFn: (id: string) => setAgentRevoked(id, true),
    onSuccess: () => client.invalidateQueries(),
  });
  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    add.mutate();
  }
  const data = agents.data;
  return (
    <section className="settings-group" aria-labelledby="agents-heading">
      <h2 id="agents-heading">Agents</h2>
      <div className="card list">
        {data?.agents.map((agent) => (
          <div key={agent.id} className="setting">
            <span className="setting-text">
              <span className="strong">{agent.name}</span>
              <span className="caption">
                {agent.mode === "messages"
                  ? `iMessage agent · ${agent.handles.join(", ")}`
                  : "Effects only: Picket sees what changes in Gmail and Calendar"}
              </span>
            </span>
            <button
              type="button"
              className="button small"
              disabled={revoke.isPending}
              onClick={() => revoke.mutate(agent.id)}
            >
              Mark as revoked
            </button>
          </div>
        ))}
        {data && (
          <div className="setting">
            <span className="setting-text">
              <span className="strong">Messages access</span>
              <span className="caption" role="status">
                {messagesStatus[data.messages]}
              </span>
            </span>
            {data.messages === "permission_missing" && (
              <button
                type="button"
                className="button small"
                onClick={() => void openLink("full_disk_access")}
              >
                Open System Settings
              </button>
            )}
          </div>
        )}
        {data && data.agents.length > 0 && !adding ? (
          <div className="setting">
            <span className="setting-text">
              <span className="caption">Picket can watch more than one agent.</span>
            </span>
            <button type="button" className="button small" onClick={() => setAdding(true)}>
              Add another agent
            </button>
          </div>
        ) : (
          <form className="setting setting-form" onSubmit={onSubmit} aria-label="Add an agent">
            <label htmlFor="agent-name">Agent name</label>
            <input
              id="agent-name"
              autoComplete="off"
              value={name}
              onChange={(event) => setName(event.target.value)}
              required
            />
            <label htmlFor="agent-mode">How you reach it</label>
            <select
              id="agent-mode"
              value={mode}
              onChange={(event) => setMode(event.target.value as AgentMode)}
            >
              <option value="messages">iMessage</option>
              <option value="effects_only">Elsewhere (Gmail and Calendar effects only)</option>
            </select>
            {mode === "messages" && (
              <>
                <label htmlFor="agent-handle">Phone number or Apple ID email</label>
                <input
                  id="agent-handle"
                  autoComplete="off"
                  value={handle}
                  onChange={(event) => setHandle(event.target.value)}
                  required
                />
              </>
            )}
            <div>
              <button type="submit" className="button" disabled={add.isPending}>
                {add.isPending ? "Saving…" : "Add agent"}
              </button>
            </div>
            {add.isError && <Failure error={add.error} />}
          </form>
        )}
      </div>
    </section>
  );
}

function ModelSection() {
  const client = useQueryClient();
  const model = useQuery({ queryKey: ["model"], queryFn: getModel, refetchInterval: 2000 });
  const download = useMutation({
    mutationFn: downloadModel,
    onSuccess: () => client.invalidateQueries({ queryKey: ["model"] }),
  });
  if (!model.data) return null;
  const state = model.data.status.state;
  return (
    <section className="settings-group" aria-labelledby="model-heading">
      <h2 id="model-heading">Understanding your requests</h2>
      <div className="card list">
        <div className="setting">
          <span className="setting-text">
            <span className="strong">On-device model · {gigabytes(model.data.bytes)}</span>
            <span className="caption" role="status">
              {modelText(model.data)}
            </span>
          </span>
          {(state === "absent" || state === "failed") && (
            <button
              type="button"
              className="button small"
              disabled={download.isPending}
              onClick={() => download.mutate()}
            >
              Download model ({gigabytes(model.data.bytes)})
            </button>
          )}
        </div>
        {download.isError && <Failure error={download.error} />}
      </div>
    </section>
  );
}

function AboutSection() {
  const runtime = useQuery({ queryKey: ["runtime"], queryFn: getRuntimeInfo });
  return (
    <section className="settings-group" aria-labelledby="about-heading">
      <h2 id="about-heading">About</h2>
      <div className="card list">
        <div className="setting">
          <span className="setting-text">
            <span className="strong">
              {runtime.isError ? "Core connection unavailable" : "Desktop core connected"}
            </span>
            <span className="caption">
              {runtime.isError
                ? "Picket couldn’t reach its core. Check again, or restart Picket."
                : runtime.data
                  ? `Picket ${runtime.data.version} · ${runtime.data.platform === "macos" ? "macOS" : runtime.data.platform} · ${runtime.data.architecture === "aarch64" ? "Apple silicon" : runtime.data.architecture}`
                  : "Checking"}
            </span>
          </span>
          <button
            type="button"
            className="button small"
            disabled={runtime.isFetching}
            onClick={() => void runtime.refetch()}
          >
            Check connection
          </button>
        </div>
      </div>
    </section>
  );
}

export function Settings() {
  const runtime = useQuery({ queryKey: ["runtime"], queryFn: getRuntimeInfo });
  if (runtime.isSuccess && runtime.data === null) return <DesktopOnly />;
  return (
    <div className="stack stack-narrow">
      <h1>Settings</h1>
      <GoogleSection />
      <AgentSection />
      <ModelSection />
      <AboutSection />
    </div>
  );
}
