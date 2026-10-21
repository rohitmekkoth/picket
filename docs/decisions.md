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

- [Alignment engine plan](alignment-engine-plan.md) records the focused review, evidence policy, implementation increments, experiments, evaluation gates and unsolved limits. The related sections of `design.md` now reflect it. This is design work only; no product milestone beyond M1 is implemented or authorized by this review.
- Separate observable account state, actor evidence, request fit and alert routing. Remove the old 0.95 claim attribution, 0.30 effects-only baseline, automatic `user` fallback and additive authorization score. Unknown actor is a first-class outcome; a claim or a known contact cannot grant permission.
- Explicit restrictions are gates. Preserve missing fields, conflicting evidence, task revisions, cardinality and collection coverage. An AI extraction is an uncertain input; no AI judge changes v1 verdicts. Expectedness feedback does not automatically label actors, trust contacts or tune thresholds.
- Header fingerprinting is an unvalidated experiment, not actor proof. Personal Gmail's documented message resource does not expose the sending app. Workspace Gmail logs do document app identity and message identifiers, correcting the earlier conversation's overly pessimistic assessment; real API/export availability and unique joins remain unverified. Workspace integration is future scope and adds no permissions now.
- Current OpenClaw documentation identifies `after_tool_call`, but optional result/error fields and harness-specific coverage require an installed-version, per-tool experiment. Treat runtime-linked reports as conditional on runtime integrity, not near-certain independent attribution.
- Metadata-only security notices and purchase indicators do not establish account settings or payments. Record what was observed and use qualified alerts. Collection gaps cannot be repaired into a guaranteed complete action history by resynchronizing current state.
- Add account/resource revision identity, time quality, coverage, evidence provenance and input versioning to the M2 schema review. Derive engine-specific tables incrementally in M3. The raw Messages text retention limit means deterministic replay uses stored extraction; full-text re-extraction and full original-word display cannot be promised.
- The plan's production thresholds are proposals, not measured accuracy. Keep mock tests and controlled real-source validation distinct. Source ambiguity may remain unsolvable; a failed attribution experiment narrows the product claim rather than justifying more confident heuristics.

## Resolve before implementing the owning milestone

1. **M2 event deduplication:** `UNIQUE(source, source_ref, kind)` cannot represent repeated changes to the same Calendar event if source_ref is only its event ID. Define a stable change identity including a source revision.
2. **M2 replay completeness:** ordinary received-mail contacts are upserted without events, while the design says contacts can be rebuilt entirely from the log. Decide how privacy-minimized contact observations and baseline state are replayed.
3. **M2 finding uniqueness:** SQLite UNIQUE constraints permit multiple NULL values. The proposed nullable finding key needs an explicit normalized identity or partial indexes.
4. **M2 retention and feedback:** define the retained chain anchor and transactional checkpoint boundary precisely. Preserve user decisions by stable event/finding identities when derived incidents are rebuilt.
5. **M5 calendar synchronization:** verify that the chosen initial bounded query and subsequent sync-token requests meet Google's parameter restrictions without introducing gaps.
6. **M6 decoder and plugin:** verify the current imessage-database license and the actual OpenClaw hooks before choosing dependencies.
7. **M7 model integrity:** verify model name, license, exact GGUF revision, checksum, and download redirect hosts. The design's model claims are not yet independently validated.
8. **Before public release:** choose the license, Developer ID signing identity, Homebrew tap owner, and validated installation instructions. Local development uses an ad hoc signature; public signing and notarization are not configured.
9. **M2 evidence contract:** settle the observation/evidence/coverage fields and versioned replay inputs described in the alignment plan before freezing migrations. Do not persist raw message bodies to make replay easier.
10. **Before M3:** review the explicit request-constraint and actor-evidence states, select bounded candidate/queue limits, and label uncertainty/adversarial seed scenarios before coding. Keep policy, extraction, coverage and attribution uncertainty separate.
11. **M5/M6 attribution experiments:** test actual header separability, request-channel coverage, tool-result resource linkage and Calendar external changes. No claim of reliable Instinct/Muse attribution is established. Workspace requires a separately scoped integration decision.
12. **M4/M7/release evidence:** operationalize the plan's denominators, held-out splits, uncertainty intervals and per-mode safety/utility gates. Do not publish the proposed thresholds as achieved performance.
