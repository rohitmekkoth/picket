// Sends reports to Picket over its local socket without ever blocking or failing the
// agent: frames wait in a bounded queue, each write has a short deadline, and anything
// that cannot be delivered is counted so Picket can show that reports were lost.

import { randomUUID } from "node:crypto";
import { readFile } from "node:fs/promises";
import { connect, type Socket } from "node:net";
import { join } from "node:path";
import type { Report } from "./gog.js";

export const QUEUE_LIMIT = 500;
export const WRITE_TIMEOUT_MS = 100;
export const HEARTBEAT_MS = 60_000;
const RETRY_MS = 5_000;

type Frame = Record<string, unknown>;

export interface ReporterOptions {
  agent: string;
  /** Picket's data directory, holding `ingest.sock` and `ingest.token`. */
  directory: string;
  now?: () => Date;
}

export class Reporter {
  readonly instance = randomUUID();
  private seq = 0;
  private dropped = 0;
  private unrecognized = 0;
  private readonly queue: Frame[] = [];
  private socket: Socket | undefined;
  private token: string | undefined;
  private flushing: Promise<void> | undefined;
  private retryAt = 0;
  private heartbeat: NodeJS.Timeout | undefined;

  constructor(private readonly options: ReporterOptions) {}

  start(): void {
    this.heartbeat = setInterval(() => this.beat(), HEARTBEAT_MS);
    this.heartbeat.unref();
    this.beat();
  }

  /** Numbers and queues one report; the oldest report is dropped when the queue is full. */
  report(report: Report): void {
    this.seq += 1;
    if (this.queue.length >= QUEUE_LIMIT) {
      this.queue.shift();
      this.dropped += 1;
    }
    this.queue.push(this.frame({ report: { ...report } }));
    void this.flush();
  }

  /** Counts a gog command the adapter could not interpret. */
  unparsed(): void {
    this.unrecognized += 1;
    this.beat();
  }

  /** Sends what it can, then a final heartbeat with the counters, within the budget. */
  async stop(budgetMs: number): Promise<void> {
    if (this.heartbeat) clearInterval(this.heartbeat);
    this.retryAt = 0;
    const deadline = new Promise<void>((resolve) => setTimeout(resolve, budgetMs).unref());
    await Promise.race([
      this.flush()
        .then(() => this.beat())
        .then(() => this.flush()),
      deadline,
    ]);
    this.socket?.destroy();
    this.socket = undefined;
  }

  private beat(): void {
    // A heartbeat goes out only behind queued reports, so it never overtakes them.
    if (this.queue.length === 0) this.queue.push(this.frame({}));
    void this.flush();
  }

  private frame(extra: { report?: Report }): Frame {
    return {
      v: 1,
      agent: this.options.agent,
      instance: this.instance,
      seq: this.seq,
      dropped: this.dropped,
      unrecognized: this.unrecognized,
      adapter: "gog",
      adapter_version: 1,
      at: (this.options.now?.() ?? new Date()).toISOString(),
      ...extra,
    };
  }

  /** One flush at a time, so a frame is never written twice. */
  private flush(): Promise<void> {
    this.flushing ??= (async () => {
      while (this.queue.length > 0 && (await this.flushOnce())) {}
    })().finally(() => {
      this.flushing = undefined;
    });
    return this.flushing;
  }

  /** Writes the oldest frame; false when Picket is unreachable for now. */
  private async flushOnce(): Promise<boolean> {
    const socket = await this.connection();
    const frame = this.queue[0];
    if (!socket || frame === undefined) return false;
    // The token is read at connect time, after the frame may have been queued.
    const written = await write(socket, `${JSON.stringify({ ...frame, token: this.token })}\n`);
    if (!written) {
      socket.destroy();
      this.socket = undefined;
      this.retryAt = Date.now() + RETRY_MS;
      return false;
    }
    this.queue.shift();
    return true;
  }

  private async connection(): Promise<Socket | undefined> {
    if (this.socket && !this.socket.destroyed) return this.socket;
    if (Date.now() < this.retryAt) return undefined;
    try {
      this.token = (await readFile(join(this.options.directory, "ingest.token"), "utf8")).trim();
      const socket = await open(join(this.options.directory, "ingest.sock"));
      socket.on("error", () => socket.destroy());
      socket.on("close", () => {
        if (this.socket === socket) this.socket = undefined;
      });
      socket.unref();
      this.socket = socket;
      return socket;
    } catch {
      this.retryAt = Date.now() + RETRY_MS;
      return undefined;
    }
  }
}

function open(path: string): Promise<Socket> {
  return new Promise((resolve, reject) => {
    const socket = connect(path);
    const timer = setTimeout(() => {
      socket.destroy();
      reject(new Error("connect timeout"));
    }, WRITE_TIMEOUT_MS);
    socket.once("connect", () => {
      clearTimeout(timer);
      resolve(socket);
    });
    socket.once("error", (error) => {
      clearTimeout(timer);
      reject(error);
    });
  });
}

function write(socket: Socket, data: string): Promise<boolean> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve(false), WRITE_TIMEOUT_MS);
    socket.write(data, (error) => {
      clearTimeout(timer);
      resolve(!error);
    });
  });
}
