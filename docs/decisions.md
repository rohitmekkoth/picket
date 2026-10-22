# Decisions and unresolved design details

## Foundation, 2026-10-20

- Package names are `picket-core`, `picket-app`, and `picket-cli`; the CLI binary is `picket`. Prefixes avoid ambiguity with Rust's built-in core library.
- Keep the original supplied design unchanged under ignored local storage. The repository copy is docs/design.md, with personal author metadata omitted and punctuation normalized. Product and package names use Picket.
- The setup branch established the environment and runnable shell. The remaining M1 native notification action investigation was completed in PR #5; later product features remain outside the foundation.
- Pin the installed Rust toolchain, Node, pnpm, direct frontend packages, lockfiles, and CI actions. Use a single pnpm workspace and a single Cargo workspace.
- Do not create the OpenClaw package until real upstream hook names and payloads are verified in M6.
- Keep the public project license undecided until the Messages decoder dependency is reviewed. Do not add a possibly GPL decoder while implying an MIT or Apache-only distribution.
- Delay model download and llama.cpp integration until the mock pipeline is evaluated, as required by the design's milestone order. CMake and Clang are available for that build.
- Do not add empty subsystem source modules, fabricated evaluation metrics, or disabled CI gates. Document the directory's future boundary instead.
- Use stderr tracing for the current data-free shell. Before handling account data, add tested redaction and size-based rotation; tracing alone is not a privacy filter.
- Generate the simple temporary app icon from `assets/icon.svg` with `pnpm tauri icon ../../assets/icon.svg --output icons`; do not manually edit generated icons or Tauri schemas.

## Native notifications, 2026-10-20

