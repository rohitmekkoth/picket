# Picket development handoff

Use this file to resume work in a new Codex chat without reconstructing project history.

## Read first

Read these files in order before changing code:

1. `AGENTS.md` - mandatory repository and Git workflow instructions.
2. `docs/design.md` - authoritative product scope, architecture, security constraints, milestones, and acceptance criteria.
3. `docs/decisions.md` - reviewed refinements and unresolved questions. These decisions clarify older assumptions in the design.
4. `docs/architecture.md` - current code boundaries and milestone order.
5. `docs/setup/development.md` - toolchain, debugging, testing, and release boundaries.
6. `docs/alignment-engine-plan.md` - focused attribution/alignment review, implementation increments, experiments, gates, and unsolved limits. Read before finalizing M2's evidence schema or starting M3.

`docs/design.md` is the single tracked copy of the design. Do not create a second copy. Update `docs/decisions.md` when implementation research resolves an ambiguity or requires a documented refinement.

## Current state

- Repository: `https://github.com/rohitmekkoth/picket`.
- Visibility: private. Do not make it public without the owner's explicit request.
- Primary branch: `main`. Fetch before starting new work. M5 merged from `feature/m5-google-collectors` in PR #11.
- M1 notification work: PR #5 merged with its two commits preserved. The post-merge [main CI run](https://github.com/rohitmekkoth/picket/actions/runs/39556842806) passed all three jobs.
- Product implementation status: M2 and M3 merged in PR #8, M4 in PR #9, and M5 in PR #11. M5 adds scoped live Gmail metadata and primary Calendar monitoring. Messages/OpenClaw adapters, the local model, incident views, notification delivery, and production alerts remain unimplemented.
- Available scaffold: Rust workspace, portable core boundary, diagnostic CLI, Tauri v2 window and menu-bar shell, Google collectors, synthetic native notification probe, React UI, tests, CI, dependency audits, and macOS smoke test.
- The browser preview intentionally cannot reach the Rust core. This is expected behavior.
- Runtime data, credentials, personal messages, account fixtures, databases, and model files must never be committed.

The runnable scaffold covers the M1 structure and acceptance checks. The M1 native notification investigation confirmed click-through and action callbacks in the packaged macOS app using `tauri-plugin-notification` 2.7. The test alert appeared in Notification Center, where tapping it and choosing This was expected opened the corresponding test views. A banner was not observed and remains an M8 UX check.

Local M1 verification passed: `make check`, `make test`, `make e2e`, `make audit`, `make smoke-macos`, and the two manual Notification Center interactions. The native probe uses synthetic text only.

## Milestone checklist

- [x] **M1 - Foundation:** Rust workspace, portable core boundary, CLI diagnostic, Tauri tray and window, React shell, CI, audits, macOS smoke test, and native notification feasibility.
- [x] **M2 - Event store foundation:** Typed events, canonical JSON, SQLite schema, ordered writer, hash chain, retention, and `picket verify` with exact broken-sequence reporting.
- [x] **M3 - Engine on mocks:** Deterministic rules, attribution and request states, task-linked action limits, claim/coverage checks, durable findings/outbox, scripted model and 24 labeled scenarios. This is synthetic validation only.
- [x] **M4 - Mock evaluation harness:** 288 fixed synthetic cases across 18 families, separate mode and family counts, JSON report, and a versioned CI regression gate. Real-source accuracy remains unmeasured.
- [x] **M5 - Google collectors:** Scoped OAuth, Keychain, allowlisted HTTP, Gmail and Calendar cursors, catch-up, and bounded baseline import. Larger Gmail baselines are marked incomplete.
- [ ] **M6 - Messages and OpenClaw:** Read-only agent-message capture and validated local plugin ingest.
- [ ] **M7 - Local AI:** Verified model download, bounded runtime, extraction and judging, and keyword fallback.
- [ ] **M8 - Product UI and alerts:** Setup wizard, incident review, settings, real notifications and digest, export, and wipe. Recheck banner presentation.
- [ ] **M9 - Release:** Packaging, release attestations, Homebrew tap, documentation, and fresh-Mac install test.

The milestone scopes and acceptance criteria in `docs/design.md`, with the alignment refinements and proposed release gates in `docs/alignment-engine-plan.md`, guide implementation. No accuracy targets have been measured yet.

## Alignment review and implementation progress, 2026-10-21

- Design review merged to `main` in PR #7 before implementation. The owner then authorized implementation on a new branch.
- Branch `feature/alignment-engine-foundation` starts at `6104d42`. Its M2/M3 work merged in PR #8 with individual commits preserved.
- The original additive matcher and actor confidence constants were replaced in the design. New plan: separate observations, actor evidence, request constraints and alert routing; preserve unknowns, contradictions, collection gaps and late corrections.
- Header fingerprints remain experimental. Runtime-linked OpenClaw evidence depends on runtime integrity. Current Workspace Gmail documentation supports further app-attribution investigation, but a working integration has not been demonstrated.
- The plan defines M2-M8 coding increments, source experiments, adversarial scenarios, precision/coverage gates, and cases with no solution under the current privacy/access constraints. Review these before implementing attribution.
- No empirical attribution accuracy is claimed. The synthetic engine leaves effects-only actors unknown, treats agent claims as reports, and requires an exact successful runtime/resource match for runtime linkage. Quantity is assessed only when validated runtime reports link the counted effects to one task, excluding unrelated human actions.

