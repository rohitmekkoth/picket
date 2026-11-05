# M8 product UI and alerts plan

Status: increments 1-3 merged (PRs #39-#41); increment 4 is next. v1 targets one agent, Instinct over iMessage. This plan implements M8 in [design.md](design.md) (user experience, UI and notifications, privacy defaults) with the user-flow gate in [alignment-engine-plan.md](alignment-engine-plan.md).

## Outcome

A person installs Picket, finishes a short setup, and then only hears from it when something needs attention. When Instinct does something that does not match what the person asked, a macOS notification appears; Review opens the incident with the request, Instinct's report and what actually happened in Gmail or Calendar side by side, the mismatch highlighted and the reason in plain words. The person can mark it expected, follow a checklist, or export a report. Settings cover the connection, model, digest, export and a full wipe.

Acceptance (design M8): on the controlled test account, a request to Instinct over iMessage leads to an email that becomes an incident showing the right request, claim and effects; uninstall and wipe leave no Picket files or Keychain items.

## What exists

- The engine produces findings with stable ids, routes (timeline, medium review, high review), revisions and a durable notification outbox (`pending_notifications`, `mark_notification_delivered`). Nothing delivers the outbox yet.
- Feedback events exist and the engine honours "expected" feedback for a finding. No command writes them.
- The window is one page (`ui/src/App.tsx`) with Google, agents and model panels, plain CSS, TanStack Query and Zod; no router. Seven Tauri commands.
- The native notification path (plugin 2.7, click-through and actions handled in Rust) was proven in M1 with a synthetic probe. Banner presentation is unchecked.

## Decisions

1. **An incident is a review-routed finding.** No new incident table: an incident is a finding whose current revision routes to medium or high review, with its revision history. A portable read model in `picket-core` joins the finding with its request, claim and effect events into display rows (Request / Claim / Effects, mismatch fields, reasons, coverage and actor evidence). It is tested against the 24 labeled scenarios, so the UI shows exactly what the engine decided.
2. **User decisions are events.** "This was expected", "Not expected" and "Mark agent revoked" append events through the single writer, validated in Rust. They never edit findings; the engine reassesses, which keeps history, replay and the hash chain intact. "Expected" resolves that finding only; it does not teach contacts, actors or thresholds.
3. **Delivery in Rust, at least once, deduplicated by id.** A delivery worker reads the outbox, applies the design's rate limits (high immediately, medium grouped at most once per 10 minutes, low digest only), posts the notification with the finding id as its identifier and marks it delivered after posting. A crash between posting and marking can repeat a notification; the same identifier replaces it in Notification Center rather than adding a second one. A revised, already-delivered finding posts a correction. Notification actions are handled in Rust so they work with the window hidden.
4. **Pause stops alerts, not collection.** "Pause for 1 hour" holds notification delivery and greys the tray icon; collection and assessment continue, so nothing is missed and held alerts deliver when the pause ends. Rejected: pausing collectors, which creates coverage gaps for the time when the person may be least attentive.
5. **Settings live in a small validated table** in the existing database (digest time, retention days, start at login, notifications paused until, wizard progress), not in the event log, because they are preferences rather than evidence. Wipe removes them with the database.
6. **Files are written only by Rust.** Incident export (Markdown) and full export (JSON) open a native save dialog from Rust and write there; the webview gets no filesystem, shell or HTTP permission. Export content is the person's own data and is never logged.
7. **Wipe is verified, not assumed.** Uninstall and wipe stops the workers, then deletes the database and its WAL files, the model, logs, settings and Picket's Keychain items, removes the login item, opens Google's third-party access page in the system browser and quits. A test script lists every path and Keychain entry Picket creates and checks each is gone. macOS privacy grants (Full Disk Access, notifications) cannot be removed by an app; the UI and README say so.
8. **Styling stays plain CSS with design tokens** instead of adding Tailwind as the design lists. The existing UI is plain CSS, the app has one window and few screens, and tokens give light and dark themes and contrast checks without a new build step. A small hash router (`#/overview`, `#/incident/<id>`, ...) replaces a router library; notification click-through already uses hash routes.
9. **v1 scope cuts, all toward Instinct only.** No OpenClaw step in the wizard or plugin status. Agents are iMessage or effects-only. No raw YAML view. The Timeline (Activity) is a paged list with filters, not virtualized until history sizes need it.
10. **Reconnect required is a state, not a retry loop.** A rejected Google grant stops the 30-second retries, sets a "reconnect required" state shown in the tray, Overview and Settings, and offers one-click sign-in.
11. **Wording follows the known limits.** The UI never says requests were fully captured: request fit is shown with its coverage ("in your iMessage conversation with Instinct"), unknown stays unknown, and severity is shown separately from evidence strength.
12. **Rules are granular, with one loudness control everywhere** (owner, 2026-11-04). Every rule has the same four levels: Urgent (notify now), Review (notify, grouped at most every 10 minutes), Summary (digest only) and Off (recorded, never flagged). Rules are grouped by what the person worries about:
    - *When Instinct doesn't do what you asked*: one row per conflict reason the engine already produces (prohibited action, missing approval, extra target, different time, too many actions, different action, outside window) and per claim check (not observed, contradicted). These apply only when the actor is Instinct by its report.
    - *Your boundaries*: account policies written as sentences: never contact a person or domain, never delete or move calendar events, quiet hours, bursts (more than N in a window), and email to people not seen in the baseline. They apply whoever acted. Each shows how often it would have triggered in the last 30 days, computed by replaying the engine over retained history with the edited rule, before saving.
    - *Account warnings* (forwarding, app access, codes, purchases) and *When Picket can't see everything* (Google access, Messages access, sleep catch-up).

    This needs engine work: a level per conflict reason and claim status instead of fixed routes; domain targets, calendar change kinds, quiet hours, burst limits and new-recipient policies; routing for notice kinds. Rule changes are versioned policy events, so a change reassesses findings and history shows which rule version flagged what. Purchase amount limits are not offered: Picket does not read amounts, and guessing from subjects would be unreliable. The design reference is the [M8 screens canvas](https://claude.ai/artifact/Rrc1oB7kfb8x8Y61znVtnB).

## Increments

Each is its own branch and PR, verified with `make check`, `make test`, `make e2e` and, when native code changes, `make smoke-macos`, and checked by eye in the packaged app.

1. **Incident model and decisions** (`feature/m8-incidents`). Core read model, list/detail/resolve/revoke commands, events for decisions, scenario tests. No new screens beyond a debug list.
2. **Rule levels and boundaries** (`feature/m8-rules-engine`). Per-reason levels, the new boundary kinds and their 30-day preview in the engine, with scenario tests for each; no UI yet.
3. **App shell and review screens** (`feature/m8-review-ui`). Hash router and navigation; Overview (today / 7 / 30 days, agent counts, open items, coverage and integrity), Incident detail, Activity, Rules and the boundary editor; live refresh from core events. Existing panels move into Settings.
4. **Alerts** (`feature/m8-alerts`). Delivery worker with rate limits, corrections and actions; click-through to the incident; tray icon states and menu (counts, up to three open alerts, Pause 1 h); notification permission request; banner presentation check.
5. **Settings, digest and export** (`feature/m8-settings`). Settings table and screen, daily digest notification, start at login, retention, verify log, incident Markdown and full JSON export, reconnect required.
6. **Setup wizard** (`feature/m8-setup`). Welcome, Connect Google (one-click sign-in with Picket's own client; own client as an advanced option, see decisions), Add Instinct, Full Disk Access with polling, baseline import and model download with progress; resumable, each step skippable and revisitable.
7. **Uninstall and wipe** (`feature/m8-wipe`). Wipe command and the verification script.
8. **Acceptance run and polish** (`feature/m8-acceptance`). Scripted live run with Instinct on the controlled account (owner sends the iMessages), a full visual pass in light and dark at the default and minimum window sizes, wipe check on a scratch macOS user if available, handoff.

## Visual design

No earlier mockups exist (owner, 2026-11-04). The [M8 screens canvas](https://claude.ai/artifact/Rrc1oB7kfb8x8Y61znVtnB) is the reference: a warm neutral ground, the system font, one blue for actions, amber for Review and red for Urgent, always paired with a text label. Copy is plain sentences in the person's terms ("Contacted someone you didn't mention"), never engine terms. Light and dark use the same tokens.

## Risks

- Banner presentation was not observed in M1; if macOS shows alerts only in Notification Center for an ad hoc signed app, the fallback is the tray icon state and a documented setting, with Developer ID signing in M9.
- Full Disk Access is lost on ad hoc rebuilds until M9 signing; the wizard's polling step must handle a revoked grant cleanly.
- Google sign-ins in Testing mode expire weekly until the OAuth app is published in M9; reconnect required makes this visible rather than silent.
