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
- Primary branch: `main`.
- Foundation baseline: merge commit `482bc24`.
- Product implementation status: no collectors, event store, detection engine, Google access, Messages access, OpenClaw integration, local model, or notifications are implemented.
- Available scaffold: Rust workspace, portable core boundary, diagnostic CLI, Tauri v2 window and menu-bar shell, React UI, tests, CI, dependency audits, and macOS smoke test.
- The browser preview intentionally cannot reach the Rust core. This is expected behavior.
- Runtime data, credentials, personal messages, account fixtures, databases, and model files must never be committed.

The runnable scaffold covers the M1 structure and acceptance checks. One M1 risk investigation remains: verify native macOS notification click-through and action-button behavior. After that, implementation proceeds in the design's order from M2 through M9.

## Next work

Do not begin feature implementation until the owner requests it. When authorized, take the following next increment:

1. Fetch `origin/main` and confirm local `main` is synchronized and clean.
2. Create a dedicated branch for the M1 notification feasibility investigation.
3. Prototype the narrowest native notification path needed to determine whether click-through and action buttons work from the Tauri app.
4. Record the result and chosen M8 implementation path in `docs/decisions.md`.
5. Add focused tests where they can verify behavior, then run the relevant checks and a manual end-user macOS flow.
6. Inspect staged files and commit metadata for secrets and machine-specific data before every push.
7. Push small, verified commits. Merge only after the branch is current with `main` and all configured CI checks pass.

After the notification investigation, begin M2 on a new `feature/event-store` branch. Resolve the four M2 questions already listed in `docs/decisions.md` before locking the schema:

- Calendar change identity for event deduplication.
- Replayable, privacy-minimized contact observations.
- Finding uniqueness when optional identifiers are absent.
- Retention checkpoints and stable feedback identity.

M2 must deliver the typed event schema, canonical JSON, SQLite migrations, one ordered writer, the hash chain, retention, and `picket verify`. Its acceptance test must demonstrate that editing or deleting a test row reports the exact first broken sequence number.

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
