import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { App } from "./App";
import { connectGoogle, getGoogleStatus, getRuntimeInfo } from "./lib/runtime";

vi.mock("./lib/runtime", () => ({
  getRuntimeInfo: vi.fn(),
  getGoogleStatus: vi.fn(),
  connectGoogle: vi.fn(),
}));
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
  getGoogle.mockResolvedValue(null);
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
