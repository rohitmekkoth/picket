// The gog adapter: turns one `exec` call that ran gog into a bounded report of ids and
// addresses, or says it saw a gog command it cannot interpret. It never returns command
// text, message bodies or tool output. Verified against gog 0.47 source.

export type Action =
  | "email_send"
  | "calendar_create"
  | "calendar_update"
  | "calendar_delete"
  | "calendar_rsvp";
export type Outcome = "succeeded" | "failed" | "unknown";

export interface Report {
  action: Action;
  account?: string;
  targets: string[];
  resource?: string;
  outcome: Outcome;
}

export type Observation =
  | { kind: "report"; report: Report }
  | { kind: "unrecognized" }
  | { kind: "ignored" };

const MAX_TARGETS = 50;
const MAX_OUTPUT = 64 * 1024;
const RESOURCE = /^[A-Za-z0-9_-]{1,256}$/;

const GMAIL = new Set(["gmail", "mail", "email"]);
const CALENDAR = new Set(["calendar", "cal"]);
// Gmail and Calendar subcommands that cannot send mail or change an event.
const GMAIL_OTHER = new Set(
  "search find query ls list messages message msg msgs thread threads read get info show raw attachment url history labels label batch archive mark-read read-messages unread mark-unread trash import track settings watch autoforward delegates filters forwarding sendas vacation".split(
    " ",
  ),
);
const CALENDAR_OTHER = new Set(
  "calendars subscribe sub add-calendar unsubscribe unsub create-calendar new-calendar update-calendar edit-calendar set-calendar delete-calendar acl permissions perms alias events list ls event get info show raw freebusy colors conflicts changed search find query time users team propose-time".split(
    " ",
  ),
);
const CALENDAR_ACTIONS: Record<string, Action> = {
  create: "calendar_create",
  add: "calendar_create",
  new: "calendar_create",
  update: "calendar_update",
  edit: "calendar_update",
  set: "calendar_update",
  delete: "calendar_delete",
  rm: "calendar_delete",
  del: "calendar_delete",
  remove: "calendar_delete",
  respond: "calendar_rsvp",
  rsvp: "calendar_rsvp",
  reply: "calendar_rsvp",
};
// Root flags that take a value; any other flag before the command path is not understood.
const ROOT_VALUE = new Set(
  "--color --home --account --acct -a --client --access-token --quota-project --enable-commands --enable-commands-exact --disable-commands --select --pick --project".split(
    " ",
  ),
);
const ROOT_SWITCH = new Set(
  "--json --machine -j --plain --tsv -p --wrap-untrusted --results-only --force --yes --assume-yes -y --no-input --non-interactive --noninteractive --verbose -v --readonly --gmail-no-send".split(
    " ",
  ),
);
const DRY_RUN = new Set(["--dry-run", "--noop", "--preview", "--dryrun", "-n"]);
const RECIPIENTS = new Set(["--to", "--cc", "--bcc"]);

export function observe(
  toolName: string,
  params: Record<string, unknown>,
  result: unknown,
  error: string | undefined,
): Observation {
  if (toolName !== "exec" || typeof params.command !== "string") return { kind: "ignored" };
  const command = params.command;
  if (!/\bgog\b/.test(command)) return { kind: "ignored" };
  const words = tokenize(command);
  if (words === null) return { kind: "unrecognized" };
  const env = environment(params.env);
  let index = 0;
  for (; index < words.length; index++) {
    const assignment = /^([A-Za-z_][A-Za-z0-9_]*)=(.*)$/.exec(words[index] ?? "");
    if (!assignment) break;
    env.set(assignment[1] ?? "", assignment[2] ?? "");
  }
  const program = words[index];
  if (program === undefined || program.split("/").pop() !== "gog") {
    // gog run through another program, such as xargs or sudo, cannot be interpreted.
    return words.some((word) => word.split("/").pop() === "gog")
      ? { kind: "unrecognized" }
      : { kind: "ignored" };
  }
  const parsed = parseArguments(words.slice(index + 1));
  if (parsed === null) return { kind: "unrecognized" };
  if (parsed.dryRun) return { kind: "ignored" };
  const action = classify(parsed.path);
  if (action === "ignored" || action === "unrecognized") return { kind: action };
  const account = email(parsed.account ?? env.get("GOG_ACCOUNT"));
  const output = text(result);
  const resource = output === undefined ? undefined : resourceFrom(action, output);
  const outcome: Outcome =
    resource !== undefined ? "succeeded" : error && error.length > 0 ? "failed" : "unknown";
  const named = action === "email_send" ? parsed.recipients : parsed.attendees;
  const targets = [...new Set(named.map(email).filter((value) => value !== undefined))];
  if (targets.length > MAX_TARGETS) return { kind: "unrecognized" };
  return {
    kind: "report",
    report: {
      action,
      targets,
      outcome,
      ...(account === undefined ? {} : { account }),
      ...(resource === undefined ? {} : { resource }),
    },
  };
}

