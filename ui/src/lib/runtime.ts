import { invoke, isTauri } from "@tauri-apps/api/core";
import { z } from "zod";

// Validate untrusted IPC output before the renderer uses it.
export const runtimeInfoSchema = z.object({
  version: z.string().min(1),
  platform: z.string().min(1),
  architecture: z.string().min(1),
  monitoring: z.boolean(),
});

export type RuntimeInfo = z.infer<typeof runtimeInfoSchema>;

// Browser previews return null and never attempt a native command.
export async function getRuntimeInfo(): Promise<RuntimeInfo | null> {
  if (!isTauri()) return null;
  return runtimeInfoSchema.parse(await invoke<unknown>("get_runtime_info"));
}

const googleStatusSchema = z.enum([
  "disconnected",
  "starting",
  "monitoring",
  "degraded",
  "unavailable",
]);
export type GoogleStatus = z.infer<typeof googleStatusSchema>;

export async function getGoogleStatus(): Promise<GoogleStatus | null> {
  if (!isTauri()) return null;
  return googleStatusSchema.parse(await invoke<unknown>("google_connected"));
}

export async function connectGoogle(clientId: string, clientSecret: string): Promise<void> {
  if (!isTauri()) throw new Error("Open the desktop app to connect Google.");
  await invoke("connect_google", { clientId, clientSecret });
}

const agentsSchema = z.object({
  agents: z.array(
    z.object({
      id: z.string().min(1),
      name: z.string().min(1),
      mode: z.enum(["open_claw", "messages", "effects_only"]),
      handles: z.array(z.string().min(1)),
      enabled: z.boolean(),
    }),
  ),
  messages: z.enum([
    "inactive",
    "waiting",
    "capturing",
    "permission_missing",
    "unsupported",
    "unavailable",
  ]),
});
export type Agents = z.infer<typeof agentsSchema>;

export async function listAgents(): Promise<Agents | null> {
  if (!isTauri()) return null;
  return agentsSchema.parse(await invoke<unknown>("list_agents"));
}

// The region only interprets numbers entered without a country code; Rust validates it.
export type AgentMode = "messages" | "effects_only";

export async function registerAgent(name: string, mode: AgentMode, handle: string): Promise<void> {
  if (!isTauri()) throw new Error("Open the desktop app to add agents.");
  const region = new Intl.Locale(navigator.language).maximize().region ?? null;
  await invoke("register_agent", {
    name,
    mode,
    handle: mode === "messages" ? handle : null,
    region,
  });
}
