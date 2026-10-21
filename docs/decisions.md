# Decisions and unresolved design details

## Foundation, 2026-10-20

- Package names are `picket-core`, `picket-app`, and `picket-cli`; the CLI binary is `picket`. Prefixes avoid ambiguity with Rust's built-in core library.
- Keep the original supplied design unchanged under ignored local storage. The repository copy is docs/design.md, with personal author metadata omitted and punctuation normalized. Product and package names use Picket.
- The setup branch establishes an environment and a runnable shell, not the complete M1 acceptance criteria or later product features. Native notification action feasibility remains an M1 investigation.
- Pin the installed Rust toolchain, Node, pnpm, direct frontend packages, lockfiles, and CI actions. Use a single pnpm workspace and a single Cargo workspace.
- Do not create the OpenClaw package until real upstream hook names and payloads are verified in M6.
- Keep the public project license undecided until the Messages decoder dependency is reviewed. Do not add a possibly GPL decoder while implying an MIT or Apache-only distribution.
- Delay model download and llama.cpp integration until the mock pipeline is evaluated, as required by the design's milestone order. CMake and Clang are available for that build.
- Do not add empty subsystem source modules, fabricated evaluation metrics, or disabled CI gates. Document the directory's future boundary instead.
- Use stderr tracing for the current data-free shell. Before handling account data, add tested redaction and size-based rotation; tracing alone is not a privacy filter.
- Generate the simple temporary app icon from `assets/icon.svg` with `pnpm tauri icon ../../assets/icon.svg --output icons`; do not manually edit generated icons or Tauri schemas.

## Resolve before implementing the owning milestone

1. **M2 event deduplication:** `UNIQUE(source, source_ref, kind)` cannot represent repeated changes to the same Calendar event if source_ref is only its event ID. Define a stable change identity including a source revision.
2. **M2 replay completeness:** ordinary received-mail contacts are upserted without events, while the design says contacts can be rebuilt entirely from the log. Decide how privacy-minimized contact observations and baseline state are replayed.
3. **M2 finding uniqueness:** SQLite UNIQUE constraints permit multiple NULL values. The proposed nullable finding key needs an explicit normalized identity or partial indexes.
4. **M2 retention and feedback:** define the retained chain anchor and transactional checkpoint boundary precisely. Preserve user decisions by stable event/finding identities when derived incidents are rebuilt.
5. **M5 calendar synchronization:** verify that the chosen initial bounded query and subsequent sync-token requests meet Google's parameter restrictions without introducing gaps.
6. **M6 decoder and plugin:** verify the current imessage-database license and the actual OpenClaw hooks before choosing dependencies.
7. **M7 model integrity:** verify model name, license, exact GGUF revision, checksum, and download redirect hosts. The design's model claims are not yet independently validated.
8. **Before public release:** choose the license, Developer ID signing identity, Homebrew tap owner, and validated installation instructions. Local development uses an ad hoc signature; public signing and notarization are not configured.
