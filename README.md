# Picket

Picket keeps an independent record of what your AI agent does. It compares what you asked the agent in iMessage, what the agent said it did, and what actually happened in your Gmail and Google Calendar, and notifies you when they don't match. Everything runs and stays on your Mac: no Picket server, account or telemetry, and the language model that reads your requests runs locally.

v1 watches one agent, Instinct, over iMessage.

| Overview | Incident |
| --- | --- |
| ![Overview: what needs a look today](docs/images/overview.png) | ![Incident: request, report and what happened side by side](docs/images/incident.png) |

Screens show synthetic test data.

## Install

Requires an Apple silicon Mac with macOS 13 or later.

```sh
brew install --cask rohitmekkoth/tap/picket
```

Or download the DMG from [Releases](https://github.com/rohitmekkoth/picket/releases) and drag Picket to Applications.

Picket is signed with its own certificate, not an Apple Developer ID, so macOS blocks the first launch. Open Picket once, then choose **Open Anyway** in System Settings > Privacy & Security. To check a download: `shasum -a 256 -c SHA256SUMS`, or `gh attestation verify <file> --repo rohitmekkoth/picket`.

## Setup (about 5 minutes)

The first launch walks you through:

1. **Google:** sign in with Google. Google hasn't reviewed Picket yet, so it shows a warning; choose **Advanced**, then **Go to Picket**. At most 100 people can use this sign-in.
2. **Your agent:** add Instinct's phone number or Apple ID email.
3. **Messages:** turn on Full Disk Access for Picket in System Settings > Privacy & Security, so it can read your chat with Instinct. Picket opens the Messages database read-only and keeps only that chat.
4. **Alerts:** open at login and the daily summary time.
5. **Model:** download the 2.7 GB on-device model.

## What Picket can and can't see

| Permission | Why | Limits |
| --- | --- | --- |
| Gmail metadata (`gmail.metadata`) | Who emails were to and from, when, and their subject | Never message bodies or attachments; cannot send or change anything |
| Calendar events, read only (`calendar.events.readonly`) | Changes to your primary calendar | Cannot create, change or delete events |
| Full Disk Access | Read your iMessage chat with your agent | Read only; other chats are ignored; message text is never stored |

Picket records what it observes in a tamper-evident log in `~/Library/Application Support/dev.picket.desktop/`. Settings > **Uninstall and wipe** removes the log, the model, the Google sign-in and the login item. See [PRIVACY.md](PRIVACY.md).

**Known limits.** Picket says the agent did something only when the agent reports it and the report matches; otherwise it says it can't tell who did it. Request reading handles plain wording; unusual phrasing can be missed, and Picket then says it couldn't tell rather than raising an alarm. Only Gmail and the primary Google Calendar are watched.

## Evaluation

The engine is checked on 288 fixed synthetic cases on every change (`make eval`). These are correlated, constructed cases that catch regressions; they are not a measure of real-world accuracy.

| Mode | Cases | Agent identified | Linked | Conflicts found | False or benign alerts | Gaps reported |
| --- | --- | --- | --- | --- | --- | --- |
| Effects only | 112 | - | - | - | 0/112 | - |
| Agent instrumented | 96 | 96/96 | 96/96 | 64/64 | 0/32 | - |
| Policy | 32 | - | - | 16/16 high | 0/16 | - |
| Calendar | 16 | - | - | - | 0/16 | - |
| Coverage | 32 | - | - | - | - | 16/16 |

Request reading with the on-device model (Qwen3.5 4B, Apple M3 Max): 0 safety errors on 49 tuning cases (8 missed fields) and 32 follow-up cases (3 missed), about 4 seconds per message. These sets were used for tuning, so they are not independent measurements.

## Develop

On an Apple silicon Mac with Xcode Command Line Tools:

```sh
brew install rustup pnpm cmake gh cargo-deny gitleaks actionlint
rustup toolchain install 1.99.1 --profile minimal --component rustfmt --component clippy
pnpm install --frozen-lockfile
make dev
```

| Command | Purpose |
| --- | --- |
| `make check` | Formatting, linting, type checks, UI tests and builds |
| `make test` | Rust workspace and UI tests |
| `make eval` | The synthetic engine evaluation |
| `make e2e` | Browser screen tests in both themes at the default and minimum window sizes |
| `make audit` | Dependency policy and production npm audit |
| `make build` | The release `.app` in `target/release/bundle/macos/` |

Start with the [development handoff](HANDOFF.md), then the [design](docs/design.md), [architecture](docs/architecture.md), [development notes](docs/setup/development.md) and [decisions](docs/decisions.md). Never commit credentials, messages, mailbox data, databases or model files.

Report security issues as described in [SECURITY.md](SECURITY.md).
