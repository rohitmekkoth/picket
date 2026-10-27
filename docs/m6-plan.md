# M6 Messages and OpenClaw plan

Status: planned 2026-10-27; increment 1 (shared write path) implemented in PR #14. This plan implements the M6 scope in [design.md](design.md) and the E2/E4 experiments in [alignment-engine-plan.md](alignment-engine-plan.md). It adds no network hosts, Google scopes or telemetry.

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

## Increments

1. **`refactor/collector-writer`:** writer thread and shared write path; Google wire types and polling move into `picket-core`. No behavior change; existing tests and a live poll verify it.
2. **`feature/m6-messages`:** agent registry; synthetic fixture `chat.db` generator; reader, decoder, date conversion, 1:1 filter, edit revisions and coverage (permission missing, unsupported schema); macOS collector; request-channel semantics with new evaluation families; E4 with Instinct, reading only the Instinct 1:1 chat.
3. **`feature/m6-openclaw`:** ingest socket and token; TypeScript plugin and `gog` adapter; plugin tests including a stopped app; E2 with a local model, a localhost-only gateway, only the `gog` skill, `exec` restricted to `gog`, and Gmail send and Calendar events access on the controlled test account. If results cannot be linked to provider resources, OpenClaw mode ships as reported, not linked.

## Acceptance

- Fixture `chat.db` yields correct user and agent message observations with correct times, edits and coverage; synthetic data only.
- Plugin reports arrive and validate; killing or stopping the app never blocks the agent; drops and gaps are visible.
- E2 and E4 results are recorded in `docs/decisions.md` with what each adapter can and cannot establish, and the product claims are narrowed to match.
- `make check`, `make test`, `make eval`, `make audit` and `make e2e` pass; `make smoke-macos` for native changes; live checks use the controlled test account and Instinct's chat only, with no account data committed.
