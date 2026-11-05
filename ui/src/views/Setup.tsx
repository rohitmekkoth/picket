import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, type ReactNode, useState } from "react";
import { Failure, Icon, Mark } from "../components/ui";
import {
  completeSetup,
  getSettings,
  openLink,
  type SettingsPatch,
  setStartAtLogin,
  updateSettings,
} from "../lib/api";
import { minutesToTime } from "../lib/format";
import { navigate, type SetupStep, setupSteps } from "../lib/router";
import {
  connectGoogle,
  downloadModel,
  getGoogleStatus,
  getModel,
  listAgents,
  registerAgent,
  signInGoogle,
} from "../lib/runtime";

// The first-run wizard. Each step reads the real state (signed in, agent added, access
// granted), so it can be left and resumed at any point, and every step can be skipped.

const stepTitles: Record<SetupStep, string> = {
  welcome: "Welcome",
  google: "Google",
  agent: "Your agent",
  access: "Messages",
  alerts: "Alerts",
  finish: "Finish",
};

function go(step: SetupStep) {
  navigate({ page: "setup", step });
}

function Done({ children }: { children: ReactNode }) {
  return (
    <p className="setup-done" role="status">
      <Icon name="check" />
      <span>{children}</span>
    </p>
  );
}

function Welcome() {
  return (
    <>
      <h1>Picket keeps an independent record of what your AI agent does</h1>
      <p className="lede">
        When your agent sends an email or changes your calendar in a way you didn’t ask for, Picket
        tells you.
      </p>
      <ul className="setup-facts">
        <li>
          <span className="strong">What Picket sees</span>
          <span className="caption">
            Who your emails are to and from, their subjects and times; your calendar events; and
            your iMessage chat with your agent.
          </span>
        </li>
        <li>
          <span className="strong">What it can never do</span>
          <span className="caption">
            Read email bodies, or send, change or delete anything. Picket only watches.
          </span>
        </li>
        <li>
          <span className="strong">Where your data lives</span>
          <span className="caption">
            Only on this Mac. There is no Picket server, account or tracking, and you can delete
            everything from Settings.
          </span>
        </li>
      </ul>
    </>
  );
}

function Google({ picketClient }: { picketClient: boolean }) {
  const client = useQueryClient();
  const google = useQuery({
    queryKey: ["google-status"],
    queryFn: getGoogleStatus,
    refetchInterval: 3000,
  });
  const [own, setOwn] = useState(!picketClient);
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const signIn = useMutation({
    mutationFn: () => signInGoogle(),
    onSuccess: () => client.invalidateQueries(),
  });
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
  const status = google.data;
  const connected = status === "monitoring" || status === "starting" || status === "degraded";
  return (
    <>
      <h1>Connect your Google account</h1>
      <p className="lede">
        Picket asks Google for read-only access to email details and your main calendar.
      </p>
      <ul className="setup-facts">
        <li>
          <span className="strong">Gmail: who, when and subject</span>
          <span className="caption">Not message bodies or attachments.</span>
        </li>
        <li>
          <span className="strong">Calendar: your events</span>
          <span className="caption">Read only. Picket can’t create or change anything.</span>
        </li>
      </ul>
      {connected ? (
        <Done>Connected. Picket is reading the last few weeks to learn who you usually email.</Done>
      ) : (
        <div className="stack setup-actions">
          {picketClient && (
            <div>
              <button
                type="button"
                className="button primary"
                disabled={signIn.isPending}
                onClick={() => signIn.mutate()}
              >
                {signIn.isPending ? "Waiting for Google…" : "Sign in with Google"}
              </button>
              <p className="caption setup-note">
                Your browser opens Google’s sign-in. Google hasn’t reviewed Picket yet, so it shows
                a warning: choose Advanced, then Go to Picket.
              </p>
              {signIn.isError && <Failure error={signIn.error} />}
            </div>
          )}
          {picketClient && !own && (
            <button type="button" className="link-button" onClick={() => setOwn(true)}>
              Use your own Google Cloud client instead
            </button>
          )}
          {own && (
            <form className="setting-form setup-form" onSubmit={onSubmit}>
              <label htmlFor="setup-client-id">Google Desktop OAuth client ID</label>
              <input
                id="setup-client-id"
                autoComplete="off"
                value={clientId}
                onChange={(event) => setClientId(event.target.value)}
                required
              />
              <label htmlFor="setup-client-secret">Client secret, if supplied</label>
              <input
                id="setup-client-secret"
                type="password"
                autoComplete="off"
                value={clientSecret}
                onChange={(event) => setClientSecret(event.target.value)}
              />
              <div>
                <button type="submit" className="button" disabled={connect.isPending}>
                  {connect.isPending ? "Waiting for Google…" : "Sign in with this client"}
                </button>
              </div>
              {connect.isError && <Failure error={connect.error} />}
            </form>
          )}
        </div>
      )}
    </>
  );
}