## Incremental assessment fix, 2026-10-27

- Branch `fix/incremental-assessment`, PR #13. A quality review found per-poll cost growing with the whole log (every poll logged a heartbeat, re-verified the chain and replayed the engine over all events), unbounded derived tables, and Gmail/Calendar backlogs that could repeat forever. The plan, reasons and rejected alternatives are in `docs/decisions.md` under "Incremental assessment and bounded collection".
- Verified locally: `make check`, `make test`, `make eval` (288), `make audit`, `make e2e` (6). PR CI passed all three jobs. Release probe on a 180-day synthetic log of 46,800 events: 0.12 ms per idle poll with no new events, 0.24 ms per poll with a new effect, 0.41 s cold start. A scratch copy of the local M5 database migrated with one reassessment, no queued alerts and unchanged verdicts; the copy was not committed.
- Not yet run: the packaged app against the controlled Google account (`make build`, launch, confirm the worker reaches monitoring, then a sent message and a Calendar change appear once, idle polls add no events, and `picket verify` passes). It restores the Keychain session and migrates the local database to schema v5, so it needs the owner's go-ahead.
- CI now runs on pushes to `main` and on pull requests only; open a draft pull request when starting a branch.

## Next work

After PR #13 merges, M6 is next. Begin by moving the portable collector logic from `picket-app` into `picket-core` behind a shared collector interface that commits through `Store::commit` and reports coverage instead of writing it. Fetch `main` and create a dedicated branch before implementation. Review the Messages and OpenClaw boundaries in `docs/design.md` and the attribution limits in `docs/alignment-engine-plan.md`. Keep Google credentials, account content, and runtime databases out of Git. Do not infer agent identity from timing or email headers.

M5 verification on `feature/m5-google-collectors`: `make check`, `make test`, `make eval` (288 cases), `make audit`, `make e2e` (6 cases), and `make smoke-macos` passed. All three PR #11 CI jobs passed. In a controlled Google test account, OAuth and Keychain restore worked; a sent message and primary Calendar create, update, and delete appeared as effects. Gmail and Calendar live coverage were healthy, Calendar recovered from an expired sync cursor, and the local event chain verified through 191 records. Replay showed four timeline effects with unknown actor and no false alerts. The metadata-only Gmail scope cannot date-filter message listings; M5 caps the first scan at 50 listed messages per label and records incomplete baseline coverage separately when more history exists. Real-source accuracy and agent attribution remain unmeasured.

M4 runs with `make eval` or `picket eval --mock --report path/to/report.json`. It prints a mode table and writes a privacy-safe JSON report. The fixed baseline fails all-abstain behavior and lost conflict recall. Its 288 variants are correlated synthetic cases; confidence intervals, real-source accuracy, extraction, latency and alerts per day remain `not_measured`. See `eval/README.md`.

M4 verification: `make eval`, `make check`, `make test`, `make e2e` (6 browser cases), and `make audit` passed locally. PR #9 and post-merge main CI passed. M4 adds no real-source validation or product monitoring.

M3 passed `make check`, `make test`, `make e2e` (6 Playwright cases), and `make audit` locally. The core tests include schema migration, replay/restart, transaction rollback, notification revision behavior, and 24 labeled scenarios. The first local E2E attempt failed to bind the dev port inside the sandbox; rerunning with local port access passed. No UI/native lifecycle code changed, so `make smoke-macos` is not required. The CLI replay reports privacy-safe counts and does not deliver alerts; the core's outbox has no OS delivery worker yet. Do not represent M3 as a finished monitoring product.

Repository housekeeping is separate from M2: Dependabot PR #2 proposes Node 26 type definitions while this project pins Node 24, so review it before merging. Its red branch status combines three cancelled checks with three passing checks; `main` CI is green. Older merged feature branches still exist on GitHub and can be deleted when convenient. The repository currently has no `main` branch protection; consider requiring CI checks before future merges.

## Resume checklist

From the repository root:

```sh
git status --short --branch
git fetch origin --prune
git switch main
git pull --ff-only
git log -5 --oneline --decorate
make doctor
make check
make test
make e2e
make audit
```

Run `make smoke-macos` when changing the desktop lifecycle, tray, native commands, packaging, or macOS integration. It requires Accessibility permission for the terminal that launches it.

Start the native scaffold with:

```sh
make dev
```

Start the browser-only UI with:

```sh
pnpm dev:ui
```

Build the packaged development app with:

```sh
make build
open target/release/bundle/macos/Picket.app
```

## Prompt for a new Codex chat

Copy this prompt into a new chat when ready to continue:

> Open the Picket repository at `/Users/rohit/Documents/Picket`. Read `AGENTS.md`, `HANDOFF.md`, `docs/design.md`, `docs/decisions.md`, `docs/architecture.md`, and `docs/setup/development.md` before making changes. Inspect Git and GitHub state, confirm `main` is clean and synchronized, and report any discrepancy. Continue with the next authorized milestone described in `HANDOFF.md`. Follow the small-commit feature-branch workflow, inspect every staged change for secrets and personal data, push each verified increment, and merge only after the branch is synchronized and all CI checks pass. Do not broaden product scope or add real credentials, account data, telemetry, permissions, network hosts, or model downloads without explicit authorization from the design and owner.

If the owner asks only for a status report or plan, do not start implementation.
