# Picket development handoff

Use this file to resume work in a new Codex chat without reconstructing project history.

## Read first

Read these files in order before changing code:

1. `AGENTS.md` - mandatory repository and Git workflow instructions.
2. `docs/design.md` - authoritative product scope, architecture, security constraints, milestones, and acceptance criteria.
3. `docs/decisions.md` - reviewed refinements and unresolved questions. These decisions clarify older assumptions in the design.
4. `docs/architecture.md` - current code boundaries and milestone order.
5. `docs/setup/development.md` - toolchain, debugging, testing, and release boundaries.

`docs/design.md` is the single tracked copy of the design. Do not create a second copy. Update `docs/decisions.md` when implementation research resolves an ambiguity or requires a documented refinement.

## Current state

- Repository: `https://github.com/rohitmekkoth/picket`.
- Visibility: private. Do not make it public without the owner's explicit request.
- Primary branch: `main`. The M1 product work merged at `bf917f2`; fetch before starting new work.
- M1 notification work: PR #5 merged with its two commits preserved. The post-merge [main CI run](https://github.com/rohitmekkoth/picket/actions/runs/39556842806) passed all three jobs.
- Product implementation status: no collectors, event store, detection engine, Google access, Messages access, OpenClaw integration, local model, or production alerts are implemented.
- Available scaffold: Rust workspace, portable core boundary, diagnostic CLI, Tauri v2 window and menu-bar shell, synthetic native notification probe, React UI, tests, CI, dependency audits, and macOS smoke test.
- The browser preview intentionally cannot reach the Rust core. This is expected behavior.
- Runtime data, credentials, personal messages, account fixtures, databases, and model files must never be committed.

The runnable scaffold covers the M1 structure and acceptance checks. The M1 native notification investigation confirmed click-through and action callbacks in the packaged macOS app using `tauri-plugin-notification` 2.7. The test alert appeared in Notification Center, where tapping it and choosing This was expected opened the corresponding test views. A banner was not observed and remains an M8 UX check.

Local M1 verification passed: `make check`, `make test`, `make e2e`, `make audit`, `make smoke-macos`, and the two manual Notification Center interactions. The native probe uses synthetic text only.

## Milestone checklist

- [x] **M1 - Foundation:** Rust workspace, portable core boundary, CLI diagnostic, Tauri tray and window, React shell, CI, audits, macOS smoke test, and native notification feasibility.
- [ ] **M2 - Event store:** Typed events, canonical JSON, SQLite migrations, ordered writer, hash chain, retention, and `picket verify` with exact broken-sequence reporting.
- [ ] **M3 - Engine on mocks:** Rules, matching, attribution, incidents, mock model, and 20 labeled seed scenarios.
- [ ] **M4 - Evaluation harness:** Scenario replay, metrics and report, plus a CI regression gate against a real baseline.
- [ ] **M5 - Google collectors:** Scoped OAuth, Keychain, allowlisted HTTP, Gmail and Calendar cursors, catch-up, and baseline import.
- [ ] **M6 - Messages and OpenClaw:** Read-only agent-message capture and validated local plugin ingest.
- [ ] **M7 - Local AI:** Verified model download, bounded runtime, extraction and judging, and keyword fallback.
- [ ] **M8 - Product UI and alerts:** Setup wizard, incident review, settings, real notifications and digest, export, and wipe. Recheck banner presentation.
- [ ] **M9 - Release:** Packaging, release attestations, Homebrew tap, documentation, and fresh-Mac install test.

The milestone scopes and acceptance criteria in `docs/design.md` remain authoritative.

## Next work

The next product milestone is M2. Do not begin it until the owner requests it. When authorized:

1. Fetch `origin/main` and confirm local `main` is synchronized and clean.
2. Create a dedicated `feature/event-store` branch from current `main`.
3. Resolve the four M2 questions listed in `docs/decisions.md` before locking the schema.
4. Build typed events, canonical JSON, SQLite migrations, an ordered writer, the hash chain, retention, and `picket verify`.
5. Verify that editing or deleting a test row reports the exact first broken sequence number; run the relevant checks.
6. Inspect staged files and commit metadata for secrets and machine-specific data before every push.
7. Push small, verified commits. Merge only after the branch is current with `main` and all configured CI checks pass.

Resolve these four schema questions before writing the M2 migration:

- Calendar change identity for event deduplication.
- Replayable, privacy-minimized contact observations.
- Finding uniqueness when optional identifiers are absent.
- Retention checkpoints and stable feedback identity.

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