- Use `tauri-plugin-notification` 2.7 for the M8 native notification path. [Version 2.6 added desktop action callbacks](https://v2.tauri.app/release/notification/v2.6.0/), so a separate `objc2` bridge is not needed for the initial implementation. Keep action handling in Rust so it works while the webview window is hidden.
- The packaged, ad hoc signed macOS app submitted a synthetic notification with Review and This was expected actions. macOS accepted it and stored it in Notification Center. The first launch showed the system's notification permission notice; notifications had to be enabled in System Settings before the test could proceed.
- Clicking the stored notification in Notification Center restored the hidden Picket window and selected the Review test view. Choosing This was expected from its Options menu restored the window and selected the expected-action test view. Both results were checked in the native window. A banner was not observed in this test; check banner presentation separately during M8 UX testing. The M8 implementation can use the plugin for click-through and actions; incident lookup and feedback persistence belong to M8.

## Alignment review, 2026-10-21

- [Alignment engine plan](alignment-engine-plan.md) records the focused review, evidence policy, implementation increments, experiments, evaluation gates and unsolved limits. The related sections of `design.md` reflect it. This review initially added no product code; subsequent M2 and M3 implementation is recorded below.
- Separate observable account state, actor evidence, request fit and alert routing. Remove the old 0.95 claim attribution, 0.30 effects-only baseline, automatic `user` fallback and additive authorization score. Unknown actor is a first-class outcome; a claim or a known contact cannot grant permission.
- Explicit restrictions are gates. Preserve missing fields, conflicting evidence, task revisions, cardinality and collection coverage. An AI extraction is an uncertain input; no AI judge changes v1 verdicts. Expectedness feedback does not automatically label actors, trust contacts or tune thresholds.
- Header fingerprinting is an unvalidated experiment, not actor proof. Personal Gmail's documented message resource does not expose the sending app. Workspace Gmail logs do document app identity and message identifiers, correcting the earlier conversation's overly pessimistic assessment; real API/export availability and unique joins remain unverified. Workspace integration is future scope and adds no permissions now.
- Current OpenClaw documentation identifies `after_tool_call`, but optional result/error fields and harness-specific coverage require an installed-version, per-tool experiment. Treat runtime-linked reports as conditional on runtime integrity, not near-certain independent attribution.
- Metadata-only security notices and purchase indicators do not establish account settings or payments. Record what was observed and use qualified alerts. Collection gaps cannot be repaired into a guaranteed complete action history by resynchronizing current state.
- Add account/resource revision identity, time quality, coverage, evidence provenance and input versioning to the M2 schema review. Derive engine-specific tables incrementally in M3. The raw Messages text retention limit means deterministic replay uses stored extraction; full-text re-extraction and full original-word display cannot be promised.
- The plan's production thresholds are proposals, not measured accuracy. Keep mock tests and controlled real-source validation distinct. Source ambiguity may remain unsolvable; a failed attribution experiment narrows the product claim rather than justifying more confident heuristics.

## Resolve before implementing the owning milestone

1. **M2 source identity, contact replay, retention and feedback:** resolved for the initial event store below. M3 finding keys are explicit and non-null. Real Calendar adapters must supply meaningful revisions in M5.
5. **M5 calendar synchronization:** verify that the chosen initial bounded query and subsequent sync-token requests meet Google's parameter restrictions without introducing gaps.
6. **M6 decoder and plugin:** verify the current imessage-database license and the actual OpenClaw hooks before choosing dependencies.
7. **M7 model integrity:** verify model name, license, exact GGUF revision, checksum, and download redirect hosts. The design's model claims are not yet independently validated.
8. **Before public release:** choose the license, Developer ID signing identity, Homebrew tap owner, and validated installation instructions. Local development uses an ad hoc signature; public signing and notarization are not configured.
9. **M5 adapter evidence contract:** the M2/M3 typed inputs, source identity and coverage states support mock replay. Before real collectors, add and validate source time quality, adapter provenance, scope and loss counters. Do not persist raw message bodies to make replay easier.
10. **M3 mock contract:** explicit request and actor states, bounded candidate/outbox reads and 24 labeled seeds are implemented. Keep policy, extraction, coverage and attribution uncertainty separate as integrations are added.
11. **M5/M6 attribution experiments:** test actual header separability, request-channel coverage, tool-result resource linkage and Calendar external changes. No claim of reliable Instinct/Muse attribution is established. Workspace requires a separately scoped integration decision.
12. **M4/M7/release evidence:** operationalize the plan's denominators, held-out splits, uncertainty intervals and per-mode safety/utility gates. Do not publish the proposed thresholds as achieved performance.

## Event store implementation, 2026-10-21

- The M2 schema uses `(account, source, resource, revision, kind)` as a non-null event identity. An adapter must supply a stable revision for each distinct Calendar change; reusing an event ID alone would lose updates. Exact retries deduplicate, while conflicting content for the same key is rejected.
- Events carry a schema version, typed payload, source and observation time. Source coverage changes and privacy-minimized received-contact observations are events, so they are available for replay. Collector adapters must validate and minimize their fields before submission; the store also caps event size and source identity lengths.
- The hash chain includes canonical JSON, ordered sequence numbers, a stored head, and a retained-prefix anchor. A single mutable connection owns writes. `picket verify --database` uses a separate read-only connection and reports the first broken sequence, including a removed tail. This detects accidental or casual modification, not a full malicious rewrite of both rows and chain metadata.
- Retention deletes a verified prefix in one transaction with an anchor update and a new checkpoint event. The latest feedback, contact, policy and coverage records are copied forward as typed snapshot events, preserving their replayable current state. A retained checkpoint marks the limited historical horizon; absent older requests or effects cannot be inferred from it.
- M3 uses stable, explicit, non-null finding identities and resource-based incident groups. The M2 migration contains only events and chain state; M3 adds assessment and outbox tables in a separate migration.

## Synthetic alignment foundation, 2026-10-21

- The portable matcher returns separate actor evidence, request fit, account-policy hits, pending state, and route. A successful OpenClaw report links only when account, operation, complete target set, provider resource, and Calendar update revision match. Runtime linkage is conditional on the adapter's honesty; real adapter capability remains untested. Claims stay at `reported` strength and cannot override a runtime link.
- Effects-only observations, external Calendar changes, failed/unlinked tool calls, incomplete targets, multiple plausible tasks, and conflicting runtime links abstain from a named actor or complete request match. Request fields use explicit `Known`, `ExplicitAny`, `Unknown`, and `Ambiguous` states. Matching known fields is only consistency with visible scope, not proof of full authorization.
- A quantity bound is evaluated only when validated runtime reports link each counted effect to the same task. Human account activity does not consume an agent's task allowance; missing task links yield insufficient evidence. Calendar start time and explicit request expiry are hard constraints.
- Account policies are local Picket events with an explicit high-consequence flag. A visible violation can route to review with an unknown actor. Retention snapshots preserve the latest policy as of the cutoff; the replay engine reads the original policy time from the snapshot. Policy editing and confirmation UI remain future work.
- `picket replay --database ... --at ...` verifies the log and prints aggregate counts only. The separate store API persists finding revisions and a notification outbox atomically with inputs, but no collector or OS delivery worker calls it yet. Its synthetic tests are a smoke gate, not the M4 evaluation baseline or measured precision.

## M3 mock engine, 2026-10-21

- Deterministic matching separates actor evidence, visible request fit, account policy, claim consistency, coverage state and alert routing. Claims without a provider ID abstain when more than one similar effect is visible. A lone request with no task link is insufficient for comparison. Baseline imports and observations with unknown origin do not create effect or metadata-notice findings.
- SQLite schema v2 preserves assessment revisions, a processing cursor and a durable notification outbox read in bounded pages. Input, assessment and outbox intent commit in one transaction through `append_assessed`; duplicate input and restart replay do not duplicate alerts. Delivered findings can receive a correction after material late evidence or feedback. Delivery itself belongs to M8.
- Twenty-four labeled synthetic scenarios replay through the store with a scripted `MockModel`; paired cases with identical visible evidence but different true actors remain unknown. These are M3 safety smoke tests. They are not independent validation samples, accuracy estimates or a substitute for the M4 held-out evaluation.
- Optional event fields added during M3 preserve older canonical log entries. Verification checks canonical source JSON and the original hash; a semantically identical retry of an old record deduplicates. Real adapter provenance, source time quality, collection completeness and runtime integrity remain M5/M6 work.

## M4 synthetic evaluation, 2026-10-21

- The M4 baseline is a versioned regression gate over 18 fixed families and 288 constructed cases. It enforces minimum denominators, actor safety, linkage and conflict recall, alert routing, and coverage visibility. Mode and family counts remain separate in the report.
- Variants within a family are correlated. The harness therefore reports raw counts and marks confidence intervals `not_measured`; calculating a binomial interval over 288 constructed variants would imply unsupported independence. Real-source, model, latency, and base-rate measures remain open. No production accuracy target is claimed from M4.

## M5 planning, 2026-10-22

- [M5 Google observation plan](m5-plan.md) records the implementation sequence and verification gates before collector code. The account label in Picket will be locally opaque and bound to the account reported by the allowed Gmail API; the approved scopes do not provide an OpenID subject claim.
- Calendar's documented `events.list` rules forbid `timeMin` and `timeMax` alongside `syncToken`. The bounded baseline and token-bearing initial sync therefore need separate queries. Validate pagination, recurrence and large-calendar behavior with synthetic fixtures and a controlled account before treating coverage as healthy.
- A cursor may advance only in the same committed write as its minimized observations and coverage state. Expired Gmail history or Calendar sync tokens create an explicit gap that current-state resync cannot erase.

## M5 implementation notes, 2026-10-22

- The Google OAuth connection verifies the two granted scopes and Gmail profile, stores a locally opaque account ID with the Keychain refresh credential, and reuses that ID on reauthorization of the same email account. The ID is a local binding, not a Google subject identifier.
- Google observations carry the adapter name, version, approved scope, and source-time quality. Gmail's internal date is provider time. A Calendar record without `updated` uses its ingestion time and is marked as a fallback; malformed provider time fails validation. Older canonical events omit these optional fields and remain verifiable.
- Calendar uses a bounded baseline, a separate unbounded token-bearing initial sync, and a daily bounded window refresh that preserves the incremental cursor. Known events remain tracked when moved outside the window; unknown events outside it are skipped. Source failures and expired cursors produce durable coverage gaps.
- Synthetic collector tests cover retry deduplication, restart, Calendar revision state, expired cursors, invalid source dates, and rolling-window entry. Live OAuth, provider behavior, source-change latency, and actual recurrence edge cases still require the controlled account check before M5 is marked complete.