/** Splits a command like a POSIX shell, or returns null for anything beyond plain words. */
export function tokenize(command: string): string[] | null {
  const words: string[] = [];
  let word = "";
  let started = false;
  for (let i = 0; i < command.length; i++) {
    const char = command[i] ?? "";
    if (char === "'") {
      const end = command.indexOf("'", i + 1);
      if (end < 0) return null;
      word += command.slice(i + 1, end);
      started = true;
      i = end;
    } else if (char === '"') {
      let j = i + 1;
      for (; j < command.length && command[j] !== '"'; j++) {
        const inner = command[j] ?? "";
        if (inner === "$" || inner === "`") return null;
        if (inner === "\\" && j + 1 < command.length) {
          const next = command[j + 1] ?? "";
          if ('"\\$`'.includes(next)) {
            word += next;
            j++;
            continue;
          }
        }
        word += inner;
      }
      if (j >= command.length) return null;
      started = true;
      i = j;
    } else if (char === "\\") {
      const next = command[i + 1];
      if (next === undefined) return null;
      if (next !== "\n") {
        word += next;
        started = true;
      }
      i++;
    } else if (char === " " || char === "\t") {
      if (started) words.push(word);
      word = "";
      started = false;
    } else if (";&|<>()$`\n\r*?[]{}~#!".includes(char)) {
      return null;
    } else {
      word += char;
      started = true;
    }
  }
  if (started) words.push(word);
  return words;
}

interface Arguments {
  path: string[];
  account: string | undefined;
  dryRun: boolean;
  recipients: string[];
  attendees: string[];
}

function parseArguments(words: string[]): Arguments | null {
  const parsed: Arguments = {
    path: [],
    account: undefined,
    dryRun: false,
    recipients: [],
    attendees: [],
  };
  for (let i = 0; i < words.length; i++) {
    const word = words[i] ?? "";
    if (word === "--") return null;
    if (!word.startsWith("-") || word === "-") {
      parsed.path.push(word.toLowerCase());
      continue;
    }
    const equals = word.startsWith("--") ? word.indexOf("=") : -1;
    const flag = equals > 0 ? word.slice(0, equals) : word;
    const inline = equals > 0 ? word.slice(equals + 1) : undefined;
    const takesValue = ROOT_VALUE.has(flag) || RECIPIENTS.has(flag) || flag === "--attendees";
    let value = inline;
    if (takesValue && value === undefined) {
      value = words[i + 1];
      if (value === undefined) return null;
      i++;
    }
    if (DRY_RUN.has(flag)) parsed.dryRun = true;
    else if (flag === "--account" || flag === "--acct" || flag === "-a") parsed.account = value;
    else if (RECIPIENTS.has(flag)) parsed.recipients.push(...list(value));
    else if (flag === "--attendees") parsed.attendees.push(...attendees(value));
    else if (!takesValue && !ROOT_SWITCH.has(flag) && !complete(parsed.path)) {
      // An unknown flag before the command is known may take a value we would misread.
      return null;
    }
  }
  return parsed;
}

/** Whether the words so far name a whole command, so later flags cannot shift it. */
function complete(path: string[]): boolean {
  const [group, command] = path;
  if (group === "send") return true;
  if (group !== undefined && GMAIL.has(group) && (command === "drafts" || command === "draft")) {
    return path.length >= 3;
  }
  return path.length >= 2;
}

