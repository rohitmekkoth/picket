import { invoke, isTauri } from "@tauri-apps/api/core";
import { expect, test, vi } from "vitest";
import { getRuntimeInfo } from "./runtime";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));

test("browser mode makes no native command calls", async () => {
  vi.mocked(isTauri).mockReturnValue(false);
  expect(await getRuntimeInfo()).toBeNull();
  expect(invoke).not.toHaveBeenCalled();
});

test("malformed IPC results fail instead of displaying a healthy runtime", async () => {
  vi.mocked(isTauri).mockReturnValue(true);
  vi.mocked(invoke).mockResolvedValue({ version: "0.1.0" });
  await expect(getRuntimeInfo()).rejects.toThrow();
});
