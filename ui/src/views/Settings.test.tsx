import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { getSettings, updateSettings, verifyLog } from "../lib/api";
import {
  connectGoogle,
  getGoogleStatus,
  getModel,
  getRuntimeInfo,
  listAgents,
  reconnectGoogle,
  registerAgent,
} from "../lib/runtime";
import { Settings } from "./Settings";

vi.mock("../lib/runtime", () => ({
  getRuntimeInfo: vi.fn(),
  getGoogleStatus: vi.fn(),
  connectGoogle: vi.fn(),
  reconnectGoogle: vi.fn(),
  listAgents: vi.fn(),
  registerAgent: vi.fn(),
  getModel: vi.fn(),
  downloadModel: vi.fn(),
}));
vi.mock("../lib/api", () => ({
  openLink: vi.fn(),
  setAgentRevoked: vi.fn(),
  getSettings: vi.fn(),
  updateSettings: vi.fn(),
  setStartAtLogin: vi.fn(),
  verifyLog: vi.fn(),
  exportAll: vi.fn(),
}));
const settings = vi.mocked(getSettings);
const update = vi.mocked(updateSettings);
const verify = vi.mocked(verifyLog);
const reconnect = vi.mocked(reconnectGoogle);
const defaults = {
  summaryMinutes: 1260,
  retentionDays: 180,
  alertsPausedUntil: null,
  lastSummaryDay: null,
  startAtLogin: true,
  setupCompleted: true,
  picketClient: true,
};
const model = vi.mocked(getModel);
const list = vi.mocked(listAgents);
const register = vi.mocked(registerAgent);
const getRuntime = vi.mocked(getRuntimeInfo);
const getGoogle = vi.mocked(getGoogleStatus);
const connect = vi.mocked(connectGoogle);
const desktop = { version: "0.1.0", platform: "macos", architecture: "aarch64", monitoring: true };

function renderSettings() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  render(
    <QueryClientProvider client={client}>
      <Settings />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.resetAllMocks();
  getRuntime.mockResolvedValue(desktop);
  getGoogle.mockResolvedValue("monitoring");
  list.mockResolvedValue({ agents: [], messages: "inactive" });
  model.mockResolvedValue(null);
  settings.mockResolvedValue(defaults);
});

test("the browser preview does not pretend the core is connected", async () => {
  getRuntime.mockResolvedValue(null);
  renderSettings();
  expect(await screen.findByText("Open the desktop app")).toBeInTheDocument();
});

test("the local model download is offered with its size and progress is shown", async () => {
  model.mockResolvedValue({ status: { state: "absent" }, id: "synthetic", bytes: 2_710_000_000 });
  renderSettings();
  const download = await screen.findByRole("button", { name: "Download model (2.7 GB)" });
  model.mockResolvedValue({
    status: { state: "downloading", received: 1_000_000_000, total: 2_710_000_000 },
    id: "synthetic",
    bytes: 2_710_000_000,
  });
  fireEvent.click(download);
  expect(await screen.findByText("Downloading 1.0 GB of 2.7 GB.")).toBeInTheDocument();
});

test("an iMessage agent can be added and missing access is explained", async () => {
  register.mockResolvedValue();
  renderSettings();
  fireEvent.change(await screen.findByLabelText("Agent name"), {
    target: { value: "Synthetic agent" },
  });
  fireEvent.change(screen.getByLabelText("Phone number or Apple ID email"), {
    target: { value: "+1 415 555 0132" },
  });
  list.mockResolvedValue({
    agents: [
      {
        id: "synthetic-agent",
        name: "Synthetic agent",
        mode: "messages",
        handles: ["+14155550132"],
        enabled: true,
      },
    ],
    messages: "permission_missing",
  });
  fireEvent.click(screen.getByRole("button", { name: "Add agent" }));
  await waitFor(() =>
    expect(register).toHaveBeenCalledWith("Synthetic agent", "messages", "+1 415 555 0132"),
  );
  expect(await screen.findByText("iMessage agent · +14155550132")).toBeInTheDocument();
  expect(screen.getByText(/needs Full Disk Access/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Open System Settings" })).toBeInTheDocument();
});

test("a rejected Google sign-in is renewed with one click", async () => {
  getGoogle.mockResolvedValue("reconnect_required");
  reconnect.mockResolvedValue();
  renderSettings();
  expect(await screen.findByText("Google stopped accepting Picket’s sign-in")).toBeInTheDocument();
  expect(screen.queryByLabelText("Google Desktop OAuth client ID")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Sign in again" }));
  await waitFor(() => expect(reconnect).toHaveBeenCalled());
});

test("an unreachable Google can be retried with a client", async () => {
  getGoogle.mockResolvedValue("unavailable");
  connect.mockResolvedValue();
  renderSettings();
  fireEvent.change(await screen.findByLabelText("Google Desktop OAuth client ID"), {
    target: { value: "synthetic-client" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Try again" }));
  await waitFor(() => expect(connect).toHaveBeenCalledWith("synthetic-client", ""));
});

test("the summary time and history length are saved as validated values", async () => {
  update.mockImplementation(async (patch) => ({ ...defaults, ...patch }));
  renderSettings();
  const summary = await screen.findByLabelText("Daily summary");
  expect(summary).toHaveValue("1260");
  fireEvent.change(summary, { target: { value: "off" } });
  await waitFor(() => expect(update).toHaveBeenCalledWith({ summaryMinutes: null }));
  fireEvent.change(screen.getByLabelText("Keep history for"), { target: { value: "90" } });
  await waitFor(() => expect(update).toHaveBeenCalledWith({ retentionDays: 90 }));
});

test("checking the log reports what it found", async () => {
  verify.mockResolvedValue({
    valid: true,
    throughSequence: 1657,
    retainedEvents: 1657,
    brokenAt: null,
  });
  renderSettings();
  fireEvent.click(await screen.findByRole("button", { name: "Check now" }));
  expect(
    await screen.findByText("The log checks out: 1,657 entries, none changed or missing."),
  ).toBeInTheDocument();
});

test("core failures can recover through the check button", async () => {
  getRuntime.mockRejectedValueOnce(new Error("IPC unavailable"));
  getRuntime.mockResolvedValue(desktop);
  renderSettings();
  expect(await screen.findByText("Core connection unavailable")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Check connection" }));
  expect(await screen.findByText("Desktop core connected")).toBeInTheDocument();
  expect(screen.getByText("Picket 0.1.0 · macOS · Apple silicon")).toBeInTheDocument();
});
