# Picket privacy policy

Effective November 5, 2026.

Picket is a macOS app that keeps an independent record of what your AI agents do in your Gmail and Google Calendar, and tells you when it doesn't match what you asked. Picket runs entirely on your Mac. There is no Picket server, no Picket account, and no telemetry. The developer never receives your data.

## What Picket reads

With your permission, Picket reads:

- **Gmail metadata** (Google scope `gmail.metadata`): the From, To, Cc, Bcc, Subject, Date, Message-ID and In-Reply-To headers of your messages, and their labels. Picket cannot read message bodies or attachments, and it cannot send, change or delete email.
- **Google Calendar events** (Google scope `calendar.events.readonly`) on your primary calendar: event titles, times, organizers and attendees. Picket cannot create, change or delete events.
- **Your iMessage conversation with an agent you register**, read-only from the Messages database on your Mac, when you grant Picket Full Disk Access. Picket reads only the conversation with that agent.

## What Picket keeps, and where

Everything Picket keeps stays in your user account on your Mac:

- A local database with message ids, times, senders, recipients, the first 80 characters of each subject, labels, calendar event details as listed above, and for your agent conversation, what was requested or reported plus a snippet of at most 120 characters per message. Full message text is held only in memory while the on-device model reads it.
- Your Google sign-in token, in the macOS Keychain.
- Your settings, and an on-device language model that runs only on your Mac.

Picket does not encrypt the database itself; it is protected by your macOS account, and by FileVault if you turn it on.

## What leaves your Mac

Picket connects only to:

- Google (`accounts.google.com`, `oauth2.googleapis.com`, `gmail.googleapis.com`, `www.googleapis.com`) to sign you in and read the data above.
- Hugging Face (`huggingface.co` and its download host) once, to download the on-device model. No personal data is sent.

Picket does not sell, share, transfer or disclose your data to anyone, does not use it for advertising, and does not use it to train AI models. Files leave Picket only when you export a report or your data to a place you choose.

## Google user data

Picket's use and transfer to any other app of information received from Google APIs will adhere to the [Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy), including the Limited Use requirements. Google data is used only to show you what your agents did and to alert you when it does not match your request.

## How long Picket keeps it

Picket keeps history for 180 days by default. You can choose 30, 90, 180 or 365 days in Settings; older history is deleted for good.

## Your choices

- **Delete everything:** Settings > Uninstall and wipe deletes Picket's database, model, settings, caches, Keychain item and login item from your Mac.
- **Revoke Google access:** remove Picket at [myaccount.google.com/connections](https://myaccount.google.com/connections). Picket then stops reading your Google data.
- **Stop reading Messages:** remove Picket from Full Disk Access in System Settings > Privacy & Security.
- **Export:** save one incident or all of Picket's data from the app.

## Children

Picket is not directed to children under 13 and does not knowingly collect their data.

## Changes

Changes to this policy are published in this file, with its history visible in the repository.

## Contact

Questions about this policy: open an issue at [github.com/rohitmekkoth/picket/issues](https://github.com/rohitmekkoth/picket/issues).
