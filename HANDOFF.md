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
- Visibility: public on GitHub as of 2026-11-05 (the privacy policy is linked from the Google consent screen). Do not change visibility without the owner's explicit request.
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
- [x] **M6 - Messages:** Read-only agent-message capture (PR #15). OpenClaw is out of v1 scope; its adapter is kept on `feature/m6-openclaw`.
- [x] **M7 - Local AI:** Verified model download, bounded runtime, extraction and judging; without the model, request capture is reported unavailable (no keyword fallback, see decisions). Closed for v1 on 2026-11-04 with known wording limits (see decisions).
- [x] **M8 - Product UI and alerts:** Setup wizard, incident review, settings, real notifications and daily summary, export, and wipe. Closed 2026-11-10 after the live acceptance run (PRs #39-#43, #46-#48).
- [x] **M9 - Release:** v1.0.0 published 2026-11-10: signed DMG and app zip with SHA256SUMS and build provenance on GitHub Releases, cask `rohitmekkoth/tap/picket`, MIT license, user README. Remaining: the fresh-Mac install test (owner, on another Mac).

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
- Live check with the packaged app and the controlled Google account, approved by the owner: the local database migrated to schema v5 with one reassessment and no new alerts; the worker reached monitoring; 39 catch-up contact observations and no coverage events were added on startup; three idle minutes of polling added no events or revisions; one sent email and one Calendar create each appeared once; a restart revised no findings; `picket verify` passed through sequence 1150. The check found that Gmail web draft autosaves produced a false `missing_resource` gap and review alert for each sent email; that is fixed and reverified live. The pending baseline-gap alert from 2026-10-22 is the expected M5 bounded-baseline notice. A pre-check backup of the local database is outside the repository.
- CI now runs on pushes to `main` and on pull requests only; open a draft pull request when starting a branch.

## M6 progress

- [M6 plan](docs/m6-plan.md) records the verified upstream facts, decisions and increments. The owner chose the own minimal `attributedBody` decoder, approved OpenClaw for E2 with a local model and the controlled test account only, and has Instinct for E4.
- Increment 1, `refactor/collector-writer` (PR #14): Google wire types and polling moved into `picket-core`; a single writer thread with per-collector `Writer` handles owns commits, the assessment tick and daily retention. All gates passed; a live run polled both sources through the writer with no warnings, added only inbox contact observations while idle, and settled a deadline pending from the previous run exactly once.

- Increment 2, `feature/m6-messages` (PR #15): agent registrations, the read-only `chat.db` reader with its own `attributedBody` decoder, the Messages worker with Full Disk Access detection, a minimal Agents panel, and `request_not_captured` until request extraction confirms capture. E4 with Instinct passed; results are in `docs/decisions.md`. Picket.app needs Full Disk Access; ad hoc rebuilds may lose the grant.

## M8 progress

- Plan: [docs/m8-plan.md](docs/m8-plan.md); visual reference: the M8 screens canvas linked there. Decisions: `docs/decisions.md`, "M8 incident model and decisions", "M8 rule levels and boundaries" and "M8 alert delivery and menu bar".
- Increment 1 (PR #39, merged): incident views from assessed evidence, decisions as feedback events, agent revocation.
- Increment 2 (PR #40, merged): per-rule levels (Urgent, Review, Summary, Off) for every finding kind; boundaries (contacts and domains, Calendar changes, quiet hours, bursts, new recipients) with a 30-day preview; quiet reassessment for user configuration and engine upgrades (engine version 3).
- Increment 3 (PR #41, merged): Overview counts and Activity timeline; the product screens; fixed-destination link opener (`open_link`); generated UI contract (`ui/src/lib/contract.json`, refresh with `PICKET_UPDATE_CONTRACT=1 cargo test -p picket-core --test ui_contract`); Playwright screens against a synthetic IPC core (`e2e/desktop.ts`).
- Increment 4 (PR #42): alert delivery planned in `picket-core` (`alerts`), posted by an app thread with Review and This was fine handled in Rust; outbox schema v7 stores delivery time and pacing; drawn tray icon states and status menu; `--background` launch; M1 probe removed. Banner presentation checked by the owner: banners show while Picket is in the background, Notification Center only while its window is frontmost (see decisions, "M8 alert delivery and menu bar").
- Increment 5 (PR #43): settings table (schema v8) with daily summary time, retention, persisted pause; daily summary notification; open at login (LaunchAgent `~/Library/LaunchAgents/Picket.plist` with `--background`); verify log; incident Markdown report and full JSON export through a native save panel; "reconnect required" with one-click sign-in. Owner checked Settings, export, report and login item in the packaged app on 2026-11-05; the daily summary was not live-checked yet.
- Increment 7, wipe (branch `feature/m8-wipe`, PR opened 2026-11-05): Settings > Uninstall and wipe plus `scripts/verify-wipe.sh`. Verified live on the owner's account: the script passed, and the database and model were restored from a backup (log valid through sequence 1658). The wipe removed the Google sign-in and the login item, as designed.
- Increment 6, setup wizard (branch `feature/m8-setup`, stacked on the wipe branch, draft PR): six skippable steps, one-click sign-in with Picket's own client injected at build time from `local/google-client.env` (present on the owner's Mac; never commit it), schema v9 `setup_completed_at`. Live check in progress: the owner got past Google's unverified-app warning (Advanced, then Go to Picket), so publishing unverified works with `gmail.metadata`. The browser then showed "127.0.0.1 refused to connect" because the loopback listener stopped waiting after 3 minutes; commit `24e5bec` waits 15 minutes. Not yet re-verified live.
- Google client (owner decision 2026-11-04): Picket ships its own OAuth client, published to production but unverified; "use my own client" stays as an advanced option. Details and costs in `docs/decisions.md`.

- Increment 6, setup wizard (PR #47) and increment 8, acceptance (PR #48), merged 2026-11-10. The live run on the controlled account passed: one-click sign-in through the unverified-app warning, Full Disk Access detected without a restart, an Urgent boundary banner with Picket in the background, Review opening the incident, This was fine, and the daily summary posting at its time. It found three extraction defects, now fixed with regression tests: a 30-day chat backlog delayed new requests (older-than-24-hour messages are now skipped), the model copied an earlier "hold off" onto a new request (prohibitions now need restricting words in the message), and display-name recipients showed as unrequested (a yes to Instinct's "Send this to <address>?" now approves that address). Real-model suites after the fixes: 0 safety errors on both (8 and 3 utility misses). See `docs/decisions.md`, "M8 acceptance run".
- CI's macOS job now runs on `macos-15`; GitHub retired `macos-14`.

## Next work

v1.0.0 is released (2026-11-10). Open items:

1. Fresh-Mac install test (owner, on someone else's Mac): `brew install --cask rohitmekkoth/tap/picket`, Open Anyway, setup wizard, one Instinct request; time it against the design's 10 minutes. Fix anything found in a v1.0.1.
2. Dependabot: close #37 (`@types/node` 26; Node 24 is pinned), defer #24 (`mail-parser` 1.0 rewrites the parser API used for Gmail headers), rebase and merge #36 (Biome 2.6.4).
3. Consider branch protection on `main` requiring the three CI jobs.

To release a new version: bump `version` in `Cargo.toml` (workspace and the two internal pins), `crates/app/tauri.conf.json` and `package.json`, update `docs/release-notes.md`, merge, then push a matching `v*` tag. Signing material is in the git-ignored `local/signing/` (back it up; losing it changes the app identity and resets every user's Full Disk Access). See `docs/decisions.md`, "M9 release", and "After v1" below.

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

## After v1 (owner, 2026-11-09)

- Tell the person's own actions from the agent's. Gmail metadata cannot; options are a gateway in the agent's path, runtime hooks, a separate agent identity (own mailbox or send-as alias), or Workspace audit logs. Until then boundaries flag reported and unattributed effects alike.
- Extraction robustness beyond plain wording (M7 known limits).
