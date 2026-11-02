// @vitest-environment node
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createServer, type Server } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, expect, test } from "vitest";
import type { Report } from "./gog.js";
import { QUEUE_LIMIT, Reporter } from "./reporter.js";

const TOKEN = "a".repeat(64);
const send: Report = {
  action: "email_send",
  targets: ["sam@example.test"],
  resource: "m1",
  outcome: "succeeded",
};

let cleanup: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const step of cleanup.reverse()) await step();
  cleanup = [];
});

async function directory(): Promise<string> {
  // Short paths keep the socket under the 104-byte limit.
  const path = await mkdtemp(join(tmpdir().length < 40 ? tmpdir() : "/tmp", "pk-"));
  await writeFile(join(path, "ingest.token"), `${TOKEN}\n`);
  cleanup.push(() => rm(path, { recursive: true, force: true }));
  return path;
}

function listen(path: string): Promise<{ frames: Record<string, unknown>[]; server: Server }> {
  const frames: Record<string, unknown>[] = [];
  const server = createServer((socket) => {
    let buffer = "";
    socket.on("data", (chunk) => {
      buffer += chunk.toString();
      for (let end = buffer.indexOf("\n"); end >= 0; end = buffer.indexOf("\n")) {
        frames.push(JSON.parse(buffer.slice(0, end)));
        buffer = buffer.slice(end + 1);
      }
    });
  });
  cleanup.push(() => new Promise((resolve) => server.close(() => resolve())));
  return new Promise((resolve) =>
    server.listen(join(path, "ingest.sock"), () => resolve({ frames, server })),
  );
}

async function until(condition: () => boolean): Promise<void> {
  for (let i = 0; i < 100 && !condition(); i++) await new Promise((r) => setTimeout(r, 10));
  expect(condition()).toBe(true);
}

test("reports arrive in order with the token, sequence and counters", async () => {
  const path = await directory();
  const { frames } = await listen(path);
  const reporter = new Reporter({ agent: "openclaw", directory: path });
  cleanup.push(() => reporter.stop(0));
  reporter.start();
  reporter.report({ ...send, account: "owner@example.test" });
  reporter.unparsed();
  reporter.report({ ...send, resource: "m2" });
  await until(() => frames.some((frame) => frame.seq === 2));
  const reports = frames.filter((frame) => frame.report !== undefined);
  expect(reports.map((frame) => frame.seq)).toEqual([1, 2]);
  expect(frames.every((frame) => frame.token === TOKEN && frame.agent === "openclaw")).toBe(true);
  expect(reports[1]?.unrecognized).toBe(1);
  // The Rust parser's test reads the same example, so both sides agree on the wire shape.
  const contract = JSON.parse(
    await readFile(new URL("../contract/report-frame.json", import.meta.url), "utf8"),
  ) as Record<string, unknown>;
  expect(shape(reports[0])).toEqual(shape(contract));
  const { report: _, ...heartbeat } = contract;
  expect(shape(frames.find((frame) => frame.report === undefined))).toEqual(shape(heartbeat));
});

function shape(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(shape);
  if (typeof value === "object" && value !== null) {
    return Object.fromEntries(
      Object.entries(value)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, item]) => [key, shape(item)]),
    );
  }
  return typeof value;
}

test("with Picket stopped, reports never throw and overflow is counted as dropped", async () => {
  const path = await directory();
  const reporter = new Reporter({ agent: "openclaw", directory: path });
  cleanup.push(() => reporter.stop(0));
  const started = Date.now();
  for (let i = 0; i < QUEUE_LIMIT + 3; i++) reporter.report({ ...send, resource: `m${i}` });
  expect(Date.now() - started).toBeLessThan(1_000);
  const { frames } = await listen(path);
  await reporter.stop(2_000);
  await until(() => frames.length > QUEUE_LIMIT);
  const reports = frames.filter((frame) => frame.report !== undefined);
  expect(reports.length).toBe(QUEUE_LIMIT);
  expect(reports[0]?.seq).toBe(4);
  expect(reports.at(-1)?.dropped).toBe(3);
  expect(frames.at(-1)).toMatchObject({ seq: QUEUE_LIMIT + 3, dropped: 3 });
  expect(frames.at(-1)?.report).toBeUndefined();
});
