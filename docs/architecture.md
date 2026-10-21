# Architecture boundaries

The planned data flow is collectors -> append-only event log -> alignment engine -> findings/incidents -> desktop views and notifications. The Rust core owns data and decisions. The renderer reads views and submits validated user decisions through narrow Tauri commands.

`picket-core` is portable and has no Tauri dependency. `picket-app` owns macOS lifecycle and the command boundary. `picket-cli` invokes the same core APIs without starting a webview. `ui` validates incoming diagnostic data before rendering it. `picket-openclaw` is a separate private TypeScript package with no invented hook contracts or transport behavior yet.

The current native command is `get_runtime_info`; its capability is explicitly granted only to the main local window. A generated application command manifest makes that allowlist enforceable. No shell, filesystem, HTTP, or external URL plugin is exposed to JavaScript. Production CSP forbids remote scripts and resources. Development-only CSP allows the local Vite connection and development style injection.

The desktop app has one window and a menu-bar menu. Closing hides the window; reopening restores it. A second launch focuses the existing instance. Monitoring is inactive. No Google, Keychain, Messages, sockets, or model runtime is started by the foundation.

Next boundaries, in design milestone order:

1. M2: typed events, canonical JSON, ordered writer, SQLite migrations, verification and retention.
2. M3/M4: deterministic engine replay, findings, seeded scenarios, regression metrics.
3. M5/M6: allowlisted account collectors, Keychain authentication, Messages reader, verified OpenClaw adapter.
4. M7: verified model download and a bounded in-process worker.
5. M8/M9: complete user flows, permissions/notifications, packaging, and public release.