function Agent() {
  const client = useQueryClient();
  const agents = useQuery({ queryKey: ["agents"], queryFn: listAgents, refetchInterval: 3000 });
  const [name, setName] = useState("Instinct");
  const [handle, setHandle] = useState("");
  const add = useMutation({
    mutationFn: () => registerAgent(name.trim(), "messages", handle.trim()),
    onSuccess: () => client.invalidateQueries(),
  });
  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    add.mutate();
  }
  const existing = agents.data?.agents.filter((agent) => agent.enabled) ?? [];
  return (
    <>
      <h1>Which agent should Picket watch?</h1>
      <p className="lede">
        Picket reads what you ask your agent in iMessage, and compares it with what then happens in
        Gmail and Calendar.
      </p>
      {existing.length > 0 ? (
        <Done>
          Watching {existing.map((agent) => agent.name).join(", ")}. You can add more in Settings.
        </Done>
      ) : (
        <form className="setting-form setup-form" onSubmit={onSubmit}>
          <label htmlFor="setup-agent-name">Agent name</label>
          <input
            id="setup-agent-name"
            autoComplete="off"
            value={name}
            onChange={(event) => setName(event.target.value)}
            required
          />
          <label htmlFor="setup-agent-handle">Its phone number or Apple ID email in iMessage</label>
          <input
            id="setup-agent-handle"
            autoComplete="off"
            value={handle}
            onChange={(event) => setHandle(event.target.value)}
            required
          />
          <div>
            <button type="submit" className="button primary" disabled={add.isPending}>
              {add.isPending ? "Saving…" : "Add agent"}
            </button>
          </div>
          {add.isError && <Failure error={add.error} />}
        </form>
      )}
    </>
  );
}

function Access() {
  const agents = useQuery({ queryKey: ["agents"], queryFn: listAgents, refetchInterval: 3000 });
  const state = agents.data?.messages;
  const name = agents.data?.agents.find((agent) => agent.mode === "messages")?.name ?? "your agent";
  return (
    <>
      <h1>Let Picket read your chat with {name}</h1>
      <p className="lede">
        macOS keeps Messages private, so Picket needs Full Disk Access to read it. Picket opens the
        Messages database read-only and keeps only your chat with {name}.
      </p>
      {state === "capturing" ? (
        <Done>Picket can read your chat with {name}.</Done>
      ) : state === "inactive" ? (
        <p className="muted">Add an iMessage agent first, or skip this step.</p>
      ) : state === "waiting" ? (
        <Done>Access is ready. Picket starts reading once Google is connected.</Done>
      ) : (
        <div className="stack setup-actions">
          <ol className="setup-steps">
            <li>Open System Settings, Privacy & Security, Full Disk Access.</li>
            <li>Turn on Picket. If it isn’t listed, add it with the + button.</li>
            <li>Come back here. Picket notices on its own.</li>
          </ol>
          <div>
            <button
              type="button"
              className="button primary"
              onClick={() => void openLink("full_disk_access")}
            >
              Open System Settings
            </button>
          </div>
          <p className="caption" role="status">
            {state === "permission_missing" ? "Waiting for access…" : "Checking…"}
          </p>
        </div>
      )}
    </>
  );
}

const summaryTimes = Array.from({ length: 36 }, (_, index) => 6 * 60 + index * 30);

function Alerts() {
  const client = useQueryClient();
  const settings = useQuery({ queryKey: ["settings"], queryFn: getSettings });
  const save = useMutation({
    mutationFn: (patch: SettingsPatch) => updateSettings(patch),
    onSuccess: (next) => client.setQueryData(["settings"], next),
  });
  const login = useMutation({
    mutationFn: (enabled: boolean) => setStartAtLogin(enabled),
    onSuccess: () => client.invalidateQueries({ queryKey: ["settings"] }),
  });
  const data = settings.data;
  return (
    <>
      <h1>How Picket tells you</h1>
      <p className="lede">
        When something needs a look, Picket sends a notification. Urgent items arrive right away;
        others are grouped, at most every 10 minutes.
      </p>
      {data && (
        <div className="card list">
          <div className="setting">
            <span className="setting-text">
              <label className="strong" htmlFor="setup-login">
                Open at login
              </label>
              <span className="caption">
                Picket waits in the menu bar, so it doesn’t miss anything while you work.
              </span>
            </span>
            <input
              id="setup-login"
              type="checkbox"
              className="setting-check"
              checked={data.startAtLogin}
              disabled={login.isPending}
              onChange={(event) => login.mutate(event.target.checked)}
            />
          </div>
          <div className="setting">
            <span className="setting-text">
              <label className="strong" htmlFor="setup-summary">
                Daily summary
              </label>
              <span className="caption">What your agent did that day, in one notification.</span>
            </span>
            <select
              id="setup-summary"
              className="setting-control"
              value={data.summaryMinutes ?? "off"}
              disabled={save.isPending}
              onChange={(event) =>
                save.mutate({
                  summaryMinutes: event.target.value === "off" ? null : Number(event.target.value),
                })
              }
            >
              <option value="off">Off</option>
              {summaryTimes.map((minutes) => (
                <option key={minutes} value={minutes}>
                  {minutesToTime(minutes)}
                </option>
              ))}
            </select>
          </div>
          <div className="setting">
            <span className="setting-text">
              <span className="strong">Banners</span>
              <span className="caption">
                For alerts that appear on screen, set Picket to Banners or Alerts in System
                Settings, Notifications.
              </span>
            </span>
            <button
              type="button"
              className="button small"
              onClick={() => void openLink("notification_settings")}
            >
              Open Notifications
            </button>
          </div>
        </div>
      )}
      {(save.isError || login.isError) && <Failure error={save.error ?? login.error} />}
    </>
  );
}

