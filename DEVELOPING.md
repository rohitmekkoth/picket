# Developing Picket

Technical notes for building, testing and releasing Picket. For installing and using it, see the [README](README.md).

## Architecture in brief

- `crates/core/`: portable domain logic (event store with hash chain, Gmail and Calendar collectors, Messages reader, request extraction, the alignment engine, rules, alerts, views). No desktop dependencies.
- `crates/app/`: the Tauri v2 shell, menu bar, notifications, Google sign-in (PKCE loopback, Keychain), and the narrow command boundary to the UI. Every command validates its input in Rust.
- `crates/model/`: the on-device model runtime (`llama-cpp-2` with Metal, grammar-constrained output). Model files are pinned to a Hugging Face revision and SHA-256.
- `crates/cli/`: diagnostics (`verify`, `replay`, `review`, `eval`, and `extraction-eval` with `--features model`).
- `ui/`: React 19, strict TypeScript, Vite, plain CSS, TanStack Query.
- `e2e/`: Playwright screen tests against a synthetic core.

Read the [design](docs/design.md), [architecture](docs/architecture.md), [decisions](docs/decisions.md) and [development notes](docs/setup/development.md) before changing behavior. Session handoff state is in [HANDOFF.md](HANDOFF.md).

## Set up

On an Apple silicon Mac with Xcode Command Line Tools:

```sh
brew install rustup pnpm cmake gh cargo-deny gitleaks actionlint
rustup toolchain install 1.99.1 --profile minimal --component rustfmt --component clippy
pnpm install --frozen-lockfile
pnpm exec playwright install chromium webkit
make doctor
make dev
```

Node is pinned in `.node-version`, pnpm in `package.json`, Rust in `rust-toolchain.toml`. `pnpm dev:ui` runs a browser preview that is deliberately not connected to the native core.

## Commands

| Command | Purpose |
| --- | --- |
| `make check` | Formatting, linting, type checks, UI tests and builds |
| `make test` | Rust workspace and UI tests |
| `make eval` | The synthetic engine evaluation and its CI gate |
| `make e2e` | Screen tests in both themes at the default and minimum window sizes; screenshots in `test-results/` |
| `make audit` | Dependency policy (`cargo deny`) and production npm audit |
| `make build` | The release `.app` in `target/release/bundle/macos/` |
| `make smoke-macos` | Build and exercise the packaged app (needs Accessibility for the terminal) |
| `cargo run -p picket-cli -- review --database <db> --since <time>` | Text-free timeline of captured, extracted and observed inputs with their assessment |

`make build` signs with the release certificate when it is in your login keychain, so Full Disk Access survives rebuilds; otherwise it signs ad hoc and macOS asks for access again after every build. A build includes Picket's Google OAuth client only when `local/google-client.env` (git-ignored) or the `PICKET_GOOGLE_CLIENT_ID` and `PICKET_GOOGLE_CLIENT_SECRET` environment variables provide it; without it, setup offers only "use your own Google Cloud client".

## Evaluation

The engine runs on 288 fixed synthetic cases on every change (`make eval`, report in `target/eval/mock-report.json`). They are correlated, constructed cases that catch regressions; they do not measure real-world accuracy.

| Mode | Cases | Agent identified | Linked | Conflicts found | False or benign alerts | Gaps reported |
| --- | --- | --- | --- | --- | --- | --- |
| Effects only | 112 | - | - | - | 0/112 | - |
| Agent instrumented | 96 | 96/96 | 96/96 | 64/64 | 0/32 | - |
| Policy | 32 | - | - | 16/16 high | 0/16 | - |
| Calendar | 16 | - | - | - | 0/16 | - |
| Coverage | 32 | - | - | - | - | 16/16 |

Request extraction with the on-device model (Qwen3.5 4B Q4_K_M, Apple M3 Max), run locally with `cargo run --release -p picket-cli --features model -- extraction-eval --model qwen3.5-4b-q4_k_m --model-file <gguf> --suite eval/extraction/<suite>.json`: 0 safety errors on the 49-case tuning suite (8 missed fields) and the 32-case follow-up suite (3 missed), about 4 seconds per message. Both suites informed tuning, so they are not independent measurements. See [eval/README.md](eval/README.md).

## Releasing

1. Bump the version in `Cargo.toml` (`[workspace.package]` and the two internal dependency pins), `crates/app/tauri.conf.json` and `package.json`; update `docs/release-notes.md`.
2. Merge to `main` with CI green.
3. Push a matching tag: `git tag -a vX.Y.Z -m "Picket X.Y.Z" && git push origin vX.Y.Z`.

`.github/workflows/release.yml` then builds the arm64 app and DMG, fails unless they are signed by the release certificate (fingerprint `9879149406518883578caa01b9912eacc606dc7c`), publishes them with `SHA256SUMS` and build provenance, and updates `Casks/picket.rb` in [rohitmekkoth/homebrew-tap](https://github.com/rohitmekkoth/homebrew-tap) from `packaging/homebrew/picket.rb`.

Repository secrets: `PICKET_SIGNING_P12` (base64), `PICKET_SIGNING_P12_PASSWORD`, `PICKET_GOOGLE_CLIENT_ID`, `PICKET_GOOGLE_CLIENT_SECRET`, `HOMEBREW_TAP_DEPLOY_KEY`. The signing material lives only in the git-ignored `local/signing/` and in those secrets; a new certificate changes the app's identity and resets every user's Full Disk Access.

## Rules for contributions

- Never commit credentials, messages, mailbox data, databases or model files. Use synthetic test data, and never log email addresses, subjects, message text or credentials.
- Keep `crates/core` free of Tauri; gate macOS code with `cfg(target_os = "macos")`.
- No new network hosts, Google scopes, permissions or telemetry beyond the design.
- No `unwrap` or `expect` in production Rust; clippy warnings stay at zero.
- Work on a branch, keep commits small, and merge with all CI checks green.
