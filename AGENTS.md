# Project instructions

- Use a plain dash, never an em dash, in new prose.
- Do not add an agent as a commit co-author.
- Do not manually edit changelogs or generated files, including lockfiles and Tauri schemas. Use their generators.
- Prefer quality, simplicity, robustness, scalability, and maintainability when choosing designs.
- Reproduce bugs through the closest available end-user flow before fixing them.
- Inspect UI output carefully and fix clear visual defects, lint failures, failing tests, and flakiness encountered during work.
- Ask before using a harness feature that launches a large swarm of agents; explain its tradeoffs first.
- Read the root design document before product work. Record ambiguities in docs/decisions.md.
- Keep crates/core portable and free of Tauri. Gate macOS integrations with cfg(target_os = "macos").
- UI commands validate their input in Rust. Do not expose general filesystem, shell, or HTTP permissions to the webview.
- Do not add network hosts, Google scopes, telemetry, or real account fixtures beyond the design.
- Do not log email addresses, subjects, message text, or credentials. Use synthetic test data.
- No unwrap or expect in production Rust. Keep clippy warnings at zero.
- Run relevant checks from the Makefile; do not weaken tests, audit policy, or future evaluation baselines to obtain green CI.

## Git workflow

- Work in small, coherent increments. Verify each increment with appropriate checks, then commit and push it to GitHub. Do not accumulate a whole feature into one final push.
- Before starting a feature, fetch the current main branch and create a dedicated feature branch from it. Use descriptive names such as feature/event-store or fix/desktop-startup. Use setup/development-environment for the initial scaffold.
- Push verified increments to that branch. Keep unrelated work out of its commits, and stage explicit paths instead of indiscriminately adding the whole workspace.
- Before each push, inspect the exact staged content for secrets, personal data, machine-specific paths, and generated/runtime artifacts. Review commit author metadata as well. Ignore rules are not a substitute for inspecting the staged changes.
- Before merging, fetch main again, integrate any new changes, resolve conflicts, and run the checks for the complete branch against that updated base. Require all configured CI checks to pass.
- Merge into main only when the full branch works and is synchronized with main. Preserve the small commits in the merge history. Verify the resulting main CI checks.
- This workflow authorizes routine commits, branch pushes, and verified merges within the agreed task scope. It does not authorize new features beyond that scope, public releases, changing repository visibility, or force-pushing shared history.
