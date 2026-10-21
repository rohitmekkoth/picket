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
