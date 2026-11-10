Picket keeps an independent, local record of what your AI agent does in Gmail and Google Calendar, compares it with what you asked the agent in iMessage, and tells you when something does not match.

## Install

- Homebrew: `brew install --cask rohitmekkoth/tap/picket`
- Or download the DMG below and drag Picket to Applications.

Picket is signed with its own certificate, not an Apple Developer ID. macOS blocks the first launch: open Picket once, then choose **Open Anyway** in System Settings > Privacy & Security. Requires an Apple silicon Mac with macOS 13 or later.

Verify a download with `shasum -a 256 -c SHA256SUMS`, or its build provenance with `gh attestation verify <file> --repo rohitmekkoth/picket`.

## What v1 does

- Watches one agent, Instinct, over iMessage, plus Gmail (who, when and subject; never message bodies) and your primary Google Calendar, read only.
- Reads your requests with a language model that runs only on your Mac, and compares them with Instinct's reports and what happened in your account.
- Notifies you when the agent does something you told it not to, contacts someone you did not mention, or crosses a boundary you set; sends a daily summary.
- Stores everything on your Mac in a tamper-evident log. No Picket server, account or telemetry. Settings > Uninstall and wipe removes it all.

## Known limits

- Picket can only say the agent acted when the agent reports it and the report matches; otherwise it says it can't tell who did it.
- Request reading handles plain wording; unusual phrasing can be missed, and Picket then says it couldn't tell rather than raising an alarm.
- Google has not reviewed Picket's sign-in, so Google shows a warning (choose Advanced, then Go to Picket), and at most 100 people can sign in.
- Only Gmail and the primary Google Calendar are watched.