function classify(path: string[]): Action | "ignored" | "unrecognized" {
  const [group, command, sub] = path;
  if (group === undefined) return "ignored";
  if (group === "send") return "email_send";
  // Raw API calls and the MCP server can send mail without a command Picket can read.
  if (group === "api" || group === "mcp") return "unrecognized";
  if (GMAIL.has(group)) {
    if (command === undefined) return "ignored";
    if (["send", "reply", "reply-all", "replyall", "forward", "fwd"].includes(command)) {
      return "email_send";
    }
    if (command === "drafts" || command === "draft") {
      return sub === "send" ? "email_send" : "ignored";
    }
    return GMAIL_OTHER.has(command) ? "ignored" : "unrecognized";
  }
  if (CALENDAR.has(group)) {
    if (command === undefined) return "ignored";
    const action = CALENDAR_ACTIONS[command];
    if (action !== undefined) return action;
    return CALENDAR_OTHER.has(command) ? "ignored" : "unrecognized";
  }
  return "ignored";
}

function resourceFrom(action: Action, output: string): string | undefined {
  const json = parseJson(output);
  let ids: string[];
  if (action === "email_send") {
    ids =
      json === undefined
        ? lines(output, "message_id")
        : [json.messageId, ...array(json.messages).map((item) => record(item)?.messageId)].filter(
            (id): id is string => typeof id === "string",
          );
  } else if (action === "calendar_delete") {
    const deleted = json === undefined ? lines(output, "deleted")[0] === "true" : json.deleted;
    ids =
      deleted === true
        ? (json === undefined ? lines(output, "eventId") : [json.eventId]).filter(
            (id): id is string => typeof id === "string",
          )
        : [];
  } else {
    const id = json === undefined ? lines(output, "id") : [record(json.event)?.id];
    ids = id.filter((value): value is string => typeof value === "string");
  }
  // Several sends from one command cannot be matched to their recipients.
  return ids.length === 1 && RESOURCE.test(ids[0] ?? "") ? ids[0] : undefined;
}

function lines(output: string, key: string): string[] {
  return output
    .split("\n")
    .map((line) => line.replace(/\r$/, ""))
    .filter((line) => line.startsWith(`${key}\t`))
    .map((line) => line.slice(key.length + 1));
}

function parseJson(output: string): Record<string, unknown> | undefined {
  const trimmed = output.trim();
  if (!trimmed.startsWith("{")) return undefined;
  try {
    return record(JSON.parse(trimmed));
  } catch {
    return undefined;
  }
}

function record(value: unknown): Record<string, unknown> | undefined {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function array(value: unknown): unknown[] {
  return Array.isArray(value) ? value : [];
}

/** The tool's text output, from a bare string or a content list. */
function text(result: unknown): string | undefined {
  let value: string | undefined;
  if (typeof result === "string") value = result;
  else {
    const parts = array(record(result)?.content)
      .map((part) => record(part))
      .filter((part) => part?.type === "text" && typeof part.text === "string")
      .map((part) => part?.text as string);
    if (parts.length > 0) value = parts.join("\n");
  }
  return value !== undefined && value.length <= MAX_OUTPUT ? value : undefined;
}

function environment(value: unknown): Map<string, string> {
  const env = new Map<string, string>();
  const entries = record(value);
  if (entries && typeof entries.GOG_ACCOUNT === "string") {
    env.set("GOG_ACCOUNT", entries.GOG_ACCOUNT);
  }
  return env;
}

function list(value: string | undefined): string[] {
  return (value ?? "").split(",");
}

/** Attendee entries may carry modifiers such as `;optional`. */
function attendees(value: string | undefined): string[] {
  return list(value).map((entry) => entry.split(";")[0] ?? "");
}

/** An address in angle brackets, or a bare one, lower-cased; aliases are not addresses. */
function email(value: string | undefined): string | undefined {
  if (value === undefined) return undefined;
  const bracketed = /<([^<>]+)>/.exec(value);
  const address = (bracketed ? bracketed[1] : value)?.trim().toLowerCase() ?? "";
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(address) && address.length <= 320 ? address : undefined;
}
