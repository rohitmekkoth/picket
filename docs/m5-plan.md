# M5 Google observation plan

Status: implemented on `feature/m5-google-collectors`, 2026-10-22; controlled-account acceptance remains pending. This plan implements the M5 scope in [design.md](design.md) and the evidence rules in [alignment-engine-plan.md](alignment-engine-plan.md). It does not add new Google scopes, hosts, account types, or actor attribution claims.

## Outcome and boundaries

Picket will observe Gmail metadata and changes to the primary Google Calendar while the Mac is awake, resume after sleep or network failure, and make uncertainty and collection gaps durable. It will use only `gmail.metadata` and `calendar.events.readonly`. It will not read bodies or attachments, write account data, infer a sending app from headers or time, or label an external Calendar edit as an agent action.

M5 is complete only when the packaged app can authenticate, collect, persist, and replay Google observations end to end, and controlled test-account changes appear within one poll. Synthetic fixtures remain in Git; account data, credentials, and raw research output stay outside Git.

## Design decisions to validate first

1. **Identity and provenance.** Extend the event contract with validated source time quality, adapter/version identity, scope, collection origin, and loss or partial-page indicators. Use a local opaque account ID in events and diagnostics, bound privately to the account returned by Google's allowed APIs. The approved scopes do not include OpenID identity. Gmail message IDs and history IDs are different concepts; Calendar event IDs need a meaningful revision. Exact retries must deduplicate, while distinct revisions remain visible.
2. **Cursor transaction.** Persist minimized observations, the new cursor, and coverage state in one ordered writer transaction. A crash before commit must replay a page; a crash after commit must not lose or duplicate effects. Never advance a cursor past an unprocessed page or failed resource fetch.
3. **Calendar bootstrap versus sync.** The documented Calendar `events.list` API forbids `timeMin` and `timeMax` with `syncToken`. Treat the bounded -30/+90 day baseline and the unfiltered token-bearing initial sync as separate operations. The latter must page to `nextSyncToken` before claiming healthy coverage. Filter stored observations to the supported window, retain the minimum snapshot state needed to classify later revisions, and periodically refresh the rolling window. Test recurring events, cancelled instances, moves into and out of the window, and large calendars. If a complete token-bearing sync cannot finish, report incomplete coverage rather than pretending the bounded baseline is a complete incremental cursor.
4. **Resync limits.** Gmail history expiry (404) and Calendar token expiry (410) invalidate continuity. Rebuild a bounded current-state view, deduplicate known resources, and emit an explicit unrecoverable-gap interval. A fresh snapshot cannot prove that transient actions during the gap did not occur.
5. **Account and scope validation.** Verify the granted scopes and stable account identity before starting collectors. A missing scope, changed account, revoked token, or Keychain failure disables that collector with a clear health state. Never silently collect under a different account.

## Implementation increments

### 1. Evidence contract and writer

- Add typed adapter provenance, source-time quality, and bounded coverage details to the portable core. Preserve existing log verification and replay compatibility through an explicit schema/version migration.
- Add durable per-account collector cursors and the minimum resource snapshot state required for Calendar revision diffing. Keep network and Keychain code outside `crates/core`; use portable interfaces and injected clock/transport for tests.
- Reject oversized or malformed adapter inputs, invalid revisions, inconsistent account IDs, and conflicting duplicate identities. Redact sensitive values from errors and diagnostics. Add size-based log rotation before real account collection.
- Verify crash points, retention/checkpoint behavior, replay determinism, and no duplicate findings on restart.

### 2. Authentication and transport

- Add system-browser desktop OAuth with a temporary loopback listener, PKCE S256, random state, exact redirect matching, timeout, and only the two approved scopes. Validate the token response, handle refresh and revocation, and store refresh credentials in macOS Keychain. Keep access tokens in memory when possible.
- Use a single HTTP client with TLS, exact scheme and host allowlist, bounded redirects, timeouts, response limits, rate-limit handling, jittered backoff, and cancellable requests. Allow only the Google authorization/token and Gmail/Calendar API endpoints required by the design. Never hand arbitrary URLs or bearer tokens to the webview.
- Provide a test transport that returns recorded synthetic wire responses. Verify no secret, email address, subject, or token reaches logs, errors, or fixture files.

