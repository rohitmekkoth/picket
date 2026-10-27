# Architecture boundaries

The planned data flow is collectors -> append-only event log -> alignment engine -> findings/incidents -> desktop views and notifications. The Rust core owns data and decisions. The renderer reads views and submits validated user decisions through narrow Tauri commands.

`picket-core` is portable and has no Tauri dependency. `picket-app` owns macOS lifecycle and the command boundary. `picket-cli` invokes the same core APIs without starting a webview. `ui` validates incoming diagnostic data before rendering it. The OpenClaw adapter is deferred until its real upstream contract is verified in M6.

The current native command is `get_runtime_info`; its capability is explicitly granted only to the main local window. A generated application command manifest makes that allowlist enforceable. No shell, filesystem, HTTP, or external URL plugin is exposed to JavaScript. Production CSP forbids remote scripts and resources. Development-only CSP allows the local Vite connection and development style injection.

The desktop app has one window and a menu-bar menu. Closing hides the window; reopening restores it. A second launch focuses the existing instance. After the user connects Google, one native worker owns Google polling and the ordered SQLite writer. Messages, sockets and the model runtime are not started.

The store is the single writer. Each `Store::commit` appends a collector batch, logs coverage only when it changes, advances the cursor, and persists the findings that batch can affect, in one transaction. The in-memory `Assessor` is derived from the log and rebuilt after a failed write or a restart. `engine::findings(events, now)` remains the reference replay; tests require incremental results to equal it.

Current and next boundaries, in design milestone order:

1. M2 is implemented: typed events, canonical JSON, ordered writer, SQLite migrations, verification and retention.
2. M3 is implemented on mocks: deterministic replay with separate actor evidence, request constraints, coverage, findings and a durable notification outbox. M4 adds fixed synthetic per-mode safety and utility gates through `picket eval --mock`. See [the alignment plan](alignment-engine-plan.md). Assessment is incremental and indexed; retention covers derived findings.
3. M5 is implemented: allowlisted Google collectors and Keychain authentication, resuming from durable progress. M6: Messages reader and verified OpenClaw adapter. Before adding them, move the portable collector logic from `picket-app` into `picket-core` behind a shared collector interface.
4. M7: verified model download and a bounded in-process worker.
5. M8/M9: complete user flows, permissions/notifications, packaging, and public release.