function Finish() {
  const client = useQueryClient();
  const model = useQuery({ queryKey: ["model"], queryFn: getModel, refetchInterval: 2000 });
  const download = useMutation({
    mutationFn: () => downloadModel(),
    onSuccess: () => client.invalidateQueries({ queryKey: ["model"] }),
  });
  const state = model.data?.status;
  const size = model.data ? `${(model.data.bytes / 1e9).toFixed(1)} GB` : "";
  return (
    <>
      <h1>One last thing: the on-device model</h1>
      <p className="lede">
        To understand what you ask your agent, Picket uses a language model that runs only on this
        Mac. Nothing you write is sent anywhere.
      </p>
      {state?.state === "ready" ? (
        <Done>The model is ready.</Done>
      ) : state?.state === "downloading" ? (
        <div className="stack setup-actions">
          <progress
            className="setup-progress"
            max={state.total || 1}
            value={state.received}
            aria-label="Model download"
          />
          <p className="caption" role="status">
            {state.total > 0
              ? `Downloading ${(state.received / 1e9).toFixed(1)} of ${(state.total / 1e9).toFixed(1)} GB. You can finish now; it continues in the background.`
              : "Starting download."}
          </p>
        </div>
      ) : (
        <div className="stack setup-actions">
          <div>
            <button
              type="button"
              className="button primary"
              disabled={download.isPending || !model.data}
              onClick={() => download.mutate()}
            >
              Download model{size && ` (${size})`}
            </button>
          </div>
          <p className="caption">
            {state?.state === "failed"
              ? `The download failed: ${state.reason}. Try again.`
              : "Until it’s downloaded, Picket watches Gmail and Calendar but can’t read what you asked."}
          </p>
          {download.isError && <Failure error={download.error} />}
        </div>
      )}
    </>
  );
}

export function Setup({ step }: { step: SetupStep }) {
  const client = useQueryClient();
  const settings = useQuery({ queryKey: ["settings"], queryFn: getSettings });
  const agents = useQuery({ queryKey: ["agents"], queryFn: listAgents });
  const finish = useMutation({
    mutationFn: () => completeSetup(),
    onSuccess: (next) => {
      client.setQueryData(["settings"], next);
      navigate({ page: "overview" });
    },
  });
  // Messages access only matters for an iMessage agent.
  const needsAccess = agents.data?.agents.some((agent) => agent.mode === "messages") ?? true;
  const steps = setupSteps.filter((item) => item !== "access" || needsAccess);
  const index = Math.max(0, steps.indexOf(step));
  const current = steps[index] ?? "welcome";
  const next = steps[index + 1];
  const previous = steps[index - 1];
  return (
    <div className="setup">
      <header className="setup-head">
        <span className="brand">
          <Mark />
          <span>Picket</span>
        </span>
        <ol className="setup-progress-list" aria-label="Setup steps">
          {steps.map((item, position) => (
            <li
              key={item}
              className={position === index ? "on" : position < index ? "done" : undefined}
              aria-current={position === index ? "step" : undefined}
            >
              {stepTitles[item]}
            </li>
          ))}
        </ol>
        <button
          type="button"
          className="link-button"
          disabled={finish.isPending}
          onClick={() => finish.mutate()}
        >
          Skip setup
        </button>
      </header>
      <main className="setup-body" id="content">
        <section className="setup-card stack" aria-label={stepTitles[current]}>
          <p className="eyebrow">
            Step {index + 1} of {steps.length}
          </p>
          {current === "welcome" && <Welcome />}
          {current === "google" && <Google picketClient={settings.data?.picketClient ?? false} />}
          {current === "agent" && <Agent />}
          {current === "access" && <Access />}
          {current === "alerts" && <Alerts />}
          {current === "finish" && <Finish />}
          {finish.isError && <Failure error={finish.error} />}
        </section>
        <footer className="setup-foot">
          {previous ? (
            <button type="button" className="button" onClick={() => go(previous)}>
              Back
            </button>
          ) : (
            <span />
          )}
          {next ? (
            <button type="button" className="button primary" onClick={() => go(next)}>
              {current === "welcome" ? "Get started" : "Continue"}
            </button>
          ) : (
            <button
              type="button"
              className="button primary"
              disabled={finish.isPending}
              onClick={() => finish.mutate()}
            >
              Finish setup
            </button>
          )}
        </footer>
      </main>
    </div>
  );
}
