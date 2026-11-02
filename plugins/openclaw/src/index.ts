// picket-openclaw: observes completed `exec` calls that ran gog and reports them to
// Picket on this Mac. It is observation only; it never changes or delays a tool call.

import { homedir } from "node:os";
import { join } from "node:path";
import { definePluginEntry } from "openclaw/plugin-sdk/plugin-entry";
import { observe } from "./gog.js";
import { Reporter } from "./reporter.js";

const DEFAULT_DIRECTORY = join(homedir(), "Library/Application Support/dev.picket.desktop");
const AGENT_ID = /^[a-z0-9][a-z0-9-]{0,31}$/;
// The gateway gives shutdown hooks five seconds; leave most of it to other plugins.
const STOP_BUDGET_MS = 1_000;

export default definePluginEntry({
  id: "picket-openclaw",
  name: "Picket",
  description: "Reports Gmail and Calendar commands run through gog to Picket on this Mac.",
  register(api) {
    const agent = api.pluginConfig.agent;
    const directory = api.pluginConfig.dataDirectory;
    if (typeof agent !== "string" || !AGENT_ID.test(agent)) {
      api.logger.warn("picket-openclaw: set config.agent to the agent id shown in Picket");
      return;
    }
    const reporter = new Reporter({
      agent,
      directory: typeof directory === "string" ? directory : DEFAULT_DIRECTORY,
    });
    reporter.start();
    api.on(
      "after_tool_call",
      (event) => {
        try {
          const observed = observe(event.toolName, event.params, event.result, event.error);
          if (observed.kind === "report") reporter.report(observed.report);
          else if (observed.kind === "unrecognized") reporter.unparsed();
        } catch {
          // Observation must never affect the agent; a failure here is not reported.
        }
      },
      { matcher: ["exec"] },
    );
    api.on("gateway_stop", () => reporter.stop(STOP_BUDGET_MS));
  },
});
