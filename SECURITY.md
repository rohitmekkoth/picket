# Security

Picket watches personal accounts, so security reports are welcome.

Report vulnerabilities privately through GitHub's **Report a vulnerability** button on the repository's Security tab. Do not open public issues for vulnerabilities, and never post tokens, personal messages, mailbox exports or database files.

What Picket protects and how is described in the design's threat model ([docs/design.md](docs/design.md), "Security and privacy"). In short: read-only Google scopes, one HTTP client limited to Google and the model host, no telemetry, the Messages database opened read-only, message text never stored, a hash-chained local log, and credentials kept in the macOS Keychain.

Releases are built by GitHub Actions from a tag, signed with Picket's own certificate (fingerprint `9879149406518883578caa01b9912eacc606dc7c`), and published with SHA-256 checksums and build provenance attestations. Verify a download with `gh attestation verify <file> --repo rohitmekkoth/picket`.

For contributors: minimize permissions, validate commands in Rust, keep secrets out of source control and logs, use synthetic fixtures, and run `make audit`.
