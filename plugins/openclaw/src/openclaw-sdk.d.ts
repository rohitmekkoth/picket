// The subset of the OpenClaw plugin SDK this plugin uses, as documented for OpenClaw
// 2026.10.9 (docs/plugins/sdk-overview.md and hooks/tool-policy.md). The host provides
// the module at runtime; declaring it here avoids depending on the whole package.

declare module "openclaw/plugin-sdk/plugin-entry" {
  export interface AfterToolCallEvent {
    toolName: string;
    params: Record<string, unknown>;
    runId?: string;
    toolCallId?: string;
    result?: unknown;
    error?: string;
    durationMs?: number;
  }

  export interface HookOptions {
    matcher?: string[];
    priority?: number;
    timeoutMs?: number;
  }

  export interface PluginLogger {
    debug(message: string): void;
    info(message: string): void;
    warn(message: string): void;
    error(message: string): void;
  }

  export interface PluginApi {
    pluginConfig: Record<string, unknown>;
    logger: PluginLogger;
    on(
      hook: "after_tool_call",
      handler: (event: AfterToolCallEvent) => void,
      options?: HookOptions,
    ): void;
    on(hook: "gateway_stop", handler: () => Promise<void> | void, options?: HookOptions): void;
  }

  export interface PluginEntry {
    id: string;
    name: string;
    description: string;
    register(api: PluginApi): void;
  }

  export function definePluginEntry(entry: PluginEntry): PluginEntry;
}
