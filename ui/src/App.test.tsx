import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { App } from "./App";
import { getRuntimeInfo } from "./lib/runtime";

vi.mock("./lib/runtime", () => ({ getRuntimeInfo: vi.fn() }));
const getRuntime = vi.mocked(getRuntimeInfo);

function renderApp() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  render(
    <QueryClientProvider client={client}>
      <App />
    </QueryClientProvider>,
  );
}

beforeEach(() => getRuntime.mockReset());

test("browser preview does not pretend the Rust core is connected", async () => {
  getRuntime.mockResolvedValue(null);
  renderApp();
  expect(await screen.findByRole("heading", { name: "Browser preview" })).toBeInTheDocument();
  expect(screen.getByText(/Monitoring is not implemented yet/)).toBeInTheDocument();
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
