# Security

Picket is currently a private development foundation. Account collection and monitoring are not implemented, and this build should not be relied on for security detection.

Report vulnerabilities privately to the repository owner through an existing private contact channel. When public distribution is enabled, configure GitHub private vulnerability reporting before accepting reports through that feature. Do not post tokens, personal messages, mailbox exports, or database files in issues.

For contributors: minimize permissions, validate commands in Rust, keep secrets out of source control and logs, and use synthetic fixtures. Review the design's threat model before implementing collectors or storage. Run `make audit` for dependency policy checks.
