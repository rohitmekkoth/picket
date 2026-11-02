# M6 Messages and OpenClaw plan

Status: increments 1 (shared write path, PR #14) and 2 (Messages, PR #15, E4 passed) are merged. Increment 3 (OpenClaw) is implemented on `feature/m6-openclaw`; E2 is pending. This plan implements the M6 scope in [design.md](design.md) and the E2/E4 experiments in [alignment-engine-plan.md](alignment-engine-plan.md). It adds no network hosts, Google scopes or telemetry.

## Outcome and boundaries

M6 adds the agent-side evidence: what the user asked an iMessage agent and what it reported (Messages), and what an instrumented runtime reports it did (OpenClaw). Each adapter must demonstrate what it can actually see; missing access, unsupported formats, dropped reports and unlinkable results degrade coverage visibly instead of creating certainty.

Picket does not infer an actor from timing, writing style or email headers. Direction inside a registered agent conversation is reliable source metadata; an observed account effect stays "actor unknown" unless a claim, a validated runtime link or the user's confirmation supports more. Header fingerprints remain the separate E1 experiment.

Out of scope for v1: group chats, an OpenClaw gateway on another machine or container, and distinguishing an agent that operates the user's own Messages account from the user. These limits are documented in the product.

## Upstream facts verified 2026-10-27

- `imessage-database` 4.x is GPL-3.0-or-later.
- OpenClaw's `after_tool_call` is observation-only: handlers run concurrently, return values are ignored, and a thrown or timed-out handler is logged. Its `runId`, `toolCallId`, `result`, `error` and `durationMs` fields are optional and harness-dependent. Plugins register with `definePluginEntry` and `api.on`.
- OpenClaw commonly reaches Gmail and Calendar through the `gog` CLI via the generic `exec` tool, so a tool result is a command and its output. Whether `gog` write commands report provider resource ids is unverified; E2 measures it.

## Decisions

1. **Own minimal `attributedBody` decoder.** Picket needs plain text only. A bounded decoder of about a hundred lines with adversarial tests keeps untrusted parsing small and leaves the project license open. If macOS changes the format, the health check reports undecodable agent messages and the agent falls back to effects-only mode with a warning.
2. **One writer, many collectors.** The store remains the only writer, on its own thread. Collectors and the ingest socket submit batches and wait for the commit; they read through their own read-only connections. A slow Gmail metadata scan cannot delay Messages capture or the plugin socket, and the log keeps one order and one transaction per batch.
3. **Portable logic in `picket-core`.** Google normalization and polling, the `chat.db` reader and decoder, and ingest frame validation live in the core and are tested on Linux CI. The app keeps HTTP, Keychain, Full Disk Access detection and the Unix socket.
4. **No stored message text.** A message observation records direction, agent, time, the stable `chat.db` message identity, edit revision and the design's bounded snippet. Extraction in M7 re-reads text from `chat.db` transiently, so messages captured before M7 can still be extracted and text never enters the log, exports or crash artifacts.
5. **Polling instead of FSEvents.** A read-only `MAX(ROWID)` and edit-watermark query every 5 s is simpler and robust across WAL checkpoints, and well inside the six-minute target. Edits and retractions become new revisions.
6. **Agent registry as log events.** Registration records an id, mode, normalized handles (E.164 phone numbers, lower-cased emails) and a connected-since date through one validated command that the M8 wizard reuses.
7. **Request-channel honesty.** "No observed request" requires healthy capture for that agent and completed extraction; otherwise the finding states that the request channel was unavailable or pending.
8. **Observe-only plugin with normalized reports.** `picket-openclaw` handles `after_tool_call` fire-and-forget with a 100 ms write timeout and a bounded queue. A pinned `gog` adapter emits only operation, targets, provider id, outcome, adapter instance, sequence and drop counters. Unrecognized tools are counted, never forwarded. Rust validates every frame; heartbeats update live status, and sequence gaps or drops change coverage. The socket token authenticates a process running as the user, not OpenClaw itself.

## Increment 3 facts and decisions (2026-11-02)

Verified against the published `openclaw` 2026.10.9 package documentation and the `gogcli` source at commit `82e7fd6` (0.47.x):

- `after_tool_call` is unchanged: observe-only, concurrent, optional `runId`, `result` and `error`. For `exec`, `result` is currently the bare stdout/stderr string with no exit code and no `error` ([openclaw#102961](https://github.com/openclaw/openclaw/issues/102961), open). Success therefore cannot come from the runtime.
- gog prints the provider id only after the API call succeeded: `message_id\t<id>` (or `messageId` in JSON) for send, reply, forward and draft send; `id\t<id>` (or `event.id`) for Calendar create, update and respond; `deleted\ttrue` with `eventId` for delete. It prints no etag, so a Calendar update can be reported but never linked, because linkage requires the revision.
- `gog api` (raw API passthrough) and `gog mcp` (a server) can send mail without a command Picket can read.

Decisions:

1. **Success only from gog's own id line.** The adapter reports `succeeded` only when the output carries exactly one well-formed id for the command it parsed; `failed` only when OpenClaw supplies an error; otherwise `unknown`. Several sends from one command are not matched to recipients and stay unknown.
2. **Exact parsing, conservative counting.** Commands are tokenized like a POSIX shell and refused on any expansion, operator, glob or comment. Only `gog` as the program, a pinned list of root flags, and the Gmail send family and Calendar create/update/delete/respond are interpreted. Wrappers, compound commands, `gog api`, `gog mcp`, `autoreply`, Calendar `move` and the special event types count as unparsed, which records an unsupported-format gap. Reads and dry runs are ignored.
3. **Account.** A report that names another Google account (`--account`, `-a` or `GOG_ACCOUNT`) is counted in the cursor and not recorded. A report naming no account is recorded under the monitored account; it links only through an exact provider id, so a wrong-account link requires an id collision.
4. **Sequence and loss.** Each plugin process has an instance id and numbers its reports; heartbeats carry the latest number and cumulative drop and unparsed counters, and travel behind queued reports. A gap or a higher drop count records `reports_lost`; a new instance restarts the counters. Reports lost when a previous process stopped cannot be known.
5. **Liveness.** No frame for three heartbeats records an interrupted gap; a registered OpenClaw agent whose plugin never connected is `not_connected`. Coverage for runtime reports is informational: a missing report never makes an effect less suspicious, it only removes a possible link.
6. **Socket and token.** Both live in the app data directory (`~/Library/Application Support/dev.picket.desktop`), owner-only; the token is 256 bits from the OS random source and compared in constant time. The socket path is checked against the macOS 104-byte limit. Picket never replies, so the plugin cannot be slowed by it.
7. **No SDK dependency.** The plugin declares the four SDK types it uses instead of depending on the 2026.10.9 package; a shared example frame in `plugins/openclaw/contract` is checked by both the TypeScript and Rust tests.

## E2 protocol

Owner-approved setup only: OpenClaw 2026.10.9 with a local model, a localhost-only gateway, only the `gog` skill, `exec` restricted to `gog`, and gog authorized for Gmail send and Calendar events on the controlled test account. Build the plugin with `pnpm build:openclaw`, register an OpenClaw agent in Picket, then `openclaw plugins install --link plugins/openclaw --force`, set `plugins.entries.picket-openclaw.config.agent` to the id Picket shows, and `openclaw plugins enable picket-openclaw`. Record, as counts only:

1. Plugin connects; heartbeat state reaches connected; stopping the gateway and Picket in turn never delays a tool call.
2. A send, a Calendar create, an update, a delete and an RSVP: whether each produced a report, whether the id was present, and whether the engine linked the observed effect.
3. A compound command and `gog api`: counted as unparsed, no report.
4. Picket stopped during a send: the drop or gap appears after restart.
5. What `event.result` actually contained for `exec` (shape only, no content), which decides whether ids are visible at all.

If ids do not reach the plugin, OpenClaw ships as reported, not linked, and the product says so.

## Increments

1. **`refactor/collector-writer`:** writer thread and shared write path; Google wire types and polling move into `picket-core`. No behavior change; existing tests and a live poll verify it.
2. **`feature/m6-messages`:** agent registry; synthetic fixture `chat.db` generator; reader, decoder, date conversion, 1:1 filter, edit revisions and coverage (permission missing, unsupported schema); macOS collector; request-channel semantics with new evaluation families; E4 with Instinct, reading only the Instinct 1:1 chat.
3. **`feature/m6-openclaw`:** ingest socket and token; TypeScript plugin and `gog` adapter; plugin tests including a stopped app; E2 with a local model, a localhost-only gateway, only the `gog` skill, `exec` restricted to `gog`, and Gmail send and Calendar events access on the controlled test account. If results cannot be linked to provider resources, OpenClaw mode ships as reported, not linked.

## Acceptance

- Fixture `chat.db` yields correct user and agent message observations with correct times, edits and coverage; synthetic data only.
- Plugin reports arrive and validate; killing or stopping the app never blocks the agent; drops and gaps are visible.
- E2 and E4 results are recorded in `docs/decisions.md` with what each adapter can and cannot establish, and the product claims are narrowed to match.
- `make check`, `make test`, `make eval`, `make audit` and `make e2e` pass; `make smoke-macos` for native changes; live checks use the controlled test account and Instinct's chat only, with no account data committed.
