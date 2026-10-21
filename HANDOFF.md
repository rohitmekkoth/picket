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
- Primary branch: `main`. The alignment design review merged at `6104d42`; fetch before starting new work.
- M1 notification work: PR #5 merged with its two commits preserved. The post-merge [main CI run](https://github.com/rohitmekkoth/picket/actions/runs/39556842806) passed all three jobs.
- Product implementation status: M2 event storage and an initial synthetic M3 matcher exist on `feature/alignment-engine-foundation`. No collectors, real account access, Messages/OpenClaw adapters, local model, persisted incidents, or production alerts are implemented.
- Available scaffold: Rust workspace, portable core boundary, diagnostic CLI, Tauri v2 window and menu-bar shell, synthetic native notification probe, React UI, tests, CI, dependency audits, and macOS smoke test.
- The browser preview intentionally cannot reach the Rust core. This is expected behavior.
- Runtime data, credentials, personal messages, account fixtures, databases, and model files must never be committed.

The runnable scaffold covers the M1 structure and acceptance checks. The M1 native notification investigation confirmed click-through and action callbacks in the packaged macOS app using `tauri-plugin-notification` 2.7. The test alert appeared in Notification Center, where tapping it and choosing This was expected opened the corresponding test views. A banner was not observed and remains an M8 UX check.

Local M1 verification passed: `make check`, `make test`, `make e2e`, `make audit`, `make smoke-macos`, and the two manual Notification Center interactions. The native probe uses synthetic text only.

## Milestone checklist

- [x] **M1 - Foundation:** Rust workspace, portable core boundary, CLI diagnostic, Tauri tray and window, React shell, CI, audits, macOS smoke test, and native notification feasibility.
- [x] **M2 - Event store foundation:** Typed events, canonical JSON, SQLite schema, ordered writer, hash chain, retention, and `picket verify` with exact broken-sequence reporting. Collector-specific observation fields still need refinement before M5.
- [ ] **M3 - Engine on mocks:** Initial deterministic matching, attribution states, explicit policy checks, request gates, replay CLI, and synthetic cases exist. Persisted incidents/outbox, complete task constraints, coverage logic, and a labeled scenario set remain.
- [ ] **M4 - Evaluation harness:** Scenario replay, metrics and report, plus a CI regression gate against a real baseline.
- [ ] **M5 - Google collectors:** Scoped OAuth, Keychain, allowlisted HTTP, Gmail and Calendar cursors, catch-up, and baseline import.
- [ ] **M6 - Messages and OpenClaw:** Read-only agent-message capture and validated local plugin ingest.
- [ ] **M7 - Local AI:** Verified model download, bounded runtime, extraction and judging, and keyword fallback.
- [ ] **M8 - Product UI and alerts:** Setup wizard, incident review, settings, real notifications and digest, export, and wipe. Recheck banner presentation.
- [ ] **M9 - Release:** Packaging, release attestations, Homebrew tap, documentation, and fresh-Mac install test.

The milestone scopes and acceptance criteria in `docs/design.md`, with the alignment refinements and proposed release gates in `docs/alignment-engine-plan.md`, guide implementation. No accuracy targets have been measured yet.

## Alignment review and implementation progress, 2026-10-21

- Design review merged to `main` in PR #7 before implementation. The owner then authorized implementation on a new branch.
- Branch `feature/alignment-engine-foundation` starts at `6104d42`. Pushed commits: `25b2be9` (M2 store), `29a976e` (synthetic matcher and replay CLI), and `8eca50c` (abstain when a claim could refer to multiple similar effects). [CI for the latest code commit](https://github.com/rohitmekkoth/picket/actions/runs/39654005019) passed all three jobs.
- The original additive matcher and actor confidence constants were replaced in the design. New plan: separate observations, actor evidence, request constraints and alert routing; preserve unknowns, contradictions, collection gaps and late corrections.
- Header fingerprints remain experimental. Runtime-linked OpenClaw evidence depends on runtime integrity. Current Workspace Gmail documentation supports further app-attribution investigation, but a working integration has not been demonstrated.
- The plan defines M2-M8 coding increments, source experiments, adversarial scenarios, precision/coverage gates, and cases with no solution under the current privacy/access constraints. Review these before implementing attribution.
- No empirical attribution accuracy is claimed. The synthetic engine deliberately leaves effects-only actors unknown, treats agent claims as reports, and requires an exact successful runtime/resource match for runtime linkage. Quantity remains unassessed until task slots can exclude human actions.

## Next work

Continue on `feature/alignment-engine-foundation`. Verify branch CI and complete M3 in small increments: retain and replay task/evidence provenance with bounded coverage states, implement task-linked action slots and condition checks, persist assessment revisions and a transactional notification outbox, then add claim consistency checks and 20 labeled adversarial scenarios. Build the M4 harness with separate safety and utility gates before any real account integration. Existing core tests are synthetic only; the CLI replay reports privacy-safe counts and does not deliver alerts. Run source capability experiments during M5/M6, not by inferring identities from timing or email headers.

The branch passed `make check`, `make test`, and `make audit` locally for the matcher increment; the final attribution fix passed focused core clippy/tests. GitHub CI passed frontend end-to-end, portable core, and macOS packaging checks on the latest code commit. No UI/native lifecycle code changed, so `make smoke-macos` was not rerun locally. Before any merge, fetch `main`, integrate changes, run full branch checks, and require green CI. Do not represent this foundation as a finished monitoring product.

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
