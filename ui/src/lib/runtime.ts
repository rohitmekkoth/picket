import { invoke, isTauri } from "@tauri-apps/api/core";
import { z } from "zod";

export const runtimeInfoSchema = z.object({
  version: z.string(),
  platform: z.string(),
  architecture: z.string(),
  monitoring: z.boolean(),
});

export type RuntimeInfo = z.infer<typeof runtimeInfoSchema>;

export async function getRuntimeInfo(): Promise<RuntimeInfo | null> {
  if (!isTauri()) return null;
  return runtimeInfoSchema.parse(await invoke<unknown>("get_runtime_info"));
}
