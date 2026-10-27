import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { App } from "./App";
import {
  connectGoogle,
  getGoogleStatus,
  getRuntimeInfo,
  listAgents,
  registerAgent,
} from "./lib/runtime";

vi.mock("./lib/runtime", () => ({
  getRuntimeInfo: vi.fn(),
  getGoogleStatus: vi.fn(),
  connectGoogle: vi.fn(),
  listAgents: vi.fn(),
  registerAgent: vi.fn(),
}));
const list = vi.mocked(listAgents);
const register = vi.mocked(registerAgent);
const getRuntime = vi.mocked(getRuntimeInfo);
const getGoogle = vi.mocked(getGoogleStatus);
const connect = vi.mocked(connectGoogle);

function renderApp() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  render(
    <QueryClientProvider client={client}>
      <App />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  getRuntime.mockReset();
  getGoogle.mockReset();
  connect.mockReset();
  list.mockReset();
  register.mockReset();
  getGoogle.mockResolvedValue(null);
  list.mockResolvedValue(null);
});

test("an iMessage agent can be added and missing access is explained", async () => {
  getRuntime.mockResolvedValue(null);
  list.mockResolvedValue({ agents: [], messages: "inactive" });
  register.mockResolvedValue();
  renderApp();
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
  expect(register).toHaveBeenCalledWith("Synthetic agent", "+1 415 555 0132");
  expect(await screen.findByText("+14155550132")).toBeInTheDocument();
  expect(screen.getByText(/Grant Full Disk Access/)).toBeInTheDocument();
});

test("degraded connection can be reauthorized", async () => {
  getRuntime.mockResolvedValue({
    version: "0.1.0",
    platform: "macos",
    architecture: "aarch64",
    monitoring: false,
  });
  getGoogle.mockResolvedValue("degraded");
  connect.mockResolvedValue();
  renderApp();
  expect(await screen.findByRole("button", { name: "Reconnect Google" })).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Google Desktop OAuth client ID"), {
    target: { value: "synthetic-client" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Reconnect Google" }));
  expect(connect).toHaveBeenCalledWith("synthetic-client", "");
});

test("browser preview does not pretend the Rust core is connected", async () => {
  getRuntime.mockResolvedValue(null);
  renderApp();
  expect(await screen.findByRole("heading", { name: "Browser preview" })).toBeInTheDocument();
  expect(screen.getByText(/Open the desktop app to connect Google/)).toBeInTheDocument();
});

test("runtime failures can recover through the check button", async () => {
  getRuntime.mockRejectedValueOnce(new Error("IPC unavailable"));
  getRuntime.mockResolvedValueOnce({
    version: "0.1.0",
    platform: "macos",
    architecture: "aarch64",
    monitoring: false,
  });
  renderApp();
  expect(
    await screen.findByRole("heading", { name: "Core connection unavailable" }),
  ).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Check connection" }));
  expect(
    await screen.findByRole("heading", { name: "Desktop core connected" }),
  ).toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent("macos / aarch64");
});