### 3. Gmail collector

- Bootstrap SENT and INBOX with metadata-only message reads and a 30-day client-side bound. Pin a mailbox history cursor in a way that covers changes during pagination; process every history page before moving it. Poll every 45 seconds while awake and promptly on wake or network return.
- Fetch only approved headers and labels. Persist minimized Sent observations with baseline, live, or catch-up origin; do not call a Sent record proof of delivery or of the sending app. Store received-contact observations, not ordinary incoming-mail events.
- Derive qualified purchase, forwarding, app-access, and OTP indicators only from metadata. Do not claim an actual purchase, changed setting, or granted app access from a subject line.
- On 404, bounded rescan and explicit gap. Cover duplicate history entries, missing/deleted messages, label transitions, partial pages, throttling, and restart.

### 4. Calendar collector

- Read only the primary calendar with `calendar.events.readonly`. Import the -30/+90 day baseline separately from a token-bearing initial sync, then poll with `syncToken`, `showDeleted=true`, and compatible fixed parameters every 60 seconds while awake.
- Diff revisions to emit create, update, delete, and self-RSVP observations. Preserve organizer and creator context and mark externally initiated changes as uncertain or external where the API supports it. Handle recurring instances and all-day timezone boundaries explicitly.
- On 410, rebuild current state, record the lost interval, and start a new token-bearing sync. Check all pages, including a last page with no events, before committing the new token.

### 5. Runtime and visibility

- Start collectors only after validated account authorization. Isolate their failures, restart with bounded exponential backoff, and trigger immediate catch-up after wake or connectivity recovery. Keep one writer and bounded queues; overflow becomes a coverage event.
- Feed validated events through the existing store and engine. A healthy coverage event means the named source and interval were actually processed; a baseline import cannot satisfy live coverage.
- Surface connection, last successful poll, source limitations, and gaps through the existing native boundary without exposing a general HTTP or filesystem capability to the webview. Product UI and real alert delivery remain M8.

## Verification gates

1. **Contract and security:** unit and property-style cases for identity, revisions, old log replay, privacy bounds, OAuth state/PKCE/redirect/scope checks, host rejection, response-size limits, Keychain failures, and redacted diagnostics.
2. **Wire fixtures:** synthetic Gmail and Calendar JSON for pagination, races during bootstrap, 429/5xx retry, partial failure, duplicate delivery, restart at each page boundary, expired cursor, recurrence, deletion, external organizer update, and rolling-window movement. Confirm cursor and observations commit together.
3. **Native end to end:** packaged-app sign-in with a controlled test account, one synthetic sent email, one Calendar create/update/delete and RSVP, sleep/wake and network-return catch-up. Measure source-change-to-event latency against one poll; inspect UI health and coverage wording. Do not commit the account's responses.
4. **Repository gates:** focused tests after each increment, then `make check`, `make test`, `make eval`, `make e2e` for any UI changes, `make smoke-macos` for native integration, `make audit`, and complete CI on the branch. Fetch and integrate current main before merging; verify main CI after merge.

## External references

- [Gmail history](https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.history/list) and [Gmail synchronization](https://developers.google.com/workspace/gmail/api/guides/sync)
- [Gmail message listing](https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.messages/list): `q` is unavailable under `gmail.metadata`
- [Calendar event listing](https://developers.google.com/workspace/calendar/api/v3/reference/events/list) and [incremental synchronization](https://developers.google.com/workspace/calendar/api/guides/sync)
- [Google OAuth for desktop apps](https://developers.google.com/identity/protocols/oauth2/native-app)
