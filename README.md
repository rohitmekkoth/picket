# Picket

Picket is a macOS app for an independent record of what personal AI agents do. It will compare what you requested, what an agent claimed, and what actually changed in Gmail and Google Calendar. The planned product is local, read-only, and requires no paid model API.

**Current status: development foundation only.** The Tauri shell, menu-bar entry, React interface, portable Rust core boundary, CLI diagnostics, and build/test tooling are available. Collection, Google authentication, Messages access, the engine, model runtime, and notifications are not implemented. This build does not monitor or protect an account.

## Start developing

On an Apple Silicon Mac with Xcode Command Line Tools:

```sh
brew install rustup pnpm cmake gh cargo-deny gitleaks actionlint
export PATH="$(brew --prefix rustup)/bin:$PATH"
rustup toolchain install 1.99.1 --profile minimal --component rustfmt --component clippy --component rust-src --component rust-analyzer
pnpm install --frozen-lockfile
pnpm check:env
pnpm tauri dev
```

Node is pinned in `.node-version`, pnpm in `package.json`, and Rust in `rust-toolchain.toml`. Install the pinned Node version through your preferred version manager if needed. `gh` is only needed for repository and CI administration. Opening this checkout does not require connecting Google or granting Full Disk Access.

Closing the window keeps Picket in the menu bar. Click the menu-bar icon to reopen it, or use its context menu to open or quit. `pnpm dev:ui` runs a browser preview; it explicitly reports that it is not connected to the native core. The development server binds only to `127.0.0.1:1420`; packaged builds have no HTTP server.

## Daily commands

| Command | Purpose |
| --- | --- |
| `make doctor` | Verify installed development tools |
| `make dev` | Desktop app and UI hot reload |
| `make fmt` | Rust and frontend formatting |
| `make check` | Formatting, linting, type checks, UI tests and builds |
| `make test` | Rust workspace and UI unit/integration tests |
| `pnpm exec playwright install chromium webkit` | Install the smoke-test browser once |
| `make e2e` | Browser smoke tests at desktop and compact sizes |
| `make audit` | Rust dependency policy and production npm audit |
| `make build` | Build the release-mode `.app` |
| `make smoke-macos` | Build and exercise the packaged macOS app |
| `cargo run -p picket-cli -- doctor` | Print local runtime information as JSON |

The app bundle is written to `target/release/bundle/macos/Picket.app`. It is ad hoc signed for local development, without a Developer ID signature or notarization. No installer, Homebrew cask, evaluation results, or release pipeline is claimed yet.

## Layout

```text
crates/core/       Portable domain logic; no desktop dependencies
crates/app/        Tauri shell, tray, command boundary, app configuration
crates/cli/        Diagnostic CLI; future verify/eval/export commands
ui/               React 19, strict TypeScript, Vite, plain CSS, TanStack Query
e2e/              Browser smoke tests
docs/             Setup, architecture, and decision records
scripts/          Toolchain diagnostics and Tauri launcher
.github/          CI and dependency updates
```

Read [development notes](docs/setup/development.md), [architecture](docs/architecture.md), and [decisions](docs/decisions.md). Read the [product design](docs/design.md) for the planned milestones. Runtime data and private local notes are excluded from Git.

## Privacy and contributions

Do not commit credentials, personal messages, mailbox exports, databases, or model weights. `.gitignore` excludes common runtime artifacts. The current shell logs startup diagnostics to stderr only; future data collection must implement the design's redaction and rotation policy before handling account data.

The repository is private while the foundation is built. The public license is pending the dependency review documented in [decisions](docs/decisions.md). See [SECURITY.md](SECURITY.md) for reporting guidance.
