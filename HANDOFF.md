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
- Primary branch: `main`. Fetch before starting new work; M4 is developed on `feature/m4-evaluation-harness`.
- M1 notification work: PR #5 merged with its two commits preserved. The post-merge [main CI run](https://github.com/rohitmekkoth/picket/actions/runs/39556842806) passed all three jobs.
- Product implementation status: M2 and M3 are merged in PR #8. M4's mock evaluation harness is on `feature/m4-evaluation-harness`. No collectors, real account access, Messages/OpenClaw adapters, local model, app incident views, notification delivery, or production alerts are implemented.
- Available scaffold: Rust workspace, portable core boundary, diagnostic CLI, Tauri v2 window and menu-bar shell, synthetic native notification probe, React UI, tests, CI, dependency audits, and macOS smoke test.
- The browser preview intentionally cannot reach the Rust core. This is expected behavior.
- Runtime data, credentials, personal messages, account fixtures, databases, and model files must never be committed.

The runnable scaffold covers the M1 structure and acceptance checks. The M1 native notification investigation confirmed click-through and action callbacks in the packaged macOS app using `tauri-plugin-notification` 2.7. The test alert appeared in Notification Center, where tapping it and choosing This was expected opened the corresponding test views. A banner was not observed and remains an M8 UX check.

Local M1 verification passed: `make check`, `make test`, `make e2e`, `make audit`, `make smoke-macos`, and the two manual Notification Center interactions. The native probe uses synthetic text only.

## Milestone checklist

- [x] **M1 - Foundation:** Rust workspace, portable core boundary, CLI diagnostic, Tauri tray and window, React shell, CI, audits, macOS smoke test, and native notification feasibility.
- [x] **M2 - Event store foundation:** Typed events, canonical JSON, SQLite schema, ordered writer, hash chain, retention, and `picket verify` with exact broken-sequence reporting. Collector-specific observation fields still need refinement before M5.
- [x] **M3 - Engine on mocks:** Deterministic rules, attribution and request states, task-linked action limits, claim/coverage checks, durable findings/outbox, scripted model and 24 labeled scenarios. This is synthetic validation only.
- [x] **M4 - Mock evaluation harness:** 288 fixed synthetic cases across 18 families, separate mode and family counts, JSON report, and a versioned CI regression gate. Real-source accuracy remains unmeasured.
- [ ] **M5 - Google collectors:** Scoped OAuth, Keychain, allowlisted HTTP, Gmail and Calendar cursors, catch-up, and baseline import.
- [ ] **M6 - Messages and OpenClaw:** Read-only agent-message capture and validated local plugin ingest.
- [ ] **M7 - Local AI:** Verified model download, bounded runtime, extraction and judging, and keyword fallback.
- [ ] **M8 - Product UI and alerts:** Setup wizard, incident review, settings, real notifications and digest, export, and wipe. Recheck banner presentation.
- [ ] **M9 - Release:** Packaging, release attestations, Homebrew tap, documentation, and fresh-Mac install test.

The milestone scopes and acceptance criteria in `docs/design.md`, with the alignment refinements and proposed release gates in `docs/alignment-engine-plan.md`, guide implementation. No accuracy targets have been measured yet.

## Alignment review and implementation progress, 2026-10-21

- Design review merged to `main` in PR #7 before implementation. The owner then authorized implementation on a new branch.
- Branch `feature/alignment-engine-foundation` starts at `6104d42`. Pushed commits: `25b2be9`, `29a976e`, `8eca50c`, `2b2a68e` (M2 and M3 foundation); `9a6025f` (task constraints); `df0408c` (coverage and claims); `7be7e3e` (incidents and outbox); `c45eff5` (coverage findings and 24 scenarios). Check the PR history for the merge result.
- The original additive matcher and actor confidence constants were replaced in the design. New plan: separate observations, actor evidence, request constraints and alert routing; preserve unknowns, contradictions, collection gaps and late corrections.
- Header fingerprints remain experimental. Runtime-linked OpenClaw evidence depends on runtime integrity. Current Workspace Gmail documentation supports further app-attribution investigation, but a working integration has not been demonstrated.
- The plan defines M2-M8 coding increments, source experiments, adversarial scenarios, precision/coverage gates, and cases with no solution under the current privacy/access constraints. Review these before implementing attribution.
- No empirical attribution accuracy is claimed. The synthetic engine leaves effects-only actors unknown, treats agent claims as reports, and requires an exact successful runtime/resource match for runtime linkage. Quantity is assessed only when validated runtime reports link the counted effects to one task, excluding unrelated human actions.

## Next work

Check whether the M4 PR has merged. If it is still open, fetch `main`, resolve any conflicts, run full checks, require green CI, and merge. Then M5 is next: Google OAuth and Gmail/Calendar collectors with observed-source gap semantics. Run controlled source capability experiments during M5/M6, not by inferring identities from timing or email headers.

M4 runs with `make eval` or `picket eval --mock --report path/to/report.json`. It prints a mode table and writes a privacy-safe JSON report. The fixed baseline fails all-abstain behavior and lost conflict recall. Its 288 variants are correlated synthetic cases; confidence intervals, real-source accuracy, extraction, latency and alerts per day remain `not_measured`. See `eval/README.md`.

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
